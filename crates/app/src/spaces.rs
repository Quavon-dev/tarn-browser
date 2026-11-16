use gpui::{
    App, AppContext, Context, Div, Entity, Focusable, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, actions,
    div, prelude::FluentBuilder, px, rgb,
};
use store::{Id, Schedule, Space};
use ui::controls::{button, caption, field};
use ui::icon::{SPACE_ICONS, icon};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{PROFILE_COLORS, TEXT_SMALL, Theme};

use crate::browser::{Browser, SpaceRef};

use crate::profiles::{Parts, ProfileEditor, choice_row, color_row, focus_ring, profile_dot, valid_name};

actions!(spaces, [EditSpace]);

/// Weekday bits of `Schedule::days`, Monday first as the card lists them.
const DAYS: [(&str, u8); 7] = [("Mon", 1), ("Tue", 2), ("Wed", 3), ("Thu", 4), ("Fri", 5), ("Sat", 6), ("Sun", 0)];
/// What the arrows step the days through: none, weekdays, the weekend, every day.
const DAY_PRESETS: [u8; 4] = [0, 0b011_1110, 0b100_0001, 0b111_1111];

/// The Edit Space and New Space card; it rides on the profile editor, whose name and colour
/// hold the new profile when "New Profile…" is chosen.
pub struct SpaceCard {
    editing: Option<Id>,
    private: bool,
    profile: Option<Id>,
    name: Entity<TextInput>,
    color: Option<usize>,
    icon: usize,
    schedule: Schedule,
    _subscription: Subscription,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Profile,
    ProfileName,
    ProfileColor,
    Name,
    Color,
    Icon,
    Days,
    Start,
    End,
}

/// The cleaned space name, unless another space of the same profile already has it.
pub fn space_name(raw: &str, spaces: &[SpaceRef], private: bool, profile: Option<Id>, editing: Option<Id>) -> Option<String> {
    let taken = spaces.iter().filter(|s| s.private == private && Some(s.space.profile_id) == profile && Some(s.space.id) != editing);
    valid_name(raw, taken.map(|s| s.space.name.as_str()))
}

impl Browser {
    pub(crate) fn scheduled_now(&self) -> Option<usize> {
        let (weekday, hour) = self.core.disk.local_time().ok()?;
        browser_core::schedule::scheduled(&self.model.spaces, weekday, hour)
    }

    /// Switches to the space scheduled for now, unless the private space, a card or the address
    /// field is open, or the user changed space within the last hour.
    pub(crate) fn follow_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.private() || self.overlay_open() || self.command_mode.is_some() {
            return;
        }
        let Ok((weekday, hour)) = self.core.disk.local_time() else { return };
        let due = browser_core::schedule::due(&self.model.spaces, self.model.space_ix, weekday, hour, self.space_switched_at, crate::browser::now());
        if let Some(ix) = due {
            self.switch_space(ix, window, cx);
            self.space_switched_at = None;
        }
    }

    pub fn new_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_space_card(None, window, cx);
    }

    pub fn open_space_card(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_space_card(ix, window, cx)) {
            return;
        }
        let editing: Option<(bool, Space)> = match ix {
            Some(ix) => match self.model.spaces.get(ix) {
                Some(s) => Some((s.private, s.space.clone())),
                None => return,
            },
            None => None,
        };
        let current = self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id);
        let first = || self.core.disk.profiles().ok()?.first().map(|p| p.id);
        let profile = editing.as_ref().map(|(_, s)| s.profile_id).or(current).or_else(first);
        let private = editing.as_ref().is_some_and(|(p, _)| *p);
        let text = match &editing {
            Some((_, s)) => s.name.clone(),
            None => {
                let count = self.model.spaces.iter().filter(|s| !s.private && Some(s.space.profile_id) == profile).count();
                let mut names = (count + 1..).map(|n| format!("Space {n}"));
                names.find(|n| space_name(n, &self.model.spaces, false, profile, None).is_some()).unwrap_or_default()
            }
        };
        let color = editing.as_ref().and_then(|(_, s)| PROFILE_COLORS.iter().position(|(n, _)| *n == s.color));
        let icon = editing.as_ref().and_then(|(_, s)| SPACE_ICONS.iter().position(|(n, _)| *n == s.icon)).unwrap_or(0);
        let schedule = editing.as_ref().map(|(_, s)| s.schedule).filter(|s| s.days != 0).unwrap_or(Schedule { days: 0, start: 9, end: 17 });
        self.open_profile_editor(None, window, cx);
        let name = cx.new(|cx| {
            let mut input = TextInput::new("Space Name", cx);
            input.set_text(text, cx);
            input
        });
        let subscription = cx.subscribe_in(&name, window, |b, _, event: &TextInputEvent, window, cx| match event {
            TextInputEvent::Submit => b.confirm_space(window, cx),
            TextInputEvent::Cancel => b.dismiss_overlay(window, cx),
            _ => cx.notify(),
        });
        let Some(editor) = &mut self.profile_editor else { return };
        let editing = editing.map(|(_, s)| s.id);
        editor.space = Some(SpaceCard { editing, private, profile, name: name.clone(), color, icon, schedule, _subscription: subscription });
        editor.field = usize::from(editing.is_none());
        window.focus(&name.focus_handle(cx));
        cx.notify();
    }

    pub(crate) fn space_card(&self) -> Option<&SpaceCard> {
        self.profile_editor.as_ref()?.space.as_ref()
    }

    fn update_card(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut SpaceCard)) {
        self.update_editor(cx, |e| {
            if let Some(card) = &mut e.space {
                f(card);
            }
        });
    }

    fn space_fields(&self) -> Vec<Field> {
        let Some(card) = self.space_card() else { return Vec::new() };
        let mut fields = Vec::new();
        if card.editing.is_none() {
            fields.push(Field::Profile);
            if card.profile.is_none() {
                fields.extend([Field::ProfileName, Field::ProfileColor]);
            }
        }
        fields.push(Field::Name);
        if !card.private {
            fields.push(Field::Color);
        }
        fields.push(Field::Icon);
        if card.editing.is_some() && !card.private {
            fields.extend([Field::Days, Field::Start, Field::End]);
        }
        fields
    }

    fn space_ready(&self, cx: &App) -> Option<(String, Option<String>)> {
        let card = self.space_card()?;
        let name = space_name(card.name.read(cx).text(), &self.model.spaces, card.private, card.profile, card.editing)?;
        let profile_name = match card.profile {
            Some(_) => None,
            None => Some(self.editor_name(cx)?),
        };
        Some((name, profile_name))
    }

    pub(crate) fn move_space_field(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self.space_fields();
        let Some(editor) = &mut self.profile_editor else { return };
        let Some(card) = &editor.space else { return };
        let at = |f: Field| fields.iter().position(|x| *x == f);
        let current = if card.name.focus_handle(cx).is_focused(window) {
            at(Field::Name)
        } else if editor.name.focus_handle(cx).is_focused(window) {
            at(Field::ProfileName)
        } else {
            Some(editor.field)
        };
        let next = (current.unwrap_or(0) as isize + step).rem_euclid(fields.len().max(1) as isize) as usize;
        let focus = match fields.get(next) {
            Some(Field::Name) => card.name.focus_handle(cx),
            Some(Field::ProfileName) => editor.name.focus_handle(cx),
            _ => editor.focus.clone(),
        };
        editor.field = next;
        window.focus(&focus);
        cx.notify();
    }

    pub(crate) fn move_space_option(&mut self, step: isize, cx: &mut Context<Self>) {
        let field = self.space_fields().get(self.profile_editor.as_ref().map_or(0, |e| e.field)).copied();
        let profiles: Vec<Option<Id>> = self.core.disk.profiles().unwrap_or_default().iter().map(|p| Some(p.id)).chain([None]).collect();
        let wrap = |ix: usize, len: usize| (ix as isize + step).rem_euclid(len as isize) as usize;
        self.update_editor(cx, |e| {
            let Some(card) = &mut e.space else { return };
            match field {
                Some(Field::Profile) => {
                    let ix = profiles.iter().position(|p| *p == card.profile).unwrap_or(0);
                    card.profile = profiles[wrap(ix, profiles.len())];
                }
                Some(Field::ProfileColor) => e.color = wrap(e.color, PROFILE_COLORS.len()),
                Some(Field::Color) => card.color = wrap(card.color.map_or(0, |c| c + 1), PROFILE_COLORS.len() + 1).checked_sub(1),
                Some(Field::Icon) => card.icon = wrap(card.icon, SPACE_ICONS.len()),
                Some(Field::Days) => {
                    // A custom set of days steps to the presets next to "none" rather than jumping to it.
                    let ix = wrap(DAY_PRESETS.iter().position(|d| *d == card.schedule.days).unwrap_or(0), DAY_PRESETS.len());
                    card.schedule.days = DAY_PRESETS[ix];
                }
                Some(Field::Start) => card.schedule.start = wrap(card.schedule.start.into(), 24) as u8,
                Some(Field::End) => card.schedule.end = wrap(card.schedule.end.into(), 24) as u8,
                _ => {}
            }
        });
    }

    pub(crate) fn confirm_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, profile_name)) = self.space_ready(cx) else { return };
        let Some(mut editor) = self.profile_editor.take() else { return };
        let Some(card) = editor.space.take() else { return };
        self.close_command(window, cx);
        let color = card.color.map_or("", |ix| PROFILE_COLORS[ix].0);
        let icon = SPACE_ICONS[card.icon].0;
        if let Some(profile_name) = profile_name {
            return self.create_profile(&editor, &profile_name, PROFILE_COLORS[editor.color].0, [&name, color, icon], window, cx);
        }
        let store = self.store(card.private);
        let saved = match (card.editing, card.profile) {
            (Some(id), _) => store
                .update_space(id, &name, color, icon)
                .and_then(|_| if card.private { Ok(()) } else { store.set_space_schedule(id, card.schedule) })
                .map(|_| id),
            (None, Some(profile)) => store.create_space(profile, &name, icon).and_then(|s| store.update_space(s.id, &name, color, icon).map(|_| s.id)),
            (None, None) => return,
        };
        let Ok(id) = saved else { return };
        self.reload_spaces();
        Self::sync_other_windows(cx);
        if card.editing.is_none()
            && let Some(ix) = self.model.spaces.iter().position(|s| s.private == card.private && s.space.id == id)
        {
            self.switch_space(ix, window, cx);
        }
        cx.notify();
    }
}

pub(crate) fn parts(b: &Browser, editor: &ProfileEditor, card: &SpaceCard, window: &Window, cx: &mut Context<Browser>) -> Parts {
    let fields = b.space_fields();
    let ring = |f: Field| fields.get(editor.field) == Some(&f) && editor.focus.is_focused(window);
    let creating = card.editing.is_none();
    let tint = card.color.map_or(Theme::text(), |ix| rgb(PROFILE_COLORS[ix].1));
    let mut body = div().flex().flex_col().gap_2();
    if creating {
        let profiles = b.core.disk.profiles().unwrap_or_default();
        let chips = profiles.iter().map(|p| {
            let id = p.id;
            choice_row(("space-profile", id as usize), card.profile == Some(id))
                .child(profile_dot(&p.color, 10.))
                .child(bounded_title(&p.name))
                .on_click(cx.listener(move |b, _, _, cx| b.update_card(cx, |c| c.profile = Some(id))))
        });
        let new = choice_row("space-new-profile", card.profile.is_none())
            .child("New Profile…")
            .on_click(cx.listener(|b, _, _, cx| b.update_card(cx, |c| c.profile = None)));
        body = body.child(caption("Profile")).child(focus_ring(div().flex().flex_wrap().gap_1().mx(px(-4.)), ring(Field::Profile)).children(chips).child(new));
        if card.profile.is_none() {
            body = body
                .child(
                    div()
                        .text_size(px(TEXT_SMALL))
                        .text_color(Theme::muted())
                        .child("Starts empty, with its own logins, cookies, site data, history and bookmarks."),
                )
                .child(div().pt_1().child(caption("Profile Name")))
                .child(field(editor.name.focus_handle(cx).is_focused(window)).child(editor.name.clone()))
                .child(div().pt_1().child(caption("Profile Color")))
                .child(color_row("new-profile-color", Some(editor.color), false, ring(Field::ProfileColor), cx, |e, ix| e.color = ix.unwrap_or(e.color)));
        }
        body = body.child(div().pt_1().child(caption("Space Name")));
    } else {
        body = body.child(caption("Name"));
    }
    body = body.child(field(card.name.focus_handle(cx).is_focused(window)).child(card.name.clone()));
    if !card.private {
        body = body.child(div().pt_1().child(caption("Color"))).child(color_row("space-color", card.color, true, ring(Field::Color), cx, |e, ix| {
            if let Some(card) = &mut e.space {
                card.color = ix;
            }
        }));
    }
    let icons = SPACE_ICONS.iter().enumerate().map(|(ix, (_, glyph))| {
        let on = ix == card.icon;
        div()
            .id(("space-icon", ix))
            .w(px(32.))
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::radius_row()))
            .cursor_pointer()
            .when(on, |d| d.bg(Theme::selected()))
            .hover(|s| s.bg(Theme::hover()))
            .active(|s| s.bg(Theme::pressed()))
            .child(icon(*glyph, 16., if on { tint } else { Theme::muted() }))
            .on_click(cx.listener(move |b, _, _, cx| b.update_card(cx, |c| c.icon = ix)))
    });
    body = body.child(div().pt_1().child(caption("Icon"))).child(focus_ring(div().flex().flex_wrap().gap_1().mx(px(-4.)), ring(Field::Icon)).children(icons));
    if !creating && !card.private {
        body = body.child(div().pt_1().child(caption("Schedule"))).child(schedule_rows(
            card.schedule,
            [ring(Field::Days), ring(Field::Start), ring(Field::End)],
            cx,
        ));
    }
    let profile = card.profile.filter(|_| !card.private);
    let leading: Div = div().flex_1().flex().children(
        profile.map(|id| button("space-edit-profile", "Edit Profile…", false).on_click(cx.listener(move |b, _, w, cx| b.open_profile_editor(Some(id), w, cx)))),
    );
    let ready = b.space_ready(cx).is_some();
    if creating {
        (
            "New Space",
            "A space keeps its own tabs, pinned tabs and folders; spaces of one profile share its logins, cookies and history.",
            body,
            leading,
            "Create Space",
            ready,
        )
    } else {
        ("Edit Space", "Rename the space, give it another colour and icon, or schedule it.", body, leading, "Save", ready)
    }
}

fn schedule_rows(schedule: Schedule, [days_ring, start_ring, end_ring]: [bool; 3], cx: &mut Context<Browser>) -> Div {
    let days = DAYS.iter().map(|&(name, bit)| {
        let on = schedule.days & (1 << bit) != 0;
        div()
            .id(("space-day", bit as usize))
            .w(px(40.))
            .py_1()
            .flex()
            .justify_center()
            .rounded(px(Theme::radius_row()))
            .cursor_pointer()
            .text_color(if on { Theme::text() } else { Theme::muted() })
            .when(on, |d| d.bg(Theme::selected()))
            .hover(|s| s.bg(Theme::hover()))
            .active(|s| s.bg(Theme::pressed()))
            .child(name)
            .on_click(cx.listener(move |b, _, _, cx| b.update_card(cx, |c| c.schedule.days ^= 1 << bit)))
    });
    let hour = |id: &'static str, value: u8, ring: bool, pick: fn(&mut Schedule, u8), cx: &mut Context<Browser>| {
        let earlier = cx.listener(move |b, _, _, cx| b.update_card(cx, |c| pick(&mut c.schedule, (value + 23) % 24)));
        let later = cx.listener(move |b, _, _, cx| b.update_card(cx, |c| pick(&mut c.schedule, (value + 1) % 24)));
        focus_ring(div().flex().items_center().gap_1(), ring)
            .child(button((id, 0usize), "‹", false).on_click(earlier))
            .child(div().w(px(44.)).flex().justify_center().child(format!("{value:02}:00")))
            .child(button((id, 1usize), "›", false).on_click(later))
    };
    let overnight = schedule.end <= schedule.start;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(focus_ring(div().flex().gap_1().mx(px(-4.)), days_ring).children(days))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child("From")
                .child(hour("space-start", schedule.start, start_ring, |s, h| s.start = h, cx))
                .child("to")
                .child(hour("space-end", schedule.end, end_ring, |s, h| s.end = h, cx))
                .when(overnight, |d| d.child(div().text_color(Theme::muted()).child("next day"))),
        )
        .child(
            div()
                .text_size(px(TEXT_SMALL))
                .text_color(Theme::muted())
                .child("On these days and hours each window switches to this space, unless you changed space in the last hour. No days, no schedule."),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: Id, profile_id: Id, name: &str, private: bool) -> SpaceRef {
        SpaceRef {
            private,
            space: Space { id, profile_id, name: name.into(), icon: String::new(), position: 0, color: String::new(), schedule: Schedule::default() },
        }
    }

    #[test]
    fn space_names_are_unique_within_their_profile_only() {
        let spaces = [space(1, 1, "Work", false), space(2, 2, "Home", false), space(1, 9, "Private", true)];
        assert_eq!(space_name(" work ", &spaces, false, Some(1), None), None);
        assert_eq!(space_name("Work", &spaces, false, Some(1), Some(1)), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, Some(2), None), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, None, None), Some("Work".into()));
        assert_eq!(space_name("private", &spaces, false, Some(9), None), Some("private".into()));
        assert_eq!(space_name("private", &spaces, true, Some(9), None), None);
        assert_eq!(space_name("  ", &spaces, false, Some(1), None), None);
    }
}
