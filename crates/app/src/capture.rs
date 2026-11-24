use std::path::PathBuf;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{ClipboardItem, Context, Div, Image, ImageFormat, InteractiveElement, actions};
use url::Url;

use browser_core::capture::{ELEMENT_HINT, NO_PICKER, element_area};
use browser_core::toast::Tone;
use engine::WebView;
use engine::view_policy::SnapshotArea;
use shield::ShieldLevel;

use crate::browser::{Browser, TabKey, now};
use crate::capture_overlay::Purpose;

pub(crate) type Done = Box<dyn Fn(Option<Vec<u8>>)>;

actions!(tarn, [TakeScreenshot, Measure, CaptureArea, CaptureElement, CaptureFullPage, SavePdf, CopyMarkdownLink, CopyTitle, PictureInPicture]);

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &TakeScreenshot, _, cx| b.capture(true, cx)))
        .on_action(cx.listener(|b, _: &CaptureArea, w, cx| b.open_overlay(Purpose::Area, w, cx)))
        .on_action(cx.listener(|b, _: &Measure, w, cx| {
            if b.devtools_on() {
                b.open_overlay(Purpose::Measure, w, cx);
            }
        }))
        .on_action(cx.listener(|b, _: &CaptureElement, _, cx| b.capture_element(cx)))
        .on_action(cx.listener(|b, _: &CaptureFullPage, _, cx| b.capture_full_page(cx)))
        .on_action(cx.listener(|b, _: &SavePdf, _, cx| b.capture(false, cx)))
        .on_action(cx.listener(|b, _: &CopyMarkdownLink, _, cx| b.copy_page(true, cx)))
        .on_action(cx.listener(|b, _: &CopyTitle, _, cx| b.copy_page(false, cx)))
        .on_action(cx.listener(|b, _: &PictureInPicture, _, cx| b.picture_in_picture(cx)))
}

impl Browser {
    fn page_title(&self, key: TabKey) -> String {
        self.views.get(&key).map(|v| v.title()).filter(|t| !t.is_empty()).or_else(|| self.model.tab(key).map(|t| t.title.clone())).unwrap_or_default()
    }

    /// Screenshot of the visible part of the active tab, or the whole page as PDF, saved to
    /// Downloads and listed in the tab's own profile store (in memory for private spaces).
    pub fn capture(&mut self, screenshot: bool, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        self.capture_tab(key, screenshot, move |view, done| if screenshot { view.snapshot_png(None, done) } else { view.pdf(done) }, cx);
    }

    fn capture_full_page(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        self.capture_tab(key, true, |view, done| view.snapshot_rect(SnapshotArea::FullPage, done), cx);
    }

    fn capture_element(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let (Some(view), Some((_, _, width, height))) = (self.views.get(&key).cloned(), self.link_rect(key)) else { return };
        if self.level_for_tab(key, &self.tab_url(key).unwrap_or_default()) == ShieldLevel::Lockdown {
            self.notify_offer(key, NO_PICKER.into(), None);
            return cx.notify();
        }
        self.show_toast_as(Tone::Info, ELEMENT_HINT, None, cx);
        let zoom = view.zoom();
        let page = (f64::from(width) / zoom, f64::from(height) / zoom);
        let (weak, (tx, mut rx)) = (Rc::downgrade(&view), mpsc::unbounded());
        view.pick_rect(move |answer| drop(tx.unbounded_send(answer)));
        cx.spawn(async move |this, cx| {
            let Some(area) = rx.next().await.and_then(|answer| element_area(&answer, page)) else { return };
            this.update(cx, |b, cx| {
                if b.same_view(key, &weak) {
                    b.capture_tab(key, true, move |view, done| view.snapshot_rect(area, done), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves what `take` hands to its callback as `key`'s screenshot or PDF.
    pub(crate) fn capture_tab(&mut self, key: TabKey, screenshot: bool, take: impl FnOnce(&WebView, Done), cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key).cloned() else { return };
        let Some(profile_id) = self.profile_of(key) else { return };
        let page = (profile_id, self.tab_url(key).unwrap_or_default(), self.page_title(key));
        let (tx, mut rx) = mpsc::unbounded();
        take(&view, Box::new(move |bytes| drop(tx.unbounded_send(bytes))));
        let started = now();
        cx.spawn(async move |this, cx| {
            if let Some(bytes) = rx.next().await {
                this.update(cx, |b, cx| b.save_capture(key.private, page, screenshot, started, bytes, cx)).ok();
            }
        })
        .detach();
    }

    /// `page` is the tab's profile, URL and title when the capture started; the tab may be gone, or
    /// its id taken by a new tab, once the bytes arrive.
    fn save_capture(
        &mut self,
        private: bool,
        page: (store::Id, String, String),
        screenshot: bool,
        started: i64,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        let (profile_id, url, title) = page;
        let name = browser_core::text::capture_name(&title, Url::parse(&url).ok().as_ref(), if screenshot { "png" } else { "pdf" });
        let result = bytes
            .ok_or_else(|| if cfg!(windows) { "Chromium could not capture the page" } else { "WebKit could not capture the page" }.to_owned())
            .and_then(|bytes| {
                if screenshot && browser_core::capture::copies(private) {
                    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes.clone())));
                }
                write_download(&name, &bytes).map_err(|e| e.to_string())
            });
        if let Ok(path) = &result
            && self.core.keeps_record_since(private, profile_id, started)
        {
            self.store(private).add_download(profile_id, &url, &path.to_string_lossy(), "finished", now()).ok();
            self.downloads_popover.changed += 1;
        }
        match result {
            Ok(path) if screenshot => self.show_tab_toast(
                browser_core::toast::Tone::Success,
                browser_core::capture::saved_note(browser_core::capture::copies(private)),
                Some(crate::toast::Action::Reveal(path)),
                (private, profile_id),
                cx,
            ),
            result => {
                self.notify_download(private, profile_id, &result.map(|path| (path, true)), true, cx);
            }
        }
    }

    fn copy_page(&mut self, markdown: bool, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let title = self.page_title(key);
        let text = if markdown { browser_core::text::markdown_link(&title, &self.tab_url(key).unwrap_or_default()) } else { ui::text::bounded_title(&title) };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast_as(browser_core::toast::Tone::Success, if markdown { "Link copied" } else { "Title copied" }, None, cx);
    }

    fn picture_in_picture(&mut self, cx: &mut Context<Self>) {
        let (Some(key), Some(view)) = (self.model.active, self.active_view()) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(&view), mpsc::unbounded());
        view.picture_in_picture(engine::page_scripts::PipMode::Toggle, move |result| drop(tx.unbounded_send(result)));
        cx.spawn(async move |this, cx| {
            let notice = match rx.next().await.as_deref() {
                Some("none") => "This page has no video",
                Some("refused") => "This video cannot play in Picture in Picture",
                _ => return,
            };
            this.update(cx, |b, cx| {
                if b.same_view(key, &weak) {
                    b.notify_offer(key, notice.into(), None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

pub(crate) fn write_download(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::home_dir().ok_or(std::io::ErrorKind::NotFound)?.join("Downloads");
    browser_core::capture::save_new(&dir, name, bytes)
}
