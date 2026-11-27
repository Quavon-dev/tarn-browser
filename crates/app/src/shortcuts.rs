use std::collections::HashMap;

use gpui::{
    Action, AnyElement, AnyView, App, Context, Div, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Keystroke, ParentElement,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, actions, div, point, px,
};
use ui::controls::{key_hint, surface};
use ui::theme::{Elevation, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

use browser_core::shortcuts::{self as core, Action as Shortcut, Keyboard, Keystrokes};

use crate::browser::Browser;

actions!(tarn, [ToggleShortcuts]);

/// The keys of a shortcut-list row: an action the user can rebind, or keys fixed to a range of
/// actions or to text editing, as shown.
pub(crate) enum Keys {
    Bound(Shortcut),
    Fixed(&'static str),
}

pub(crate) const FIXED: [(Shortcut, &str, &str); 11] = [
    (Shortcut::CloseAllTabs, "Select tab 1–8", "⌘1 ⌘2 ⌘3 ⌘4 ⌘5 ⌘6 ⌘7 ⌘8"),
    (Shortcut::PreviousTab, "Next or previous tab, other keys", "⌘} ⌘{ ⌥⌘→ ⌥⌘←"),
    (Shortcut::ZoomIn, "Zoom in, other key", "⌘+"),
    (Shortcut::Print, "Open link in a peek", "⇧-click"),
    (Shortcut::BurnerTab, "Switch to space 1–9", "⌃1 … ⌃9"),
    (Shortcut::ToggleShortcuts, "Open the menu", "F10"),
    (Shortcut::ToggleShortcuts, "Move between page, address field and sidebar", "F6  ⇧F6"),
    (Shortcut::ToggleShortcuts, "Close command bar, overlay, popover or link peek", "Esc"),
    (Shortcut::ToggleShortcuts, "Open or close the assistant", "⇧Esc"),
    (Shortcut::ToggleShortcuts, "Undo / Redo", "⌘Z  ⇧⌘Z"),
    (Shortcut::ToggleShortcuts, "Quit", "⌘Q"),
];

pub(crate) fn sections() -> Vec<(&'static str, Vec<(&'static str, Keys)>)> {
    core::SECTIONS
        .iter()
        .map(|(name, rows)| {
            let items = rows.iter().flat_map(|(label, action, _)| {
                let fixed = FIXED.iter().filter(move |(after, _, keys)| after == action && (*keys != "F10" || cfg!(windows)));
                MAC.offered(*action).then_some((*label, Keys::Bound(*action))).into_iter().chain(fixed.map(|(_, label, keys)| (*label, Keys::Fixed(keys))))
            });
            (*name, items.collect())
        })
        .collect()
}

/// Keys no rebindable action may take: the fixed rows' bindings, text fields' own and ⌘⇥.
pub(crate) const RESERVED: [&str; 40] = [
    "cmd-1",
    "cmd-2",
    "cmd-3",
    "cmd-4",
    "cmd-5",
    "cmd-6",
    "cmd-7",
    "cmd-8",
    "ctrl-1",
    "ctrl-2",
    "ctrl-3",
    "ctrl-4",
    "ctrl-5",
    "ctrl-6",
    "ctrl-7",
    "ctrl-8",
    "ctrl-9",
    "cmd-z",
    "cmd-shift-z",
    "cmd-x",
    "cmd-c",
    "cmd-v",
    "cmd-a",
    "cmd-left",
    "cmd-right",
    "shift-cmd-left",
    "shift-cmd-right",
    "cmd-backspace",
    "cmd-delete",
    "ctrl-cmd-space",
    "ctrl-a",
    "ctrl-e",
    "ctrl-k",
    "cmd-tab",
    "cmd-q",
    "cmd-+",
    "cmd-}",
    "cmd-{",
    "alt-cmd-right",
    "alt-cmd-left",
];

/// One keystroke in GPUI's canonical form, if it can be a shortcut: ⌘ or ⌃ with a key.
pub(crate) fn parse_keys(raw: &str) -> Result<String, &'static str> {
    let keystroke = Keystroke::parse(&raw.trim().replace("secondary-", "cmd-"))
        .ok()
        .filter(|_| !raw.trim().contains(char::is_whitespace))
        .ok_or("Not a key combination.")?;
    if ["shift", "control", "alt", "platform", "function"].contains(&keystroke.key.as_str()) {
        return Err("Add a key to the modifiers.");
    }
    if !(keystroke.modifiers.platform || keystroke.modifiers.control) {
        return Err(if cfg!(windows) { "Use Ctrl with the key, or Alt with Shift or a digit." } else { "Use ⌘ or ⌃ with the key." });
    }
    Ok(unparse(&keystroke))
}

/// GPUI's text for a keystroke with the platform key named `cmd` everywhere; GPUI says `win` on
/// Windows and `super` on Linux.
fn unparse(keystroke: &Keystroke) -> String {
    keystroke.unparse().replace("win-", "cmd-").replace("super-", "cmd-")
}

fn bound(keys: &str) -> Option<String> {
    Keystroke::parse(&platform(keys)).map(|k| k.unparse()).ok()
}

pub(crate) struct Mac;

pub(crate) const MAC: Mac = Mac;

impl Keystrokes for Mac {
    fn keyboard(&self) -> Keyboard {
        Keyboard::Apple
    }

    fn offered(&self, action: Shortcut) -> bool {
        gpui_action(action).is_some_and(|a| !macos_only(a))
    }

    fn written(&self, keys: &str) -> String {
        let keys = keys.replace("mod", "cmd").replace('+', "-");
        parse_keys(&keys).unwrap_or(keys)
    }

    fn parse(&self, raw: &str) -> Result<String, &'static str> {
        parse_keys(raw)
    }

    fn fixed(&self, keys: &str) -> bool {
        // ⌘⇥, ⌃⌘Space and the Emacs keys are macOS's own; on Windows ⌘⇥ would land on Ctrl+Tab, Tarn's
        // recent tab, and ⌃E on Ctrl+E, which ⌘E maps to.
        let os_only = |r: &&str| cfg!(windows) && matches!(*r, "cmd-tab" | "ctrl-cmd-space" | "ctrl-a" | "ctrl-e" | "ctrl-k");
        let mine = bound(keys);
        RESERVED.iter().filter(|r| !os_only(r)).any(|r| bound(r) == mine)
    }

    fn same(&self, a: &str, b: &str) -> bool {
        bound(a) == bound(b)
    }

    fn display(&self, keys: &str) -> String {
        ui::controls::keys(&display(keys))
    }
}

pub(crate) fn gpui_action(action: Shortcut) -> Option<&'static dyn Action> {
    use Shortcut as S;
    Some(match action {
        S::NewTab => &crate::NewTab,
        S::OpenLocation => &crate::OpenLocation,
        S::OpenFile => &crate::OpenFile,
        S::CloseTab => &crate::CloseTab,
        S::ReopenClosedTab => &crate::ReopenClosedTab,
        S::CloseAllTabs => &crate::CloseAllTabs,
        S::NextTab => &crate::NextTab,
        S::PreviousTab => &crate::PreviousTab,
        S::RecentTab => &crate::RecentTab,
        S::RecentTabBack => &crate::RecentTabBack,
        S::LastTab => &crate::LastTab,
        S::TogglePin => &crate::TogglePin,
        S::NewGroup => &crate::NewGroup,
        S::ToggleSplitView => &crate::split::ToggleSplitView,
        S::FocusNextPane => &crate::split::FocusNextPane,
        S::FocusPreviousPane => &crate::split::FocusPreviousPane,
        S::SwapPanes => &crate::split::SwapPanes,
        S::RotateSplitLayout => &crate::split::RotateSplitLayout,
        S::RemoveFromSplitView => &crate::split::RemoveFromSplitView,
        S::Reload => &crate::Reload,
        S::ReloadFromOrigin => &crate::ReloadFromOrigin,
        S::StopLoading => &crate::StopLoading,
        S::Back => &crate::Back,
        S::Forward => &crate::Forward,
        S::Find => &crate::Find,
        S::FindNext => &crate::FindNext,
        S::FindPrevious => &crate::FindPrevious,
        S::FindInTabs => &crate::find_tabs::FindInTabs,
        S::ZoomIn => &crate::ZoomIn,
        S::ZoomOut => &crate::ZoomOut,
        S::ZoomReset => &crate::ZoomReset,
        S::CopyLink => &crate::CopyLink,
        S::CopyCleanLink => &crate::CopyCleanLink,
        S::CopyMarkdownLink => &crate::capture::CopyMarkdownLink,
        S::TakeScreenshot => &crate::capture::TakeScreenshot,
        S::ToggleReader => &crate::reader::ToggleReader,
        S::PictureInPicture => &crate::capture::PictureInPicture,
        S::Print => &crate::Print,
        S::BookmarkTab => &crate::BookmarkTab,
        S::ToggleLibrary => &crate::ToggleLibrary,
        S::ToggleDownloads => &crate::ToggleDownloads,
        S::ManageBookmarks => &crate::menubar::ManageBookmarks,
        S::ShowHistory => &crate::menubar::ShowHistory,
        S::NewWindow => &crate::NewWindow,
        S::CloseWindow => &crate::CloseWindow,
        S::ToggleSidebar => &crate::ToggleSidebar,
        S::ToggleTabsOnTop => &crate::ToggleTabsOnTop,
        S::NewSpace => &crate::NewSpace,
        S::PrivateSpace => &crate::PrivateSpace,
        S::BurnerTab => &crate::BurnerTab,
        S::Minimize => &crate::menubar::Minimize,
        S::ToggleFullScreen => &crate::ToggleFullScreen,
        S::HideTarn => &crate::HideTarn,
        S::HideOthers => &crate::HideOthers,
        S::Settings => &crate::Settings,
        S::ToggleShortcuts => &ToggleShortcuts,
        S::CommandBar | S::SelectTab(_) | S::SwitchSpace(_) => return None,
    })
}

pub(crate) fn keymap(stored: &HashMap<String, String>) -> Vec<(String, &'static dyn Action)> {
    core::keymap(&MAC, stored).into_iter().filter_map(|(keys, action)| Some((keys, gpui_action(action)?))).collect()
}

pub(crate) use core::stored;

/// Binds every key again with the stored overrides and lets the menus show the new keys.
pub(crate) fn rebind(cx: &mut App) {
    let stored = stored(&crate::shared::core(cx).disk);
    cx.clear_key_bindings();
    crate::bind_keys(cx, &keymap(&stored));
    crate::shared::each(cx, |b, _, _| b.menu.reset());
    // Settings is in front while recording, and the menu bar lists the front browser's menus.
    if let Some(front) = cx.window_stack().into_iter().flatten().find_map(|w| w.downcast::<crate::browser::Browser>()) {
        front.update(cx, |b, _, cx| b.refresh_menus(cx)).ok();
    }
    cx.refresh_windows();
}

/// Hiding apps and bringing every window forward are macOS-only; GPUI on Windows does not do them.
pub(crate) fn macos_only(action: &dyn Action) -> bool {
    cfg!(windows)
        && (action.partial_eq(&crate::HideTarn)
            || action.partial_eq(&crate::HideOthers)
            || action.partial_eq(&crate::ShowAll)
            || action.partial_eq(&crate::menubar::BringAllToFront))
}

pub(crate) fn binding(keys: &str, action: &dyn Action) -> Option<KeyBinding> {
    if macos_only(action) {
        return None;
    }
    KeyBinding::load(&platform(keys), action.boxed_clone(), None, false, None, &gpui::DummyKeyboardMapper).ok()
}

/// The keys GPUI binds for canonical keys on this system. Ctrl is the shortcut key on Windows,
/// so ⌘ becomes Ctrl there, ⌃ with a digit becomes Alt, ⌃⌘ becomes Alt+Shift and ⌃⌘F is F11.
pub(crate) fn platform(keys: &str) -> String {
    if cfg!(windows) { to_windows(keys) } else { keys.to_owned() }
}

/// Canonical keys for keys GPUI bound on this system; the inverse of `platform`.
pub(crate) fn canonical(keys: &str) -> String {
    if cfg!(windows) { from_windows(keys) } else { keys.to_owned() }
}

fn chord(ctrl: bool, alt: bool, shift: bool, cmd: bool, key: &str) -> String {
    [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift"), (cmd, "cmd")].iter().filter(|(on, _)| *on).map(|(_, m)| *m).chain([key]).collect::<Vec<_>>().join("-")
}

fn to_windows(keys: &str) -> String {
    let Ok(k) = Keystroke::parse(keys) else { return keys.to_owned() };
    let (m, key) = (k.modifiers, k.key.as_str());
    match (m.platform, m.control) {
        (true, true) if key == "f" => "f11".to_owned(),
        (true, true) => chord(false, true, true, false, key),
        (true, false) => chord(true, m.alt, m.shift, false, key),
        (false, true) if key.chars().all(|c| c.is_ascii_digit()) => chord(false, true, m.shift, false, key),
        _ => keys.to_owned(),
    }
}

fn from_windows(keys: &str) -> String {
    let Ok(k) = Keystroke::parse(keys) else { return keys.to_owned() };
    let (m, key) = (k.modifiers, k.key.as_str());
    match (m.control, m.alt, m.shift) {
        _ if key == "f11" && !m.control && !m.alt && !m.shift => chord(true, false, false, true, "f"),
        (false, true, true) => chord(true, false, false, true, key),
        (false, true, false) if key.chars().all(|c| c.is_ascii_digit()) => chord(true, false, false, false, key),
        (true, _, _) if key != "tab" => chord(false, m.alt, m.shift, true, key),
        _ => keys.to_owned(),
    }
}

/// `shift-cmd-t` as `⇧⌘T`.
pub(crate) fn display(keys: &str) -> String {
    let Ok(k) = Keystroke::parse(keys) else { return keys.to_owned() };
    let m = k.modifiers;
    let mods = [(m.control, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.platform, "⌘")].iter().filter(|(on, _)| *on).map(|(_, s)| *s).collect::<String>();
    let key = match k.key.as_str() {
        "tab" => "⇥".to_owned(),
        "escape" => "Esc".to_owned(),
        "enter" => "↩".to_owned(),
        "space" => "Space".to_owned(),
        "backspace" => "⌫".to_owned(),
        "delete" => "⌦".to_owned(),
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        "pageup" => "⇞".to_owned(),
        "pagedown" => "⇟".to_owned(),
        key => key.to_uppercase(),
    };
    mods + &key
}

/// The keys bound to `action` now, such as `cmd-shift-t`, or empty: the first ones, which GPUI's
/// menu bar shows too and which are the rebindable ones.
pub(crate) fn keys(action: &dyn Action, cx: &App) -> String {
    let bindings = cx.key_bindings();
    let keymap = bindings.borrow();
    keymap.bindings_for_action(action).next().and_then(|b| b.keystrokes().first()).map_or(String::new(), |k| canonical(&k.inner().unparse()))
}

/// The keys bound to `action` now, as shown, or empty.
pub(crate) fn shown(action: &dyn Action, cx: &App) -> String {
    let keys = keys(action, cx);
    if keys.is_empty() { keys } else { ui::controls::keys(&display(&keys)) }
}

/// A tooltip naming `action`'s keys, looked up when it opens rather than on every frame.
pub(crate) fn tip(text: impl Into<SharedString>, action: &'static dyn Action) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| ui::controls::tooltip(text.clone(), Some(shown(action, cx).into()))(window, cx)
}

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ToggleShortcuts, w, cx| b.toggle_shortcuts(w, cx))).on_key_down(cx.listener(|b, e: &KeyDownEvent, _, cx| {
        if b.shortcuts_open && scroll_sheet(&b.shortcuts_scroll, &e.keystroke.key) {
            cx.stop_propagation();
            cx.notify();
        }
    }))
}

/// Scrolls the sheet for an arrow, page or Home and End key; false for any other key.
fn scroll_sheet(scroll: &ScrollHandle, key: &str) -> bool {
    const LINE: f32 = 40.;
    let (y, page, end) = (f32::from(scroll.offset().y), f32::from(scroll.bounds().size.height) - LINE, f32::from(scroll.max_offset().height));
    let y = match key {
        "down" => y - LINE,
        "up" => y + LINE,
        "pagedown" | "space" => y - page,
        "pageup" => y + page,
        "home" => 0.,
        "end" => -end,
        _ => return false,
    };
    scroll.set_offset(point(px(0.), px(y.clamp(-end, 0.))));
    true
}

impl Browser {
    /// The cheat sheet covers the page like the command card, so the web view is hidden under it.
    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.shortcuts_open;
        let kept = self.reader.take().filter(|r| crate::reader::keeps_reader(open, r.speaking()));
        self.close_command(window, cx);
        self.reader = kept.or(self.reader.take());
        if !open {
            return;
        }
        self.shortcuts_open = true;
        self.shortcuts_scroll.set_offset(point(px(0.), px(0.)));
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }
}

pub fn render(scroll: &ScrollHandle, cx: &mut Context<Browser>) -> AnyElement {
    let column = |(title, items): &(&'static str, Vec<(&'static str, Keys)>)| {
        div()
            .w(px(250.))
            .flex()
            .flex_col()
            .gap_1()
            .child(div().pb_1().text_size(px(TEXT_SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::muted()).child(title.to_uppercase()))
            .children(items.iter().map(|(action, keys)| {
                div().flex().flex_wrap().gap_x_3().child(div().max_w_full().child(*action)).child(
                    key_hint(match keys {
                        Keys::Bound(action) => gpui_action(*action).map_or(String::new(), |a| shown(a, cx)),
                        Keys::Fixed(keys) => (*keys).to_owned(),
                    })
                    .flex_none()
                    .ml_auto(),
                )
            }))
    };
    let card = surface(Theme::radius_card())
        .id("shortcuts")
        .track_scroll(scroll)
        .w(px(1100.))
        .max_w_full()
        .max_h_full()
        .overflow_y_scroll()
        .p_5()
        .flex()
        .flex_col()
        .gap_4()
        .shadow(Elevation::Modal.shadow())
        .text_size(px(TEXT))
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(div().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).child("Keyboard Shortcuts"))
                .child(key_hint("Esc to close")),
        )
        .child(div().flex().flex_wrap().gap_6().children(sections().iter().map(column)));
    // A click beside the sheet closes it, as one outside a pop-up does.
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .justify_center()
        .items_start()
        .pt(px(48.))
        .px_4()
        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|b, _, w, cx| b.dismiss_overlay(w, cx)))
        .child(card.occlude())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[test]
    fn windows_keys_round_trip() {
        for (mac, win) in [
            ("cmd-shift-t", "ctrl-shift-t"),
            ("ctrl-3", "alt-3"),
            ("ctrl-cmd-n", "alt-shift-n"),
            ("ctrl-cmd-]", "alt-shift-]"),
            ("ctrl-cmd-f", "f11"),
            ("alt-cmd-g", "ctrl-alt-g"),
            ("ctrl-tab", "ctrl-tab"),
            ("cmd-,", "ctrl-,"),
        ] {
            assert_eq!(super::to_windows(mac), win);
            assert_eq!(super::to_windows(&super::from_windows(win)), win, "{win}");
        }
    }

    use super::*;

    #[test]
    fn keys_parse_to_one_canonical_keystroke_with_cmd_or_ctrl() {
        assert_eq!(parse_keys("shift-cmd-T"), Ok("cmd-shift-t".to_owned()));
        assert_eq!(parse_keys(" alt-cmd-k "), Ok("alt-cmd-k".to_owned()));
        assert_eq!(parse_keys("ctrl-tab"), Ok("ctrl-tab".to_owned()));
        assert_eq!(parse_keys("cmd--"), Ok("cmd--".to_owned()));
        assert!(parse_keys("t").is_err() && parse_keys("shift-alt-t").is_err(), "plain typing keys stay free");
        assert!(parse_keys("cmd-shift").is_err() && parse_keys("cmd").is_err());
        assert!(parse_keys("cmd-k cmd-j").is_err() && parse_keys("").is_err() && parse_keys("cmd-k-j").is_err());
        assert_eq!(display("ctrl-alt-cmd-shift-t"), "⌃⌥⇧⌘T");
        assert_eq!(display("ctrl-shift-tab"), "⌃⇧⇥");
    }

    #[test]
    fn the_defaults_are_valid_and_distinct() {
        let defaults = core::keymap(&MAC, &HashMap::new());
        assert_eq!(defaults.len(), core::rebindable(&MAC).count());
        for (keys, action) in &defaults {
            assert_eq!(parse_keys(keys).as_deref(), Ok(keys.as_str()), "{action:?} has no valid default");
            assert_eq!(core::conflict(&MAC, keys, *action, &defaults), None, "{keys} is taken twice");
        }
    }

    #[test]
    fn every_offered_action_keeps_its_stored_setting_and_default() {
        for (_, action) in core::rebindable(&MAC) {
            let gpui = gpui_action(action).unwrap();
            assert_eq!(action.setting_key(), format!("keymap.{}", gpui.name()), "keys stored for {action:?} still apply");
        }
        let map = keymap(&HashMap::new());
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        for (action, default) in [
            (&crate::Back as &dyn Action, "cmd-["),
            (&crate::split::ToggleSplitView, "ctrl-cmd-n"),
            (&crate::capture::CopyMarkdownLink, "alt-shift-cmd-c"),
            (&crate::capture::TakeScreenshot, "alt-cmd-s"),
            (&crate::reader::ToggleReader, "alt-cmd-r"),
            (&crate::menubar::ShowHistory, "cmd-y"),
            (&crate::ToggleDownloads, "shift-cmd-j"),
            (&crate::CopyCleanLink, "ctrl-shift-cmd-c"),
            (&crate::ToggleFullScreen, "ctrl-cmd-f"),
            (&crate::StopLoading, "cmd-."),
            (&crate::find_tabs::FindInTabs, "shift-cmd-f"),
        ] {
            assert_eq!(keys(action).map(|k| parse_keys(k).unwrap()), Some(parse_keys(default).unwrap()), "{}", action.name());
        }
        assert!(sections().iter().all(|(_, rows)| rows.iter().all(|(_, keys)| !matches!(keys, Keys::Bound(Shortcut::CommandBar)))));
    }

    #[test]
    fn taken_and_reserved_keys_conflict() {
        let map = core::keymap(&MAC, &HashMap::new());
        let conflict = |keys: &str, action| core::conflict(&MAC, keys, action, &map);
        assert_eq!(conflict("cmd-shift-t", Shortcut::NewTab), Some("Reopen closed tab"));
        assert_eq!(conflict("cmd-shift-t", Shortcut::ReopenClosedTab), None, "an action keeps its own keys");
        assert_eq!(conflict("cmd-c", Shortcut::NewTab), Some("a fixed shortcut"));
        assert_eq!(conflict("cmd-3", Shortcut::NewTab), Some("a fixed shortcut"));
        assert_eq!(conflict("cmd-shift-z", Shortcut::NewTab), Some("a fixed shortcut"));
        assert_eq!(conflict("alt-cmd-k", Shortcut::NewTab), None);
        assert_eq!(conflict("cmd-h", Shortcut::NewTab), (!cfg!(windows)).then_some("Hide Tarn"), "macOS-only actions hold no keys on Windows");
    }

    #[test]
    fn stored_keys_may_take_defaults_that_moved_away() {
        let stored = HashMap::from([(Shortcut::NewTab.setting_key(), "cmd-l".to_owned()), (Shortcut::OpenLocation.setting_key(), "cmd-t".to_owned())]);
        let map = keymap(&stored);
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        assert_eq!((keys(&crate::NewTab), keys(&crate::OpenLocation)), (Some("cmd-l"), Some("cmd-t")));
    }

    #[test]
    fn a_fallback_never_leaves_two_actions_on_one_key() {
        let stored = HashMap::from([(Shortcut::NewTab.setting_key(), "cmd-l".to_owned()), (Shortcut::OpenLocation.setting_key(), "cmd-r".to_owned())]);
        let map = keymap(&stored);
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        assert_eq!((keys(&crate::NewTab), keys(&crate::OpenLocation), keys(&crate::Reload)), (Some("cmd-t"), Some("cmd-l"), Some("cmd-r")));
        let map = core::keymap(&MAC, &stored);
        assert!(map.iter().all(|(k, a)| core::conflict(&MAC, k, *a, &map).is_none()));
    }

    #[test]
    fn fixed_and_text_field_bindings_are_reserved() {
        let reserved: Vec<String> = RESERVED.iter().map(|r| unparse(&Keystroke::parse(r).unwrap())).collect();
        for source in [include_str!("lib.rs"), include_str!("../../ui/src/text_input.rs")] {
            for binding in source.replace("KeyBinding::new(&shortcuts::platform(\"", "KeyBinding::new(\"").split("KeyBinding::new(\"").skip(1) {
                let keys = binding.split('"').next().unwrap_or_default();
                if let Ok(keys) = parse_keys(keys) {
                    assert!(reserved.contains(&keys), "{keys} is bound but not reserved");
                }
            }
        }
    }

    #[test]
    fn valid_free_overrides_replace_the_defaults() {
        let stored = HashMap::from([
            (Shortcut::NewTab.setting_key(), "alt-cmd-k".to_owned()),
            (Shortcut::Reload.setting_key(), "cmd-shift-t".to_owned()),
            (Shortcut::Back.setting_key(), "cmd-left".to_owned()),
            (Shortcut::Print.setting_key(), "nonsense-".to_owned()),
            (Shortcut::Find.setting_key(), "cmd-c".to_owned()),
        ]);
        assert_eq!(Shortcut::NewTab.setting_key(), "keymap.tarn::NewTab");
        let map = keymap(&stored);
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        assert_eq!(keys(&crate::NewTab), Some("alt-cmd-k"));
        assert_eq!(keys(&crate::Reload), Some("cmd-r"), "taken by Reopen closed tab");
        assert_eq!(keys(&crate::NewGroup), Some("alt-cmd-g"));
        assert_eq!(keys(&crate::Print), Some("cmd-p"), "unparsable");
        assert_eq!(keys(&crate::Find), Some("cmd-f"), "reserved");
        assert_eq!(keys(&crate::Back), Some("cmd-["), "a text field's own");
        assert_eq!(keys(&crate::CloseTab), Some("cmd-w"));
    }
}
