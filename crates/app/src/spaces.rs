use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Subscription, Task, WeakEntity, Window, actions, canvas, div, point, prelude::FluentBuilder, px, rgb, size,
};
use store::{Id, Profile, Schedule};
use ui::controls::{button, caption, popup_options};
use ui::icon::{Icon, SPACE_ICONS, icon};
use ui::motion::{FADE, POP, REOPEN_GUARD, animate};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{PROFILE_COLORS, TEXT_SMALL, TOOLBAR_HEIGHT, Theme};

use browser_core::space_card::SpaceDraft;
pub use browser_core::space_card::space_name;

use crate::browser::Browser;
use crate::library::SPACE_ROW;
use crate::prefs::Pref;
use crate::profiles::{Parts, ProfileEditor, choice_row, color_row, focus_ring, input, profile_dot};
use crate::sidebar::SIDEBAR_MARGIN;

actions!(spaces, [EditSpace]);

/// Weekday bits of `Schedule::days`, Monday first as the card lists them.
const DAYS: [(&str, u8); 7] = [("Mon", 1), ("Tue", 2), ("Wed", 3), ("Thu", 4), ("Fri", 5), ("Sat", 6), ("Sun", 0)];
/// What the arrows step the days through: none, weekdays, the weekend, every day.
const DAY_PRESETS: [u8; 4] = [0, 0b011_1110, 0b100_0001, 0b111_1111];
const POPOVER_WIDTH: f32 = 400.;
const POPOVER_GUESS: f32 = 480.;

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

impl Browser {
    pub(crate) fn scheduled_now(&self) -> Option<usize> {
        let (weekday, hour) = self.core.disk.local_time().ok()?;
        browser_core::schedule::scheduled(&self.model.spaces, weekday, hour)
    }

    /// Switches to the space scheduled for now, unless the private space, a card or the address
    /// field is open, or the user changed space within the last hour.
    pub(crate) fn follow_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.private() || self.overlay_open() || self.profile_editor.is_some() || self.command_mode.is_some() {
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
        if self.in_main(window, cx, |b, window, cx| b.new_space(window, cx)) {
            return;
        }
        if self.space_closed.is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
            return;
        }
        if self.space_popover.is_some() && self.space_card().is_some_and(|c| c.editing.is_none()) {
            return self.close_space_card(Some(window), cx);
        }
        self.open_space_card(None, window, cx);
    }

    pub fn open_space_card(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_space_card(ix, window, cx)) {
            return;
        }
        let draft = match ix {
            Some(ix) => match self.model.spaces.get(ix) {
                Some(s) => SpaceDraft::edit(s),
                None => return,
            },
            None => SpaceDraft::new(&self.model.spaces, self.model.current_space(), &self.core.disk.profiles().unwrap_or_default()),
        };
        let SpaceDraft { editing, private, profile, schedule, .. } = draft;
        let color = PROFILE_COLORS.iter().position(|(n, _)| *n == draft.color);
        let icon = SPACE_ICONS.iter().position(|(n, _)| *n == draft.icon).unwrap_or(0);
        self.start_editor(None, true, window, cx);
        let name = cx.new(|cx| {
            let mut input = TextInput::new("Space Name", cx);
            input.set_text(draft.name, cx);
            input
        });
        let subscription = cx.subscribe_in(&name, window, |b, _, event: &TextInputEvent, window, cx| match event {
            TextInputEvent::Submit => b.confirm_space(window, cx),
            TextInputEvent::Cancel => b.dismiss_overlay(window, cx),
            TextInputEvent::MenuClosed => b.space_menu_closed(cx),
            _ => cx.notify(),
        });
        let Some(editor) = &mut self.profile_editor else { return };
        editor.space = Some(SpaceCard { editing, private, profile, name: name.clone(), color, icon, schedule, _subscription: subscription });
        editor.field = usize::from(editing.is_none());
        self.show_space_popover(window, cx);
        cx.notify();
    }

    fn show_space_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.space_card().map(|card| card.name.focus_handle(cx)) else { return };
        if let Some((popover, _)) = self.space_popover {
            cx.defer(move |cx| {
                drop(popover.update(cx, |p, window, cx| {
                    p.leave = None;
                    window.activate_window();
                    window.focus(&name);
                    cx.notify();
                }))
            });
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let anchor = window.bounds();
        let x = SIDEBAR_MARGIN + 4.;
        let bottom = f32::from(anchor.size.height) - SIDEBAR_MARGIN - SPACE_ROW;
        let width = POPOVER_WIDTH.min(f32::from(anchor.size.width) - x - SIDEBAR_MARGIN);
        let height = POPOVER_GUESS.min(bottom - TOOLBAR_HEIGHT);
        let bounds = Bounds::new(point(anchor.origin.x + px(x), anchor.origin.y + px(bottom - height)), size(px(width), px(height)));
        let display = window.display(cx).map(|d| d.id());
        let host = self.host.clone();
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            let handle = cx.open_window(popup_options(bounds, display, true), |window, cx| {
                let popup = engine::Host::from_window(window);
                if let Some(popup) = &popup {
                    popup.hide_on_deactivate();
                    popup.round_corners(0., 0., 0., 0., f64::from(Theme::radius_panel()));
                }
                window.focus(&name);
                let place = Place { hosts: host.zip(popup), x, bottom, width, height: Rc::new(Cell::new(height)) };
                cx.new(|cx| SpacePopover::new(browser.clone(), place, window, cx))
            });
            let Ok(handle) = handle else { return };
            let kept = browser.update(cx, |b, cx| {
                let open = b.profile_editor.as_ref().is_some_and(|e| e.floating);
                if open {
                    b.space_popover = Some((handle, main));
                    cx.notify();
                }
                open
            });
            if !kept.unwrap_or(false) {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    pub(crate) fn in_space_popover(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> bool {
        let Some((_, main)) = self.space_popover.filter(|(p, _)| p.window_id() == window.window_handle().window_id()) else { return false };
        cx.defer(move |cx| drop(main.update(cx, f)));
        true
    }

    pub(crate) fn space_menu_closed(&mut self, cx: &mut Context<Self>) {
        if let Some((popover, _)) = self.space_popover {
            cx.defer(move |cx| {
                drop(popover.update(cx, |p, window, cx| {
                    if !window.is_window_active() {
                        p.leave(window, cx);
                    }
                }))
            });
        }
        cx.notify();
    }

    pub(crate) fn close_space_card(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.space_closed = Some(Instant::now());
        let floating = self.profile_editor.as_ref().is_some_and(|e| e.floating);
        match self.space_popover {
            Some((popover, _)) if floating => cx.defer(move |cx| drop(popover.update(cx, |p, window, cx| p.leave(window, cx)))),
            _ if floating => self.profile_editor = None,
            _ => {}
        }
        if let Some(window) = window {
            window.activate_window();
            window.focus(&self.focus_handle(cx));
            if let Some(view) = self.active_view().filter(|_| !self.overlay_open()) {
                view.focus();
            }
        }
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

    fn card_draft(&self, cx: &App) -> Option<SpaceDraft> {
        let editor = self.profile_editor.as_ref()?;
        let card = editor.space.as_ref()?;
        Some(SpaceDraft {
            editing: card.editing,
            private: card.private,
            profile: card.profile,
            name: card.name.read(cx).text().to_owned(),
            color: card.color.map_or("", |ix| PROFILE_COLORS[ix].0).into(),
            icon: SPACE_ICONS[card.icon].0.into(),
            schedule: card.schedule,
            profile_name: editor.name.read(cx).text().to_owned(),
            profile_color: PROFILE_COLORS[editor.color].0.into(),
        })
    }

    fn space_ready(&self, profiles: &[Profile], cx: &App) -> Option<(SpaceDraft, String, Option<String>)> {
        let draft = self.card_draft(cx)?;
        let (name, profile_name) = draft.ready(&self.model.spaces, profiles)?;
        Some((draft, name, profile_name))
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
        let profiles = self.core.disk.profiles().unwrap_or_default();
        let Some((draft, name, profile_name)) = self.space_ready(&profiles, cx) else { return };
        if let Some(profile_name) = profile_name {
            let Some(editor) = self.profile_editor.take() else { return };
            let made = self.create_profile(&editor, &profile_name, &draft.profile_color, [&name, &draft.color, &draft.icon], window, cx);
            self.profile_editor = Some(editor);
            if made {
                self.close_space_card(Some(window), cx);
            }
            return;
        }
        let id = match draft.save(&self.core.disk, self.store(draft.private), &self.model.spaces, &profiles) {
            Ok(id) => id,
            Err(_) => {
                self.warning = Some("The space could not be saved.".into());
                return cx.notify();
            }
        };
        self.close_space_card(Some(window), cx);
        self.reload_spaces();
        Self::sync_other_windows(cx);
        if draft.editing.is_none()
            && let Some(ix) = self.model.spaces.iter().position(|s| s.private == draft.private && s.space.id == id)
        {
            self.switch_space(ix, window, cx);
        }
        cx.notify();
    }
}

struct Place {
    hosts: Option<(engine::Host, engine::Host)>,
    x: f32,
    bottom: f32,
    width: f32,
    height: Rc<Cell<f32>>,
}

pub struct SpacePopover {
    browser: WeakEntity<Browser>,
    place: Place,
    leave: Option<Task<()>>,
    removed: bool,
    _subscriptions: Vec<Subscription>,
}

impl SpacePopover {
    fn new(browser: WeakEntity<Browser>, place: Place, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe_window_activation(window, |this, window, cx| {
            let menu = this.browser.upgrade().is_some_and(|b| {
                let b = b.read(cx);
                let open = |input: &Entity<TextInput>| input.read(cx).menu_open();
                b.profile_editor.as_ref().is_some_and(|e| open(&e.name) || e.space.as_ref().is_some_and(|c| open(&c.name)))
            });
            if !window.is_window_active() && !menu {
                this.browser.update(cx, |b, _| b.space_closed = Some(Instant::now())).ok();
                this.leave(window, cx);
            }
        })];
        if let Some(b) = browser.upgrade() {
            subscriptions.push(cx.observe_in(&b, window, |this, b, window, cx| {
                if b.read(cx).profile_editor.as_ref().is_some_and(|e| e.floating) {
                    cx.notify();
                } else {
                    this.close(window, cx);
                }
            }));
        }
        SpacePopover { browser, place, leave: None, removed: false, _subscriptions: subscriptions }
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.leave.is_some() || self.removed {
            return;
        }
        cx.notify();
        let reduce = self.browser.upgrade().is_some_and(|b| b.read(cx).pref(Pref::ReduceMotion));
        self.leave = Some(cx.spawn_in(window, async move |this, cx| {
            if !reduce {
                cx.background_executor().timer(FADE).await;
            }
            this.update_in(cx, |this, window, cx| this.close(window, cx)).ok();
        }));
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.removed {
            return;
        }
        self.removed = true;
        let id = window.window_handle().window_id();
        self.browser
            .update(cx, |b, cx| {
                if b.space_popover.is_some_and(|(p, _)| p.window_id() == id) {
                    b.space_popover = None;
                    if b.profile_editor.as_ref().is_some_and(|e| e.floating) {
                        b.profile_editor = None;
                    }
                    cx.notify();
                }
            })
            .ok();
        window.remove_window();
    }
}

impl Render for SpacePopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let reduce = browser.read(cx).pref(Pref::ReduceMotion);
        let card = browser.update(cx, |b, cx| {
            let editor = b.profile_editor.as_ref().filter(|e| e.floating && e.space.is_some())?;
            Some(crate::profiles::editor_card(b, editor, window, cx))
        });
        let Some(card) = card else { return div().into_any_element() };
        let Place { hosts, x, bottom, width, height } = &self.place;
        let (hosts, x, bottom, width, height) = (hosts.clone(), *x, *bottom, *width, height.clone());
        let measure = canvas(
            move |bounds, _, cx| {
                let h = f32::from(bounds.size.height).min(bottom - TOOLBAR_HEIGHT);
                if (h - height.get()).abs() < 0.5 {
                    return;
                }
                height.set(h);
                if let Some((main, popup)) = hosts {
                    cx.spawn(async move |_| main.place(&popup, x.into(), (bottom - h).into(), width.into(), h.into())).detach();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let card = card.w_full().flex_none().max_h(px(bottom - TOOLBAR_HEIGHT)).child(measure);
        let card: AnyElement = if self.leave.is_some() {
            animate(card, "space-out", FADE, reduce, |el, t| el.opacity(1. - t).relative().top(px(t * 10.)))
        } else {
            animate(card, "space-in", POP, reduce, |el, t| el.opacity(t).relative().top(px((1. - t) * 10.)))
        };
        div().size_full().flex().flex_col().justify_end().child(card).into_any_element()
    }
}

pub(crate) fn parts(b: &Browser, editor: &ProfileEditor, card: &SpaceCard, window: &Window, cx: &mut Context<Browser>) -> Parts {
    let fields = b.space_fields();
    let ring = |f: Field| fields.get(editor.field) == Some(&f) && editor.focus.is_focused(window);
    let creating = card.editing.is_none();
    let tint = card.color.map_or(Theme::text(), |ix| rgb(PROFILE_COLORS[ix].1));
    let profiles = if creating { b.core.disk.profiles().unwrap_or_default() } else { Vec::new() };
    let mut body = div().flex().flex_col().gap_2();
    if creating {
        let chips = profiles.iter().map(|p| {
            let id = p.id;
            choice_row(("space-profile", id as usize), card.profile == Some(id))
                .child(profile_dot(&p.color, 10.))
                .child(bounded_title(&p.name))
                .on_click(cx.listener(move |b, _, _, cx| b.update_card(cx, |c| c.profile = Some(id))))
        });
        let new = choice_row("space-new-profile", card.profile.is_none())
            .child(div().w(px(10.)).flex().justify_center().child(icon(Icon::Plus, 12., Theme::muted())))
            .child("New Profile…")
            .on_click(cx.listener(|b, _, _, cx| b.update_card(cx, |c| c.profile = None)));
        body =
            body.child(caption("Profile")).child(focus_ring(div().flex().flex_col().gap(px(1.)).mx(px(-4.)), ring(Field::Profile)).children(chips).child(new));
        if card.profile.is_none() {
            body = body
                .child(
                    div()
                        .text_size(px(TEXT_SMALL))
                        .text_color(Theme::muted())
                        .child("Starts empty, with its own logins, cookies, site data, history and bookmarks."),
                )
                .child(div().pt_1().child(caption("Profile Name")))
                .child(input(editor.name.focus_handle(cx).is_focused(window)).child(editor.name.clone()))
                .child(div().pt_1().child(caption("Profile Color")))
                .child(color_row("new-profile-color", Some(editor.color), false, ring(Field::ProfileColor), cx, |e, ix| e.color = ix.unwrap_or(e.color)));
        }
        body = body.child(div().pt_1().child(caption("Space Name")));
    } else {
        body = body.child(caption("Name"));
    }
    body = body.child(input(card.name.focus_handle(cx).is_focused(window)).child(card.name.clone()));
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
    let ready = b.space_ready(&profiles, cx).is_some();
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
