use browser_core::command::ENGINES;
use browser_core::profiles::NewProfile;
use browser_core::space_card::name_too_long;
use nsh_core::prelude::*;
use shield::ShieldLevel;
use store::Id;

use super::panels::{heading, levels, menu_row, popover, section};
use super::tools::{line, note};
use super::{Panel, Ui, act, confirmed, read, show};
use crate::browser::profiles::SpaceKey;
use crate::controls::{accent_pill, body, group, label, pill, segmented, small, switch, title};
use crate::icons;
use crate::theme::Theme;
use tarn_nsh_shared::theme::PROFILE_COLORS;

const DEFAULT_COLOR: usize = 2;
const SWATCH: f32 = 24.;

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    color: usize,
    level: ShieldLevel,
    engine: usize,
    share: Option<Id>,
    error: Option<String>,
}

impl Default for Form {
    fn default() -> Form {
        Form { color: DEFAULT_COLOR, level: ShieldLevel::Standard, engine: 0, share: None, error: None }
    }
}

fn color_index(name: &str) -> Option<usize> {
    PROFILE_COLORS.iter().position(|(n, _)| *n == name)
}

fn open(ui: Ui, editing: Option<Id>) {
    ui.panel.set(Panel::None);
    show(ui, Panel::Card(editing));
}

pub fn open_profile_card(ui: Ui, editing: Option<Id>) {
    let profile = editing.and_then(|id| read(ui, |b| b.core.disk.profile(id).ok()).flatten());
    if editing.is_some() && profile.is_none() {
        return;
    }
    ui.form.set(Form { color: profile.as_ref().and_then(|p| color_index(&p.color)).unwrap_or(DEFAULT_COLOR), ..Form::default() });
    ui.card_name.set(profile.map(|p| p.name).unwrap_or_default());
    ui.card_space.set("Home".to_owned());
    open(ui, editing);
}

fn update(ui: Ui, f: impl FnOnce(&mut Form)) {
    ui.form.update(|form| {
        f(form);
        form.error = None;
    });
    act(ui, |_| {});
}

fn swatches(ui: Ui, selected: Option<usize>, none: bool, pick: fn(&mut Form, Option<usize>)) -> View {
    let swatch = move |ix: Option<usize>| {
        let (name, fill) = match ix {
            Some(ix) => (PROFILE_COLORS[ix].0, Theme::profile_color(PROFILE_COLORS[ix].0).unwrap_or(Theme::faint())),
            None => ("none", Theme::control()),
        };
        let on = selected == ix;
        crate::controls::pressable(name.to_owned(), move || update(ui, |f| pick(f, ix)))
            .w(px(SWATCH))
            .h(px(SWATCH))
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(fill)
            .border_1()
            .border_color(if on { Theme::text() } else { Theme::hairline() })
            .selected(on)
            .text_color(Theme::on_fill(fill))
            .children(on.then(|| icons::icon(icons::CHECK, 13.)))
            .into_element()
    };
    let all: Vec<AnyElement> = none.then_some(None).into_iter().chain((0..PROFILE_COLORS.len()).map(Some)).map(swatch).collect();
    view().flex_row().flex_wrap().gap_2().children(all)
}

/// Says so while the typed profile name is too long or another profile's (`space_card::profile_name_note`).
pub(super) fn name_note(typed: impl Fn() -> String + 'static, names: Vec<String>) -> AnyElement {
    dynamic(move || browser_core::space_card::profile_name_note(&typed(), names.iter().map(String::as_str)).map(|text| note(text).text_color(Theme::warning())))
}

fn field(ui: Ui, name: &str, value: Signal<String>) -> View {
    view().flex_col().gap_1().child(small(name.to_owned())).child(super::text_field(value, name, 320.).on_submit(move |_: &str| confirm(ui)))
}

fn profile_card(ui: Ui, editing: Option<Id>) -> View {
    let form = ui.form.get_untracked();
    let others = read(ui, |b| b.saved_profiles()).unwrap_or_default();
    let (title_text, subtitle) = match editing {
        Some(_) => ("Edit Profile", "Rename the profile or give it another colour."),
        None => ("Create Profile", "Each profile keeps its own logins, cookies, site data, history, bookmarks and tabs."),
    };
    let mut body = view()
        .flex_col()
        .gap_3()
        .child(note(subtitle))
        .child(field(ui, "Name", ui.card_name))
        .child(name_note(move || ui.card_name.get(), others.iter().filter(|p| Some(p.id) != editing).map(|p| p.name.clone()).collect()))
        .child(view().flex_col().gap_1().child(small("Color")).child(swatches(ui, Some(form.color), false, |f, ix| f.color = ix.unwrap_or(f.color))));
    if editing.is_none() {
        let engines: Vec<(usize, String)> = ENGINES.iter().enumerate().map(|(ix, e)| (ix, e.name.to_owned())).collect();
        body = body
            .child(field(ui, "First Space", ui.card_space))
            .child(section("Protection and Search"))
            .child(segmented(levels(), form.level, move |level| update(ui, move |f| f.level = level)))
            .child(segmented(engines, form.engine, move |ix| update(ui, move |f| f.engine = ix)))
            .child(note("How strictly this profile blocks trackers, and where it searches. Both can be changed later in Settings."));
        if !others.is_empty() {
            let first = others.first().map(|p| p.id);
            let shared = form.share;
            body = body.child(group().child(switch(shared.is_some(), "Share data with another profile", move || {
                update(ui, move |f| f.share = if shared.is_some() { None } else { first })
            })));
            body = body.child(note("Both profiles use the same logins, cookies and site data, so sites can link them. Deleting data in either, or choosing Lockdown in the one whose data is shared, clears it for both."));
            if shared.is_some() {
                let rows: Vec<AnyElement> =
                    others.iter().map(|p| profile_choice(ui, p, shared == Some(p.id), |f, id| f.share = Some(id)).into_element()).collect();
                body = body.child(group().children(rows));
            }
        }
    }
    let confirm_name = if editing.is_some() { "Save" } else { "Create Profile" };
    popover()
        .child(heading(ui, title(title_text)))
        .child(body)
        .children(form.error.map(|e| note(e).text_color(Theme::warning())))
        .child(footer(ui, confirm_name))
}

fn profile_choice(ui: Ui, p: &store::Profile, on: bool, pick: fn(&mut Form, Id)) -> nsh_base::ButtonElement {
    let id = p.id;
    crate::controls::row(p.name.clone(), move || update(ui, move |f| pick(f, id)))
        .bg(if on { Theme::selected() } else { Theme::clear() })
        .selected(on)
        .child(dot(&p.color))
        .child(label(engine::view_policy::title(&p.name)).flex_1().truncate())
        .children(on.then(|| icons::icon(icons::CHECK, 13.)))
}

pub(super) fn dot(color: &str) -> View {
    view().w(px(10.)).h(px(10.)).flex_none().rounded_full().bg(Theme::profile_color(color).unwrap_or(Theme::faint()))
}

fn footer(ui: Ui, confirm_name: &'static str) -> View {
    view().flex_row().items_center().gap_2().pt_2().child(view().flex_1()).child(pill("Cancel", move || show(ui, Panel::None))).child(
        accent_pill(confirm_name, move || confirm(ui)).disabled(move || name_too_long(&ui.card_name.get())).disabled_style(|s| s.text_color(Theme::faint())),
    )
}

fn confirm(ui: Ui) {
    let Panel::Card(editing) = ui.panel.get_untracked() else { return };
    let form = ui.form.get_untracked();
    let (name, space) = (ui.card_name.get_untracked(), ui.card_space.get_untracked());
    if name_too_long(&name) {
        return;
    }
    let mut result: Result<(), String> = Ok(());
    act(ui, |b| {
        result = match editing {
            Some(id) if b.edit_profile(id, &name, PROFILE_COLORS[form.color].0) => Ok(()),
            Some(_) => Err("Choose a name no other profile has.".to_owned()),
            None => b.create_profile(&new_profile(&form, &name, [space.as_str(), "", ""])),
        };
    });
    match result {
        Ok(()) => {
            show(ui, Panel::None);
            super::sync_others(ui);
        }
        Err(error) => {
            ui.form.update(|f| f.error = Some(error));
            act(ui, |_| {});
        }
    }
}

fn new_profile<'a>(form: &Form, name: &'a str, space: [&'a str; 3]) -> NewProfile<'a> {
    NewProfile { name, color: PROFILE_COLORS[form.color].0, level: form.level, engine: ENGINES[form.engine].name, share: form.share, space }
}

pub fn card_panel(ui: Ui, editing: Option<Id>) -> AnyElement {
    profile_card(ui, editing).into_element()
}

pub fn space_menu(ui: Ui, key: SpaceKey) -> AnyElement {
    let Some((name, current, deletable)) = read(ui, |b| {
        let ix = b.model.spaces.iter().position(|s| (s.private, s.space.id) == key)?;
        let s = &b.model.spaces[ix];
        let name = if s.private { "Private".to_owned() } else { engine::view_policy::title(&s.space.name) };
        Some((name, ix == b.model.space_ix, b.model.can_delete_space(ix)))
    })
    .flatten() else {
        return view().into_element();
    };
    let rename = current.then(|| menu_row("Rename", None, move || show(ui, Panel::Space)));
    popover()
        .child(heading(ui, title(format!("Space “{name}”")).truncate()))
        .children(rename.map(|r| group().child(r)))
        .child(space_actions_with(ui, key, deletable))
        .into_element()
}

fn space_actions_with(ui: Ui, key: SpaceKey, deletable: bool) -> View {
    let (hidden, hideable) = read(ui, |b| {
        let ix = b.model.spaces.iter().position(|s| (s.private, s.space.id) == key);
        (ix.is_some_and(|ix| b.model.spaces[ix].space.hidden), ix.is_some_and(|ix| b.model.can_hide_space(ix)))
    })
    .unwrap_or_default();
    let hide = (hidden || hideable).then(|| {
        menu_row(if hidden { "Show Space" } else { "Hide Space" }, None, move || {
            ui.panel.set(Panel::None);
            act(ui, |b| b.set_space_hidden(key, !hidden));
            super::sync_others(ui);
        })
    });
    let rows = group()
        .child(menu_row("Edit Space…", None, move || edit_space(ui, key)))
        .child(menu_row("New Space…", super::shortcut_of(ui, crate::shortcuts::Action::NewSpace), move || show(ui, Panel::NewSpace)))
        .children(hide);
    view().flex_col().gap_2().child(rows).children(deletable.then(|| delete_row(ui, key)))
}

fn edit_space(ui: Ui, key: SpaceKey) {
    ui.panel.set(Panel::None);
    tarn_nsh_shared::motion::space_switch();
    act(ui, |b| {
        if let Some(ix) = b.model.spaces.iter().position(|s| (s.private, s.space.id) == key) {
            b.switch_space_by_user(ix);
        }
    });
    show(ui, Panel::Space);
}

pub fn space_delete(ui: Ui, key: SpaceKey) -> Option<View> {
    let deletable =
        read(ui, |b| b.model.spaces.iter().position(|s| (s.private, s.space.id) == key).is_some_and(|ix| b.model.can_delete_space(ix))).unwrap_or(false);
    deletable.then(|| delete_row(ui, key))
}

fn delete_row(ui: Ui, key: SpaceKey) -> View {
    // The last space of a profile takes the profile with it.
    let profile = read(ui, |b| b.model.spaces.iter().position(|s| (s.private, s.space.id) == key).and_then(|ix| b.model.profile_deleted_with(ix))).flatten();
    let delete = line("Delete Space…").child(confirmed(ui, format!("space {key:?}"), "Delete", move || {
        ui.panel.set(Panel::None);
        act(ui, |b| b.delete_space_or_profile(key, profile));
        super::sync_others(ui);
    }));
    let detail = if profile.is_some() {
        "The last space of its profile: deletes the profile too, with its spaces, tabs and history, and its cookies and website data unless another profile shares them."
    } else {
        "Deletes the space and all its tabs."
    };
    view().flex_col().gap_2().child(delete).child(note(detail))
}

pub fn switcher(ui: Ui) -> View {
    let (profiles, current) = read(ui, |b| (b.saved_profiles(), b.current_profile())).unwrap_or_default();
    let rows: Vec<AnyElement> = profiles
        .into_iter()
        .map(|p| {
            let (id, on) = (p.id, current == Some(p.id));
            crate::controls::row(p.name.clone(), move || {
                ui.panel.set(Panel::None);
                act(ui, |b| b.switch_profile(id));
            })
            .selected(on)
            .child(dot(&p.color))
            .child(label(engine::view_policy::title(&p.name)).flex_1().truncate())
            .children(on.then(|| icons::icon(icons::CHECK, 13.)))
            .into_element()
        })
        .collect();
    view().flex_col().child(section("Profiles")).child(
        group().children(rows).child(menu_row("New Profile…", None, move || open_profile_card(ui, None))).child(menu_row(
            "Manage Profiles…",
            None,
            move || super::open_settings(ui, tarn_nsh_shared::panes::Pane::Profiles),
        )),
    )
}

pub fn profiles_pane(ui: Ui) -> View {
    let profiles = read(ui, |b| b.saved_profiles()).unwrap_or_default();
    let deletable = profiles.iter().filter(|p| !p.is_private).count() > 1;
    let rows: Vec<AnyElement> = profiles
        .into_iter()
        .map(|p| {
            let id = p.id;
            let mut row = view()
                .flex_row()
                .items_center()
                .gap_2()
                .min_h(px(32.))
                .child(dot(&p.color))
                .child(body(engine::view_policy::title(&p.name)).flex_1().truncate());
            if p.is_private {
                row = row.child(small("Private, not saved"));
            }
            if p.is_dev {
                row = row.child(small("Developer"));
            }
            row = row.child(small(super::panels::capitalised(p.shield_level.as_str())));
            if !p.is_private {
                row = row.child(pill("Edit…", move || open_profile_card(ui, Some(id))));
            }
            if !p.is_private && deletable {
                row = row.child(confirmed(ui, format!("profile {id}"), "Delete", move || {
                    let mut deleted = false;
                    act(ui, |b| deleted = b.delete_profile(id));
                    if deleted {
                        super::forget_chats();
                        browser_core::ext_package::remove_tree(&crate::data::root().join("extensions").join(id.to_string())).ok();
                        super::sync_others(ui);
                    }
                }));
            }
            row.into_element()
        })
        .collect();
    view()
        .flex_col()
        .child(view().flex_row().items_center().pt_3().child(section("Profiles").flex_1()).child(pill("New Profile…", move || open_profile_card(ui, None))))
        .child(view().flex_col().children(rows))
        .child(note("Delete removes a profile with its spaces, tabs, history and website data. Website data another profile shares stays for that profile."))
}
