use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Context, Div, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px, relative, rgb, transparent_black,
};
use shield::ShieldLevel;
use store::{Id, Profile};
use ui::command::ENGINES;
use ui::controls::{button, field, key_hint, radio, segment, segmented, surface, swatch, switch};
use ui::icon::{Icon, icon};
use ui::motion::pop_in;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{PROFILE_COLORS, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

use browser_core::{share_store, shared_key};

use crate::browser::Browser;
use crate::prefs::{Pref, option_index};
use crate::settings::level_description;
use crate::shared::AppCore;
use crate::spaces::SpaceCard;
use crate::toolbar::shield_badge;

actions!(profiles, [NewProfile, EditProfile, ConfirmProfile, NextField, PreviousField, NextOption, PreviousOption]);

const MAX_NAME: usize = 48;
const DEFAULT_COLOR: usize = 2;

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("ProfileEditor");
    cx.bind_keys([
        KeyBinding::new("enter", ConfirmProfile, ctx),
        KeyBinding::new("tab", NextField, ctx),
        KeyBinding::new("shift-tab", PreviousField, ctx),
        KeyBinding::new("right", NextOption, ctx),
        KeyBinding::new("down", NextOption, ctx),
        KeyBinding::new("left", PreviousOption, ctx),
        KeyBinding::new("up", PreviousOption, ctx),
    ]);
}

pub struct ProfileEditor {
    editing: Option<Id>,
    protection: bool,
    pub(crate) name: Entity<TextInput>,
    pub(crate) color: usize,
    share: Option<Id>,
    level: ShieldLevel,
    engine: usize,
    pub(crate) field: usize,
    pub(crate) focus: FocusHandle,
    pub(crate) space: Option<SpaceCard>,
    _subscription: Subscription,
}

/// The trimmed name without control or bidi characters, if it is non-empty and not taken.
pub fn valid_name<'a>(raw: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let name: String = raw.trim().chars().filter(|c| !c.is_control() && !ui::text::is_bidi(*c)).take(MAX_NAME).collect();
    let name = name.trim_end().to_owned();
    let free = taken.into_iter().all(|t| !t.trim().eq_ignore_ascii_case(&name));
    (!name.is_empty() && free).then_some(name)
}

use browser_core::command::engine_key;

/// Stores of deleted profiles or of "Delete everything" that WebKit has not removed yet.
const RETIRED_STORES: &str = "retired_stores";

/// Only stores Tarn retired itself: other data folders share WebKit's store folder.
pub(crate) fn delete_retired_stores(core: &Rc<AppCore>) {
    let listed = core.disk.setting(RETIRED_STORES).ok().flatten().unwrap_or_default();
    for uuid in listed.lines().filter(|u| !u.is_empty()) {
        delete_store(core.clone(), uuid.to_owned());
    }
}

fn delete_store(core: Rc<AppCore>, uuid: String) {
    engine::DataStore::delete_persistent(&uuid.clone(), move |gone| {
        if gone {
            let listed = core.disk.setting(RETIRED_STORES).ok().flatten().unwrap_or_default();
            core.disk.set_setting(RETIRED_STORES, &toggled(&listed, &uuid, false)).ok();
        }
    });
}

impl Browser {
    /// Removes the store a few seconds from now, once its views and extension pages let go of it,
    /// and keeps it listed until WebKit confirms, so a later launch retries.
    pub(crate) fn retire_store(&mut self, uuid: &str, cx: &mut Context<Self>) {
        let listed = self.core.disk.setting(RETIRED_STORES).ok().flatten().unwrap_or_default();
        self.core.disk.set_setting(RETIRED_STORES, &toggled(&listed, uuid, true)).ok();
        let (uuid, core) = (uuid.to_owned(), self.core.clone());
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            delete_store(core, uuid);
        })
        .detach();
    }

    /// The Chromium side of deleting a profile's website data, once its Chromium views are
    /// closed.
    #[cfg(feature = "chromium")]
    pub(crate) fn wipe_chromium(&self, own_store: &str, to: &str) {
        engine::chromium::wipe(&crate::browser::data_dir().join("Chromium"), own_store, to);
    }

    /// The Chromium side of clearing one site.
    #[cfg(feature = "chromium")]
    pub(crate) fn clear_chromium_site(&self, own_store: &str, host: &str) {
        engine::chromium::clear_site(&crate::browser::data_dir().join("Chromium"), own_store, host);
    }

    /// Whether this profile borrows another's store or another profile borrows this one's.
    pub(crate) fn store_is_shared(&self, id: Id, own_store: &str) -> bool {
        self.core.disk.setting(&shared_key(id)).ok().flatten().is_some_and(|u| !u.is_empty())
            || self
                .core
                .disk
                .profiles()
                .unwrap_or_default()
                .iter()
                .any(|p| p.id != id && self.core.disk.setting(&shared_key(p.id)).ok().flatten().as_deref() == Some(own_store))
    }
}

pub fn profile_dot(color: &str, size: f32) -> Div {
    div().w(px(size)).h(px(size)).flex_none().rounded_full().bg(Theme::profile_color(color).unwrap_or(Theme::faint()))
}

impl Browser {
    pub fn search_engine(&self) -> usize {
        let Some(space) = self.model.current_space() else { return 0 };
        let names = ENGINES.map(|e| e.name);
        let stored = self.core.disk.setting(&engine_key(space.private, space.space.profile_id)).ok().flatten();
        option_index(&names, stored.as_deref()).unwrap_or(0)
    }

    pub fn set_search_engine(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space() else { return };
        let key = engine_key(space.private, space.space.profile_id);
        if let Some(engine) = ENGINES.get(ix)
            && self.core.disk.set_setting(&key, engine.name).is_ok()
        {
            cx.notify();
        }
    }

    fn other_profiles(&self, except: Option<Id>) -> Vec<Profile> {
        self.core.disk.profiles().unwrap_or_default().into_iter().filter(|p| Some(p.id) != except).collect()
    }

    pub(crate) fn editor_name(&self, cx: &App) -> Option<String> {
        let editor = self.profile_editor.as_ref()?;
        let others = self.other_profiles(editor.editing);
        valid_name(editor.name.read(cx).text(), others.iter().map(|p| p.name.as_str()))
    }

    pub fn open_profile_editor(&mut self, editing: Option<Id>, window: &mut Window, cx: &mut Context<Self>) {
        let profile = editing.and_then(|id| self.core.disk.profile(id).ok());
        if editing.is_some() && profile.is_none() {
            return;
        }
        self.command_mode = None;
        self.forget_text_matches();
        self.overlay_exit = None;
        self.reader = None;
        let name = cx.new(|cx| TextInput::new("Profile Name", cx));
        if let Some(p) = &profile {
            name.update(cx, |input, cx| input.set_text(p.name.clone(), cx));
        }
        let subscription = cx.subscribe_in(&name, window, |b, _, event: &TextInputEvent, window, cx| match event {
            TextInputEvent::Submit => b.confirm_profile(window, cx),
            TextInputEvent::Cancel => b.dismiss_overlay(window, cx),
            _ => cx.notify(),
        });
        let color = profile.and_then(|p| PROFILE_COLORS.iter().position(|(n, _)| *n == p.color)).unwrap_or(DEFAULT_COLOR);
        self.profile_editor = Some(ProfileEditor {
            editing,
            protection: false,
            name: name.clone(),
            color,
            share: None,
            level: ShieldLevel::Standard,
            engine: 0,
            field: 0,
            focus: cx.focus_handle(),
            space: None,
            _subscription: subscription,
        });
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        window.activate_window();
        window.focus(&name.focus_handle(cx));
        cx.notify();
    }

    pub fn edit_current_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(space) = self.model.current_space().filter(|s| !s.private) {
            self.open_profile_editor(Some(space.space.profile_id), window, cx);
        }
    }

    pub(crate) fn update_editor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ProfileEditor)) {
        if let Some(editor) = &mut self.profile_editor {
            f(editor);
            cx.notify();
        }
    }

    fn profile_step(&mut self, protection: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.profile_editor else { return };
        editor.protection = protection;
        editor.field = 0;
        let focus = if protection { editor.focus.clone() } else { editor.name.focus_handle(cx) };
        window.focus(&focus);
        cx.notify();
    }

    fn field_count(&self) -> usize {
        let Some(editor) = &self.profile_editor else { return 0 };
        if editor.protection {
            return 2;
        }
        let sharing = editor.editing.is_none() && !self.other_profiles(None).is_empty();
        2 + usize::from(sharing) + usize::from(sharing && editor.share.is_some())
    }

    fn move_field(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.space_card().is_some() {
            return self.move_space_field(step, window, cx);
        }
        let count = self.field_count() as isize;
        let Some(editor) = &mut self.profile_editor else { return };
        editor.field = (editor.field as isize + step).rem_euclid(count.max(1)) as usize;
        let focus = if editor.field == 0 && !editor.protection { editor.name.focus_handle(cx) } else { editor.focus.clone() };
        window.focus(&focus);
        cx.notify();
    }

    fn move_option(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.space_card().is_some() {
            return self.move_space_option(step, cx);
        }
        let candidates: Vec<Id> = self.other_profiles(None).iter().map(|p| p.id).collect();
        let shift = |ix: usize, len: usize| (ix as isize + step).clamp(0, len as isize - 1) as usize;
        self.update_editor(cx, |e| match (e.protection, e.field) {
            (true, 0) => {
                let ix = ShieldLevel::ALL.iter().position(|l| *l == e.level).unwrap_or(0);
                e.level = ShieldLevel::ALL[shift(ix, ShieldLevel::ALL.len())];
            }
            (true, _) => e.engine = shift(e.engine, ENGINES.len()),
            (false, 1) => e.color = (e.color as isize + step).rem_euclid(PROFILE_COLORS.len() as isize) as usize,
            (false, 2) => e.share = if step > 0 { candidates.first().copied() } else { None },
            (false, 3) => {
                let ix = candidates.iter().position(|id| Some(*id) == e.share).unwrap_or(0);
                e.share = candidates.get(shift(ix, candidates.len())).copied();
            }
            _ => {}
        });
    }

    pub fn confirm_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.space_card().is_some() {
            return self.confirm_space(window, cx);
        }
        let Some(name) = self.editor_name(cx) else { return };
        let Some(editor) = &self.profile_editor else { return };
        if editor.editing.is_none() && !editor.protection {
            return self.profile_step(true, window, cx);
        }
        let Some(editor) = self.profile_editor.take() else { return };
        self.close_command(window, cx);
        let color = PROFILE_COLORS[editor.color].0;
        match editor.editing {
            Some(id) => {
                if self.core.disk.set_profile(id, &name, color).is_ok()
                    && let Some(rt) = self.core.profiles.borrow_mut().get_mut(&(false, id))
                {
                    rt.profile.name = name;
                    rt.profile.color = color.into();
                }
            }
            None => self.create_profile(&editor, &name, color, ["Home", "", ""], window, cx),
        }
    }

    /// Writes the profile with its first space, given as `[name, color, icon]`, and switches to it.
    pub(crate) fn create_profile(&mut self, editor: &ProfileEditor, name: &str, color: &str, space: [&str; 3], window: &mut Window, cx: &mut Context<Self>) {
        let shared = editor.share.and_then(|id| self.core.disk.profile(id).ok()).map(|mut p| {
            share_store(&self.core.disk, &mut p);
            p.data_store_uuid
        });
        let Ok(profile) = self.core.disk.create_profile(name, &uuid::Uuid::new_v4().to_string(), false) else { return };
        let id = profile.id;
        let written = self
            .core
            .disk
            .set_profile(id, name, color)
            .and_then(|_| self.core.disk.set_profile_shield_level(id, editor.level))
            .and_then(|_| self.core.disk.set_setting(&shared_key(id), shared.as_deref().unwrap_or("")))
            .and_then(|_| self.core.disk.set_setting(&engine_key(false, id), ENGINES[editor.engine].name))
            .and_then(|_| self.core.disk.create_space(id, space[0], space[2]))
            .and_then(|s| self.core.disk.update_space(s.id, space[0], space[1], space[2]));
        if let Err(e) = written {
            self.core.disk.delete_profile(id).ok();
            self.warning = Some(format!("The profile could not be created ({e})."));
            cx.notify();
            return;
        }
        self.reload_spaces();
        if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.profile_id == id) {
            self.switch_space(ix, window, cx);
        }
    }

    /// Deletes a saved profile with its spaces, tabs and history; website data goes too unless
    /// another profile still uses the same store.
    pub fn delete_profile(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let profiles = self.core.disk.profiles().unwrap_or_default();
        let Some(mut profile) = profiles.iter().find(|p| p.id == id).cloned().filter(|_| profiles.len() > 1) else { return };
        let own_store = profile.data_store_uuid.clone();
        share_store(&self.core.disk, &mut profile);
        let keys: Vec<_> = self.views.keys().filter(|k| !k.private && self.profile_of(**k) == Some(id)).copied().collect();
        for key in keys {
            self.hibernate(key, cx);
        }
        let current = self.model.current_space().map(|s| (s.private, s.space.id));
        if self.core.disk.delete_profile(id).is_err() {
            return;
        }
        Self::sync_other_windows(cx);
        crate::shared::others(cx, |b, _, _| b.forget_history(false, id));
        #[cfg(feature = "chromium")]
        self.wipe_chromium(&own_store, &own_store);
        self.core.disk.delete_setting(&shared_key(id)).ok();
        self.core.disk.delete_setting(&engine_key(false, id)).ok();
        self.core.disk.delete_setting(&browser_core::theme::pages_setting(id)).ok();
        crate::ext_package::remove_tree(&crate::extensions::root().join(id.to_string())).ok();
        let still_used = profiles.into_iter().filter(|p| p.id != id).any(|mut p| {
            share_store(&self.core.disk, &mut p);
            p.data_store_uuid == profile.data_store_uuid
        });
        #[cfg(windows)]
        crate::shared::count_chromium_blocks(&self.core);
        if let Some(rt) = self.core.profiles.borrow_mut().remove(&(false, id)) {
            rt.data.set_extensions(&[], &[], rt.profile.shield_level);
            if still_used || self.downloads_popover.active.get(&(false, id)).is_some_and(|n| *n > 0) {
                self.core.retired_stores.borrow_mut().push(rt.data);
            }
        }
        engine::DataStore::wipe_extensions(&own_store);
        crate::zaps::forget(&own_store);
        self.forget_favicons((false, id));
        self.core.wiped_at.borrow_mut().remove(&(false, id));
        self.pages.retain(|k, _| k.private || self.core.disk.tab(k.id).is_ok());
        if !still_used {
            engine::DataStore::wipe_persistent(&profile.data_store_uuid);
            self.retire_store(&profile.data_store_uuid, cx);
        }
        self.reload_spaces();
        self.model.closed_tabs.retain(&self.model.spaces);
        let ix = self.model.spaces.iter().position(|s| Some((s.private, s.space.id)) == current).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }
}

/// The stored list `stored` (one entry per line) with `entry` added or removed.
fn toggled(stored: &str, entry: &str, on: bool) -> String {
    let mut entries: Vec<&str> = stored.lines().filter(|p| !p.is_empty() && *p != entry).collect();
    if on {
        entries.push(entry);
    }
    entries.join("\n")
}

pub(crate) fn caption(text: &'static str) -> Div {
    div().text_size(px(TEXT_SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::muted()).child(text)
}

fn text_block(title: impl Into<SharedString>, detail: &'static str) -> Div {
    div().flex_1().min_w_0().flex().flex_col().gap_0p5().child(title.into()).child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(detail))
}

pub(crate) fn focus_ring(d: Div, on: bool) -> Div {
    d.p(px(2.)).rounded(px(Theme::radius_row() + 2.)).border_2().border_color(if on { Theme::accent_soft().into() } else { transparent_black() })
}

pub(crate) fn choice_row(id: impl Into<gpui::ElementId>, selected: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .when(selected, |d| d.bg(Theme::raised()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(radio(selected))
}

pub(crate) fn color_row(
    id: &'static str,
    selected: Option<usize>,
    none: bool,
    ring: bool,
    cx: &mut Context<Browser>,
    pick: fn(&mut ProfileEditor, Option<usize>),
) -> Div {
    let none = none.then(|| {
        let glyph = if selected.is_none() { Icon::Check } else { Icon::Close };
        swatch((id, PROFILE_COLORS.len()), transparent_black().into(), selected.is_none(), 26.)
            .child(icon(glyph, 14., Theme::muted()))
            .on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| pick(e, None))))
    });
    let colors = PROFILE_COLORS.iter().enumerate().map(|(ix, (_, c))| {
        let on = selected == Some(ix);
        swatch((id, ix), rgb(*c), on, 26.)
            .when(on, |d| d.child(icon(Icon::Check, 14., Theme::text())))
            .on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| pick(e, Some(ix)))))
    });
    div().flex().ml(px(-4.)).child(focus_ring(div().flex(), ring).child(div().flex().gap_2p5().children(none).children(colors)))
}

fn details(b: &Browser, editor: &ProfileEditor, window: &Window, cx: &mut Context<Browser>) -> Div {
    let focused = editor.name.focus_handle(cx).is_focused(window);
    let ring = |field: usize| editor.field == field && editor.focus.is_focused(window);
    let candidates = if editor.editing.is_none() { b.other_profiles(None) } else { Vec::new() };
    let share = editor.share;
    let first = candidates.first().map(|p| p.id);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(caption("Name"))
        .child(field(focused).child(editor.name.clone()))
        .child(div().pt_2().child(caption("Color")))
        .child(color_row("profile-color", Some(editor.color), false, ring(1), cx, |e, ix| e.color = ix.unwrap_or(e.color)))
        .when(!candidates.is_empty(), |d| {
            d.child(
                div()
                    .id("profile-share")
                    .mt_3()
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(Theme::radius_group()))
                    .bg(Theme::raised())
                    .border_1()
                    .border_color(if ring(2) { Theme::accent() } else { Theme::hairline() })
                    .cursor_pointer()
                    .child(text_block(
                        "Share data with another profile",
                        "Both profiles use the same logins, cookies and site data, so sites can link them. Deleting data or choosing Lockdown in one clears it for both.",
                    ))
                    .child(switch(share.is_some()))
                    .on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| e.share = if share.is_some() { None } else { first }))),
            )
            .when(share.is_some(), |d| {
                d.child(focus_ring(div().flex().flex_col().mx(px(-4.)), ring(3)).children(candidates.into_iter().map(|p| {
                    let id = p.id;
                    choice_row(("share-from", id as usize), share == Some(id))
                        .child(profile_dot(&p.color, 10.))
                        .child(ui::text::bounded_title(&p.name))
                        .on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| e.share = Some(id))))
                })))
            })
        })
}

fn protection(editor: &ProfileEditor, window: &Window, cx: &mut Context<Browser>) -> Div {
    let engine = editor.engine;
    let ring = |field: usize| editor.field == field && editor.focus.is_focused(window);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(caption("Shield"))
        .child(focus_ring(div().flex().flex_col().gap_1().mx(px(-4.)), ring(0)).children(ShieldLevel::ALL.into_iter().map(|level| {
            choice_row(level.as_str(), level == editor.level)
                .child(text_block(shield_badge(level), level_description(level)))
                .on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| e.level = level)))
        })))
        .child(div().pt_3().pb_1().child(caption("Search engine")))
        .child(div().flex().ml(px(-4.)).child(focus_ring(div().flex(), ring(1)).child(segmented().children(ENGINES.iter().enumerate().map(|(ix, e)| {
            segment(("profile-engine", ix), e.name, ix == engine).on_click(cx.listener(move |b, _, _, cx| b.update_editor(cx, |e| e.engine = ix)))
        })))))
}

/// Title, subtitle, body, the footer's leading controls, the confirm label and whether it is enabled.
pub(crate) type Parts = (&'static str, &'static str, Div, Div, &'static str, bool);

fn profile_parts(b: &Browser, editor: &ProfileEditor, window: &Window, cx: &mut Context<Browser>) -> Parts {
    let ready = b.editor_name(cx).is_some();
    let creating = editor.editing.is_none();
    let step = editor.protection;
    let (title, subtitle) = match (creating, step) {
        (_, true) => ("Protection and Search", "How strictly this profile blocks trackers, and where it searches. Both can be changed later in Settings."),
        (true, false) => ("Create Profile", "Each profile keeps its own logins, cookies, site data, history, bookmarks and tabs."),
        (false, false) => ("Edit Profile", "Rename the profile or give it another colour."),
    };
    let confirm = match (creating, step) {
        (false, _) => "Save",
        (true, false) => "Continue",
        (true, true) => "Create Profile",
    };
    let body = if step { protection(editor, window, cx) } else { details(b, editor, window, cx) };
    let leading = div()
        .flex_1()
        .flex()
        .items_center()
        .gap_2()
        .when(creating, |d| d.child(key_hint(if step { "Step 2 of 2" } else { "Step 1 of 2" })))
        .child(div().flex_1())
        .when(step, |d| d.child(button("profile-back", "Back", false).on_click(cx.listener(|b, _, w, cx| b.profile_step(false, w, cx)))));
    (title, subtitle, body, leading, confirm, ready)
}

pub fn render_editor(b: &Browser, window: &Window, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let editor = b.profile_editor.as_ref()?;
    let (title, subtitle, body, leading, confirm, ready) = match &editor.space {
        Some(card) => crate::spaces::parts(b, editor, card, window, cx),
        None => profile_parts(b, editor, window, cx),
    };
    let step = editor.protection;
    let footer = div()
        .pt_2()
        .flex()
        .items_center()
        .gap_2()
        .child(leading)
        .child(button("profile-cancel", "Cancel", false).child(key_hint("ESC")).on_click(cx.listener(|b, _, w, cx| b.dismiss_overlay(w, cx))))
        .child(
            button("profile-confirm", confirm, ready)
                .child(key_hint("↩"))
                .when(!ready, |d| d.text_color(Theme::faint()).cursor_default())
                .when(ready, |d| d.on_click(cx.listener(|b, _, w, cx| b.confirm_profile(w, cx)))),
        );
    let card = surface(Theme::radius_card())
        .id("profile-editor")
        .key_context("ProfileEditor")
        .track_focus(&editor.focus)
        .on_action(cx.listener(|b, _: &ConfirmProfile, w, cx| b.confirm_profile(w, cx)))
        .on_action(cx.listener(|b, _: &NextField, w, cx| b.move_field(1, w, cx)))
        .on_action(cx.listener(|b, _: &PreviousField, w, cx| b.move_field(-1, w, cx)))
        .on_action(cx.listener(|b, _: &NextOption, _, cx| b.move_option(1, cx)))
        .on_action(cx.listener(|b, _: &PreviousOption, _, cx| b.move_option(-1, cx)))
        .w(px(540.))
        .max_w_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .shadow(Theme::elevation(18., 48.))
        .text_size(px(TEXT))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(px(TEXT_TITLE + 3.)).font_weight(FontWeight::BOLD).child(title))
                .child(div().text_color(Theme::muted()).child(subtitle)),
        )
        .child(body)
        .child(footer);
    Some(
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .pt(relative(if editor.space.is_some() { 0.04 } else { 0.12 }))
            .px_4()
            .child(pop_in(card, ("profile-editor", step as usize), b.pref(Pref::ReduceMotion)))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_cleaned_and_unique() {
        assert_eq!(valid_name("  Work  ", []), Some("Work".into()));
        assert_eq!(valid_name("   ", []), None);
        assert_eq!(valid_name("Wo\u{202e}rk\n", []), Some("Work".into()));
        assert_eq!(valid_name("work", ["Personal", " Work"]), None);
        assert_eq!(valid_name(&"x".repeat(200), []).map(|n| n.chars().count()), Some(MAX_NAME));
    }
}
