use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSColor, NSControlStateValueOn, NSEvent, NSEventMask, NSEventModifierFlags, NSFloatingWindowLevel, NSImage, NSImageSymbolConfiguration,
    NSMenu, NSMenuItem,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSPoint, NSSize, NSString, NSUserDefaults};

use crate::Host;
pub use crate::menu_item::Item;

type Chosen = RefCell<Option<Box<dyn FnOnce(isize)>>>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnMenuTarget"]
    #[ivars = Chosen]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(choose:))]
        fn choose(&self, item: &NSMenuItem) {
            if let Some(chosen) = self.ivars().borrow_mut().take() {
                chosen(item.tag());
            }
        }
    }
);

/// Pops up `items` as a native menu in `host` at `at` (points from the top left, such as a
/// button's lower left corner) or else at the mouse location, and calls `chosen` with the value
/// of the entry the user picks. AppKit runs a modal tracking loop inside this call, so the caller
/// must not hold UI state borrowed across it.
pub fn show<T: 'static>(host: &Host, items: Vec<Item<T>>, at: Option<(f64, f64)>, chosen: impl FnOnce(T) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let mut values = Vec::new();
    let target = Target::alloc(mtm).set_ivars(RefCell::new(None));
    // SAFETY: NSObject's designated initializer on a freshly allocated instance.
    let target: Retained<Target> = unsafe { msg_send![super(target), init] };
    let menu = build(mtm, &target, items, &mut values);
    *target.ivars().borrow_mut() = Some(Box::new(move |tag| {
        if let Some(value) = usize::try_from(tag).ok().and_then(|ix| values.into_iter().nth(ix)) {
            chosen(value);
        }
    }));
    let Some(window) = host.view.window() else { return };
    // GPUI puts its PopUp windows on the pop-up menu level, where they would cover this menu.
    let lowered: Vec<_> = NSApplication::sharedApplication(mtm)
        .windows()
        .into_iter()
        .filter(|w| w.level() > NSFloatingWindowLevel)
        .map(|w| {
            let level = w.level();
            w.setLevel(NSFloatingWindowLevel);
            (w, level)
        })
        .collect();
    let location = match at {
        Some((x, y)) => NSPoint::new(x, if host.view.isFlipped() { y } else { host.view.bounds().size.height - y }),
        None => host.view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None),
    };
    menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(&host.view));
    for (w, level) in lowered {
        w.setLevel(level);
    }
}

/// An image beside a menu entry: a PNG such as a site icon, a PNG glyph drawn in the menu's text
/// colour, an SF Symbol, or a dot in an RGB colour.
pub enum Image {
    Png(Vec<u8>),
    Template(Vec<u8>),
    Symbol(&'static str),
    Dot(u32),
}

/// What GPUI's menu items cannot carry: an image and a check mark.
pub struct Mark {
    pub title: String,
    pub image: Option<Image>,
    pub checked: bool,
}

/// Decorates entries of the app menu named `menu`. Marks are matched in order, depth first, to
/// the entries with their titles, so they must be listed in the order the menu shows them.
pub fn decorate(menu: &str, marks: &[Mark]) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(top) = NSApplication::sharedApplication(mtm).mainMenu().and_then(|m| m.itemWithTitle(&NSString::from_str(menu))) else {
        return;
    };
    if let Some(submenu) = top.submenu() {
        mark_items(&submenu, &mut marks.iter().peekable());
    }
}

/// Lists the app's open windows in the Window menu again; replacing the menu bar drops them.
pub fn list_windows() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().into_iter().filter(|w| w.isVisible() && w.canBecomeMainWindow() && !w.isExcludedFromWindowsMenu()) {
        app.addWindowsItem_title_filename(&window, &window.title(), false);
    }
}

fn mark_items(menu: &NSMenu, marks: &mut std::iter::Peekable<std::slice::Iter<Mark>>) {
    for item in menu.itemArray() {
        if let Some(mark) = marks.next_if(|m| item.title().to_string() == m.title) {
            if mark.checked {
                item.setState(NSControlStateValueOn);
            }
            set_image(&item, mark.image.as_ref());
        }
        if let Some(submenu) = item.submenu() {
            mark_items(&submenu, marks);
        }
    }
}

fn set_image(item: &NSMenuItem, image: Option<&Image>) {
    item.setImage(image.and_then(self::image).as_deref());
    if item.respondsToSelector(sel!(setPreferredImageVisibility:)) {
        // SAFETY: macOS 27's `preferredImageVisibility` takes an NSInteger; 1 is
        // `NSMenuItemImageVisibilityVisible`, without which AppKit hides most item images.
        unsafe {
            let _: () = msg_send![item, setPreferredImageVisibility: 1isize];
        }
    }
}

fn image(image: &Image) -> Option<Retained<NSImage>> {
    let symbol = |name| NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None);
    let (image, side) = match image {
        Image::Png(bytes) => (NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?, 16.),
        Image::Template(bytes) => {
            let glyph = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
            glyph.setTemplate(true);
            (glyph, 16.)
        }
        Image::Symbol(name) => (symbol(name)?, 16.),
        Image::Dot(rgb) => {
            let channel = |shift: u32| f64::from((rgb >> shift) & 0xff) / 255.;
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.);
            let config = NSImageSymbolConfiguration::configurationWithPaletteColors(&NSArray::from_retained_slice(&[color]));
            (symbol("circle.fill")?.imageWithSymbolConfiguration(&config)?, 10.)
        }
    };
    image.setSize(NSSize::new(side, side));
    Some(image)
}

/// Keeps AppKit from saving window state and snapshots in `~/Library/Saved Application State`,
/// which could hold a private window; Tarn restores its windows itself. Call before the app runs.
pub fn disable_saved_application_state() {
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.setBool_forKey(true, &NSString::from_str("ApplePersistenceIgnoreState"));
    defaults.setBool_forKey(false, &NSString::from_str("NSQuitAlwaysKeepsWindows"));
}

/// Whether "Natural scrolling" is on in System Settings, which is the default.
pub fn natural_scrolling() -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str("com.apple.swipescrolldirection");
    defaults.objectForKey(&key).is_none() || defaults.boolForKey(&key)
}

fn build<T>(mtm: MainThreadMarker, target: &Retained<Target>, items: Vec<Item<T>>, values: &mut Vec<T>) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for item in items {
        let entry = match item {
            Item::Separator => NSMenuItem::separatorItem(mtm),
            Item::Submenu(title, children) => {
                let entry = NSMenuItem::new(mtm);
                entry.setTitle(&NSString::from_str(&title));
                entry.setSubmenu(Some(&build(mtm, target, children, values)));
                entry
            }
            Item::Action { title, key, checked, enabled, value, image } => {
                let (equivalent, modifiers) = key_equivalent(&key);
                // SAFETY: `choose:` is implemented by `Target` with the `(id)sender` signature
                // AppKit sends. Menu items hold their target weakly; the represented object
                // keeps it alive for as long as AppKit holds the item, even past this call.
                let entry = unsafe {
                    let entry = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&title),
                        Some(sel!(choose:)),
                        &NSString::from_str(&equivalent),
                    );
                    entry.setTarget(Some(target));
                    entry.setRepresentedObject(Some(target));
                    entry
                };
                entry.setKeyEquivalentModifierMask(modifiers);
                entry.setTag(values.len() as isize);
                entry.setEnabled(enabled);
                if checked {
                    entry.setState(NSControlStateValueOn);
                }
                if image.is_some() {
                    set_image(&entry, image.as_ref());
                }
                values.push(value);
                entry
            }
        };
        menu.addItem(&entry);
    }
    menu
}

/// A GPUI keystroke as an NSMenu key equivalent; named keys become AppKit's function-key
/// characters, and a key AppKit has no character for gets none.
fn key_equivalent(shortcut: &str) -> (String, NSEventModifierFlags) {
    let (mods, key) = match shortcut.strip_suffix("--") {
        Some(mods) => (mods, "-"),
        None => shortcut.rsplit_once('-').unwrap_or(("", shortcut)),
    };
    let parts: Vec<&str> = mods.split('-').collect();
    let key = match key {
        k if k.chars().count() == 1 => k.to_owned(),
        "space" => " ".to_owned(),
        "enter" => "\r".to_owned(),
        "tab" => "\t".to_owned(),
        "backspace" => "\u{8}".to_owned(),
        "escape" => "\u{1b}".to_owned(),
        "up" => "\u{f700}".to_owned(),
        "down" => "\u{f701}".to_owned(),
        "left" => "\u{f702}".to_owned(),
        "right" => "\u{f703}".to_owned(),
        "delete" => "\u{f728}".to_owned(),
        "home" => "\u{f729}".to_owned(),
        "end" => "\u{f72b}".to_owned(),
        "pageup" => "\u{f72c}".to_owned(),
        "pagedown" => "\u{f72d}".to_owned(),
        k => match k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=20).contains(n)) {
            Some(n) => char::from_u32(0xf703 + n).map(String::from).unwrap_or_default(),
            None => String::new(),
        },
    };
    let modifiers = parts.into_iter().fold(NSEventModifierFlags::empty(), |flags, part| {
        flags
            | match part {
                "cmd" => NSEventModifierFlags::Command,
                "shift" => NSEventModifierFlags::Shift,
                "alt" => NSEventModifierFlags::Option,
                "ctrl" => NSEventModifierFlags::Control,
                _ => NSEventModifierFlags::empty(),
            }
    });
    (key, modifiers)
}

/// Removes the event monitor when dropped.
pub struct EventMonitor(Retained<AnyObject>);

impl Drop for EventMonitor {
    fn drop(&mut self) {
        // SAFETY: the object was returned by `addLocalMonitorForEventsMatchingMask:handler:`.
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

impl Host {
    /// Calls `navigate(true)` for the mouse's back button and `navigate(false)` for forward,
    /// anywhere in the host's window including over web views, which ignore those buttons.
    pub fn on_navigation_buttons(&self, navigate: impl Fn(bool) + 'static) -> Option<EventMonitor> {
        let window = self.view.window()?.windowNumber();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event for the duration of the handler.
            let e = unsafe { event.as_ref() };
            let back = match e.buttonNumber() {
                3 => true,
                4 => false,
                _ => return event.as_ptr(),
            };
            if e.windowNumber() != window {
                return event.as_ptr();
            }
            navigate(back);
            std::ptr::null_mut()
        });
        // SAFETY: main-thread registration; AppKit copies the block and calls it on the main
        // thread with a valid event, and the monitor is removed when the returned guard drops.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::OtherMouseDown, &handler) }?;
        Some(EventMonitor(monitor))
    }

    /// Calls `moved(x, y, pressed)` for each pointer move the app sees, with the pointer in the
    /// host's top-left coordinates, over web views and the app's other windows too; `pressed`
    /// while any mouse button is down. Events name the window they started in, so the position is
    /// read from the screen instead.
    pub fn on_mouse_moved(&self, moved: impl Fn(f64, f64, bool) + 'static) -> Option<EventMonitor> {
        let host = self.view.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            if let Some(window) = host.window() {
                let p = host.convertPoint_fromView(window.convertPointFromScreen(NSEvent::mouseLocation()), None);
                let y = if host.isFlipped() { p.y } else { host.bounds().size.height - p.y };
                moved(p.x, y, NSEvent::pressedMouseButtons() != 0);
            }
            event.as_ptr()
        });
        let mask = NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged | NSEventMask::RightMouseDragged | NSEventMask::OtherMouseDragged;
        // SAFETY: as in `on_navigation_buttons`.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }?;
        Some(EventMonitor(monitor))
    }

    /// Calls `clicked` when a mouse button goes down over a web view in the host's window,
    /// which takes the click natively, so the shell never sees it.
    pub fn on_page_mouse_down(&self, clicked: impl Fn() + 'static) -> Option<EventMonitor> {
        let host = self.view.clone();
        let window = host.window()?.windowNumber();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event for the duration of the handler.
            let e = unsafe { event.as_ref() };
            if e.windowNumber() != window {
                return event.as_ptr();
            }
            // SAFETY: the handler runs on the main thread, and the superview is returned retained.
            if let Some(frame) = unsafe { host.superview() } {
                let point = frame.convertPoint_fromView(e.locationInWindow(), None);
                if host.hitTest(point).is_some_and(|hit| !std::ptr::eq(Retained::as_ptr(&hit), Retained::as_ptr(&host))) {
                    clicked();
                }
            }
            event.as_ptr()
        });
        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
        // SAFETY: as in `on_navigation_buttons`.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }?;
        Some(EventMonitor(monitor))
    }

    /// Calls `pressed` for Esc in the host's window, also while a web view has keyboard focus,
    /// and keeps the key from the page.
    pub fn on_escape(&self, pressed: impl Fn() + 'static) -> Option<EventMonitor> {
        const ESCAPE: u16 = 53;
        let window = self.view.window()?.windowNumber();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event for the duration of the handler.
            let e = unsafe { event.as_ref() };
            if e.windowNumber() != window || e.keyCode() != ESCAPE {
                return event.as_ptr();
            }
            pressed();
            std::ptr::null_mut()
        });
        // SAFETY: as in `on_navigation_buttons`.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) }?;
        Some(EventMonitor(monitor))
    }

    /// Calls `clicked(x, y)` for each left mouse-down in the host's window, in this view's
    /// coordinates (points, origin top-left); the click still reaches its target.
    pub fn on_mouse_down(&self, clicked: impl Fn(f64, f64) + 'static) -> Option<EventMonitor> {
        let window = self.view.window()?.windowNumber();
        let view = self.view.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event for the duration of the handler.
            let e = unsafe { event.as_ref() };
            if e.windowNumber() == window {
                let p = view.convertPoint_fromView(e.locationInWindow(), None);
                clicked(p.x, if view.isFlipped() { p.y } else { view.bounds().size.height - p.y });
            }
            event.as_ptr()
        });
        // SAFETY: as in `on_navigation_buttons`; the handler passes every event on unchanged.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseDown, &handler) }?;
        Some(EventMonitor(monitor))
    }
}

#[test]
fn key_equivalents_parse() {
    assert_eq!(key_equivalent("cmd-shift-t"), ("t".to_owned(), NSEventModifierFlags::Command | NSEventModifierFlags::Shift));
    assert_eq!(key_equivalent(""), (String::new(), NSEventModifierFlags::empty()));
    assert_eq!(key_equivalent("cmd--"), ("-".to_owned(), NSEventModifierFlags::Command));
    assert_eq!(key_equivalent("cmd-up").0, "\u{f700}");
    assert_eq!(key_equivalent("cmd-f5").0, "\u{f708}");
    assert_eq!(key_equivalent("cmd-tab").0, "\t");
    assert_eq!(key_equivalent("cmd-volumeup").0, "");
}
