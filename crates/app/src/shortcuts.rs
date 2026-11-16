use std::collections::HashMap;

use gpui::{
    Action, AnyElement, AnyView, App, Context, Div, FontWeight, InteractiveElement, IntoElement, KeyBinding, Keystroke, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, actions, div, px,
};
use ui::controls::{key_hint, surface};
use ui::theme::{Elevation, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

use crate::browser::Browser;
use crate::settings::SHORTCUTS;

actions!(tarn, [ToggleShortcuts]);

/// The keys of a shortcut-list row: an action the user can rebind with its default keys, or keys
/// fixed to a range of actions or to text editing, as shown.
pub(crate) enum Keys {
    Bound(&'static str, &'static dyn Action),
    Fixed(&'static str),
}

/// Keys no rebindable action may take: the fixed rows' bindings, text fields' own and ⌘⇥.
pub(crate) const RESERVED: [&str; 35] = [
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
];

const PREFIX: &str = "keymap.";

/// The setting holding the user's keys for an action.
pub(crate) fn setting_key(action: &dyn Action) -> String {
    format!("{PREFIX}{}", action.name())
}

/// Every rebindable row: label, default keys and action, in list order.
pub(crate) fn bindable() -> impl Iterator<Item = (&'static str, &'static str, &'static dyn Action)> {
    SHORTCUTS.iter().flat_map(|(_, rows)| rows.iter()).filter_map(|(label, keys)| match keys {
        Keys::Bound(keys, action) => Some((*label, *keys, *action)),
        Keys::Fixed(_) => None,
    })
}

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

/// The label of what already uses `keys`, other than `action`, given each action's keys.
/// Keys compare as GPUI binds them, so two canonical keys that land on one Windows key collide.
pub(crate) fn conflict(keys: &str, action: &dyn Action, keymap: &[(String, &'static dyn Action)]) -> Option<&'static str> {
    let bound = |keys: &str| Keystroke::parse(&platform(keys)).map(|k| k.unparse()).ok();
    let mine = bound(keys);
    // ⌘⇥, ⌃⌘Space and the Emacs keys are macOS's own; on Windows ⌘⇥ would land on Ctrl+Tab, Tarn's
    // next tab, and ⌃E on Ctrl+E, which ⌘E maps to.
    let os_only = |r: &&str| cfg!(windows) && matches!(*r, "cmd-tab" | "ctrl-cmd-space" | "ctrl-a" | "ctrl-e" | "ctrl-k");
    if RESERVED.iter().filter(|r| !os_only(r)).any(|r| bound(r) == mine) {
        return Some("a fixed shortcut");
    }
    let (_, other) = keymap.iter().find(|(k, a)| !macos_only(*a) && bound(k) == mine && !a.partial_eq(action))?;
    bindable().find(|(_, _, a)| a.partial_eq(*other)).map(|(label, ..)| label)
}

/// Each rebindable action's keys: the valid stored ones, else the default. Saving and resetting
/// refuse taken keys, so a stored key that still collides (an edited database) falls back.
pub(crate) fn keymap(stored: &HashMap<String, String>) -> Vec<(String, &'static dyn Action)> {
    let default = |keys: &str| parse_keys(keys).unwrap_or_else(|_| keys.to_owned());
    let chosen = |action: &dyn Action| stored.get(&setting_key(action)).and_then(|k| parse_keys(k).ok());
    let mut keymap: Vec<(String, &'static dyn Action)> = bindable().map(|(_, keys, a)| (chosen(a).unwrap_or_else(|| default(keys)), a)).collect();
    // A fallback can collide with a row already checked, so repeat until nothing changes.
    let mut changed = true;
    while changed {
        changed = false;
        for (ix, (_, keys, action)) in bindable().enumerate() {
            if keymap[ix].0 != default(keys) && conflict(&keymap[ix].0, action, &keymap).is_some() {
                keymap[ix].0 = default(keys);
                changed = true;
            }
        }
    }
    keymap
}

/// The stored keys of every rebindable action.
pub(crate) fn stored(disk: &store::Store) -> HashMap<String, String> {
    disk.settings_with_prefix(PREFIX).unwrap_or_default().into_iter().collect()
}

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

/// Whether a shortcut row applies on this system: the macOS-only commands and the Windows menu key
/// are listed only where they work.
pub(crate) fn listed(keys: &Keys) -> bool {
    match keys {
        Keys::Fixed("F10") => cfg!(windows),
        Keys::Bound(_, action) => !macos_only(*action),
        Keys::Fixed(_) => true,
    }
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
        key => key.to_uppercase(),
    };
    mods + &key
}

/// The keys bound to `action` now, such as `cmd-shift-t`, or empty.
pub(crate) fn keys(action: &dyn Action, cx: &App) -> String {
    let bindings = cx.key_bindings();
    let keymap = bindings.borrow();
    keymap.bindings_for_action(action).last().and_then(|b| b.keystrokes().first()).map_or(String::new(), |k| canonical(&k.inner().unparse()))
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
    root.on_action(cx.listener(|b, _: &ToggleShortcuts, w, cx| b.toggle_shortcuts(w, cx)))
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
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        cx.notify();
    }
}

pub fn render(cx: &mut Context<Browser>) -> AnyElement {
    let column = |(title, items): &(&'static str, &'static [(&'static str, Keys)])| {
        div()
            .w(px(250.))
            .flex()
            .flex_col()
            .gap_1()
            .child(div().pb_1().text_size(px(TEXT_SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::muted()).child(title.to_uppercase()))
            .children(items.iter().filter(|(_, keys)| listed(keys)).map(|(action, keys)| {
                div().flex().gap_3().justify_between().child(div().child(*action)).child(
                    key_hint(match keys {
                        Keys::Bound(_, action) => shown(*action, cx),
                        Keys::Fixed(keys) => (*keys).to_owned(),
                    })
                    .flex_none(),
                )
            }))
    };
    let card = surface(Theme::radius_card())
        .id("shortcuts")
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
        .child(div().flex().flex_wrap().gap_6().children(SHORTCUTS.iter().map(column)));
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
        let defaults = keymap(&HashMap::new());
        assert_eq!(defaults.len(), bindable().count());
        for ((keys, action), (_, default, _)) in defaults.iter().zip(bindable()) {
            assert_eq!(parse_keys(default).as_deref(), Ok(keys.as_str()), "{default} is not a valid shortcut");
            assert_eq!(conflict(keys, *action, &defaults), None, "{default} is taken twice");
        }
    }

    #[test]
    fn taken_and_reserved_keys_conflict() {
        let map = keymap(&HashMap::new());
        assert_eq!(conflict("cmd-shift-t", &crate::NewTab, &map), Some("Reopen closed tab"));
        assert_eq!(conflict("cmd-shift-t", &crate::ReopenClosedTab, &map), None, "an action keeps its own keys");
        assert_eq!(conflict("cmd-c", &crate::NewTab, &map), Some("a fixed shortcut"));
        assert_eq!(conflict("cmd-3", &crate::NewTab, &map), Some("a fixed shortcut"));
        assert_eq!(conflict("cmd-shift-z", &crate::NewTab, &map), Some("a fixed shortcut"));
        assert_eq!(conflict("alt-cmd-k", &crate::NewTab, &map), None);
        assert_eq!(conflict("cmd-h", &crate::NewTab, &map), (!cfg!(windows)).then_some("Hide Tarn"), "macOS-only actions hold no keys on Windows");
    }

    #[test]
    fn stored_keys_may_take_defaults_that_moved_away() {
        let stored = HashMap::from([(setting_key(&crate::NewTab), "cmd-l".to_owned()), (setting_key(&crate::OpenLocation), "cmd-t".to_owned())]);
        let map = keymap(&stored);
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        assert_eq!((keys(&crate::NewTab), keys(&crate::OpenLocation)), (Some("cmd-l"), Some("cmd-t")));
    }

    #[test]
    fn a_fallback_never_leaves_two_actions_on_one_key() {
        let stored = HashMap::from([(setting_key(&crate::NewTab), "cmd-l".to_owned()), (setting_key(&crate::OpenLocation), "cmd-r".to_owned())]);
        let map = keymap(&stored);
        let keys = |action: &dyn Action| map.iter().find(|(_, a)| a.partial_eq(action)).map(|(k, _)| k.as_str());
        assert_eq!((keys(&crate::NewTab), keys(&crate::OpenLocation), keys(&crate::Reload)), (Some("cmd-t"), Some("cmd-l"), Some("cmd-r")));
        assert!(map.iter().all(|(k, a)| conflict(k, *a, &map).is_none()));
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
            (setting_key(&crate::NewTab), "alt-cmd-k".to_owned()),
            (setting_key(&crate::Reload), "cmd-shift-t".to_owned()),
            (setting_key(&crate::Back), "cmd-left".to_owned()),
            (setting_key(&crate::Print), "nonsense-".to_owned()),
            (setting_key(&crate::Find), "cmd-c".to_owned()),
        ]);
        assert_eq!(setting_key(&crate::NewTab), "keymap.tarn::NewTab");
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
