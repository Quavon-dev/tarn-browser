use std::cell::{Cell, RefCell};
use std::rc::Rc;

use engine::page_scripts::FIND_COUNT_MAX;
use nsh_platform_ios::ComponentContext;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSObject, NSOrderedSet, NSString, NSStringCompareOptions};
use objc2_ui_kit::{UITextRange, UITextSearchAggregator, UITextSearchFoundTextStyle, UITextSearchMatchMethod, UITextSearchOptions};
use serde_json::json;

use crate::webkit::WebView;

/// The running find: its text, the matches in the order WebKit reports them, and the current one.
#[derive(Default)]
pub(crate) struct Find {
    text: RefCell<String>,
    ranges: RefCell<Vec<Retained<UITextRange>>>,
    at: Cell<usize>,
    search: RefCell<Option<Retained<Aggregator>>>,
    searches: Cell<u64>,
}

impl Find {
    /// Marks every match of `text` and moves to the next (or previous) one through WebKit's
    /// `UITextSearching`, which draws the marks outside the page's reach; `found` reports how many
    /// there are and which one is current.
    pub fn find(self: &Rc<Self>, web: &WebView, context: &ComponentContext, text: &str, backwards: bool) {
        if text.is_empty() {
            self.clear(web);
            return;
        }
        if *self.text.borrow() == text {
            if self.search.borrow().is_none() {
                let at = browser_core::find::step(self.at.get(), self.ranges.borrow().len(), backwards);
                self.show(web, context, at);
            }
            return;
        }
        self.clear(web);
        *self.text.borrow_mut() = text.to_owned();
        let (find, view, context, this) = (Rc::downgrade(self), Weak::from_retained(&web.retain()), context.clone(), self.searches.get());
        let aggregator = Aggregator::new(
            web.mtm(),
            Box::new(move |ranges| {
                let (Some(find), Some(view)) = (find.upgrade(), view.load()) else { return };
                if find.searches.get() != this {
                    return;
                }
                find.search.take();
                for range in &ranges {
                    decorate(&view, range, UITextSearchFoundTextStyle::Found);
                }
                let at = if backwards { ranges.len().saturating_sub(1) } else { 0 };
                *find.ranges.borrow_mut() = ranges;
                find.at.set(at);
                find.show(&view, &context, at);
            }),
        );
        *self.search.borrow_mut() = Some(aggregator.clone());
        let options = Options::new(web.mtm());
        // SAFETY: main thread; WKWebView adopts `UITextSearching` (iOS 16), and the aggregator lives
        // in `search` until it finishes.
        let _: () = unsafe {
            msg_send![web, performTextSearchWithQueryString: &*NSString::from_str(text), usingOptions: &**options, resultAggregator: ProtocolObject::<dyn UITextSearchAggregator>::from_ref(&*aggregator)]
        };
    }

    /// Removes every mark and ends a running search; answers whether a find was running.
    pub fn clear(&self, web: &WebView) -> bool {
        self.searches.set(self.searches.get() + 1);
        let running = !std::mem::take(&mut *self.text.borrow_mut()).is_empty();
        self.ranges.borrow_mut().clear();
        self.search.take();
        // SAFETY: main thread; WKWebView adopts `UITextSearching` (iOS 16).
        let _: () = unsafe { msg_send![web, clearAllDecoratedFoundText] };
        running
    }

    fn show(&self, web: &WebView, context: &ComponentContext, at: usize) {
        let ranges = self.ranges.borrow();
        let Some(range) = ranges.get(at) else { return context.emit("found", json!({ "matched": 0 })) };
        if let Some(old) = ranges.get(self.at.get()) {
            decorate(web, old, UITextSearchFoundTextStyle::Found);
        }
        decorate(web, range, UITextSearchFoundTextStyle::Highlighted);
        if web.respondsToSelector(sel!(scrollRangeToVisible:inDocument:)) {
            // SAFETY: main thread; the selector exists (checked above) and the range is this view's.
            let _: () = unsafe { msg_send![web, scrollRangeToVisible: &**range, inDocument: None::<&AnyObject>] };
        }
        self.at.set(at);
        context.emit("found", json!({ "matched": ranges.len(), "current": at + 1 }));
    }
}

fn decorate(web: &WebView, range: &UITextRange, style: UITextSearchFoundTextStyle) {
    // SAFETY: main thread; the range came from this view's own search.
    let _: () = unsafe { msg_send![web, decorateFoundTextRange: range, inDocument: None::<&AnyObject>, usingStyle: style] };
}

type Finished = Box<dyn FnOnce(Vec<Retained<UITextRange>>)>;

pub struct Collected {
    ranges: RefCell<Vec<Retained<UITextRange>>>,
    finished: RefCell<Option<Finished>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnFindAggregator"]
    #[ivars = Collected]
    pub struct Aggregator;

    unsafe impl NSObjectProtocol for Aggregator {}

    unsafe impl UITextSearchAggregator for Aggregator {
        #[unsafe(method_id(allFoundRanges))]
        fn all_found_ranges(&self) -> Retained<NSOrderedSet<UITextRange>> {
            NSOrderedSet::orderedSetWithArray(&NSArray::from_retained_slice(&self.ivars().ranges.borrow()))
        }

        #[unsafe(method(foundRange:forSearchString:inDocument:))]
        fn found_range(&self, range: &UITextRange, _string: &NSString, _document: Option<&AnyObject>) {
            let mut ranges = self.ivars().ranges.borrow_mut();
            if ranges.len() < FIND_COUNT_MAX {
                ranges.push(range.retain());
            }
        }

        #[unsafe(method(invalidateFoundRange:inDocument:))]
        fn invalidate_found_range(&self, range: &UITextRange, _document: Option<&AnyObject>) {
            self.ivars().ranges.borrow_mut().retain(|r| **r != *range);
        }

        #[unsafe(method(invalidate))]
        fn invalidate(&self) {
            self.ivars().ranges.borrow_mut().clear();
        }

        #[unsafe(method(finishedSearching))]
        fn finished_searching(&self) {
            if let Some(finished) = self.ivars().finished.take() {
                finished(self.ivars().ranges.take());
            }
        }
    }
);

impl Aggregator {
    fn new(mtm: MainThreadMarker, finished: Finished) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Collected { ranges: RefCell::default(), finished: RefCell::new(Some(finished)) });
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    /// Search options without case and within words; UIKit's own are read-only and match case.
    #[unsafe(super(UITextSearchOptions, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnFindOptions"]
    struct Options;

    impl Options {
        #[unsafe(method(stringCompareOptions))]
        fn string_compare_options(&self) -> NSStringCompareOptions {
            NSStringCompareOptions::CaseInsensitiveSearch
        }

        #[unsafe(method(wordMatchMethod))]
        fn word_match_method(&self) -> UITextSearchMatchMethod {
            UITextSearchMatchMethod::Contains
        }
    }
);

impl Options {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: UITextSearchOptions' designated initialiser on a freshly allocated instance.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}
