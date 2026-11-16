use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use engine::menu::{self, Item};
use engine::nav_policy::PopupRule;
use futures::channel::oneshot;
use gpui::{
    AppContext, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, WindowHandle, actions, div, point, prelude::FluentBuilder, px, size,
};
use shield::ShieldLevel;
use store::Id;
use ui::controls::field;
use ui::controls::{POPOVER_INSET, button, caption, icon_button, open_popup, popover, segment, segmented, switch_row, tab_stop, tooltip};
use ui::icon::{Icon, icon};
use ui::motion::{REOPEN_GUARD, popover_motion};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{TEXT, TEXT_SMALL, Theme};
use url::Url;

use crate::address::measure;
use crate::browser::{Browser, TabKey};
use crate::peek::remove_after_exit;
use crate::prefs::Pref;
use crate::toolbar::{host_tail, shield_badge};
use crate::{ZoomIn, ZoomOut, ZoomReset};

actions!(tarn, [SiteSettings]);

const WIDTH: f32 = 320.;
const HEADER: f32 = 58.;
const ROW: f32 = 40.;
const SECTION: f32 = 28.;
const SHIELD: f32 = 64.;
const PAD: f32 = 8.;
const GAP: f32 = 4.;
const DECISIONS: [(&str, &str); 3] = [("allow", "Allow"), ("ask", "Ask"), ("deny", "Deny")];
/// Hands the profile's style sheets to its data store, which never applies them under Lockdown:
/// the theme's first, then each host's forced dark mode and the user's own CSS, which may adjust
/// both.
/// True when the profile's sheets changed.
pub(crate) fn apply_site_styles(core: &crate::shared::AppCore, private: bool, profile_id: store::Id) -> bool {
    let store = core.store(private);
    let mut styles = crate::themes::page_sheets(core, private, profile_id);
    styles.extend(engine::page_scripts::site_styles(store.site_css(profile_id).unwrap_or_default(), store.dark_sites(profile_id).unwrap_or_default()));
    core.profiles.borrow().get(&(private, profile_id)).is_some_and(|rt| rt.data.set_site_styles(&styles))
}

#[derive(Default)]
pub struct SitePopover {
    pub(crate) window: Option<WindowHandle<SiteSettingsWindow>>,
    /// The shield button, as last painted.
    pub(crate) anchor: Rc<Cell<Bounds<Pixels>>>,
    closed: Option<Instant>,
}

/// A remembered permission for the page's origin or for a frame embedded in it.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRow {
    pub origin: String,
    pub kind: String,
    pub label: String,
    pub decision: String,
}

/// The rows stored for `page` itself, then those of frames embedded in it, named by their host.
pub fn permission_rows(stored: Vec<(String, String, String)>, page: &str) -> Vec<PermissionRow> {
    let embedded = format!(" in {page}");
    let mut rows: Vec<(bool, PermissionRow)> = stored
        .into_iter()
        .filter_map(|(origin, kind, decision)| {
            let frame = match origin.strip_suffix(&embedded) {
                Some(frame) => Some(frame.to_owned()),
                None if origin == page => None,
                None => return None,
            };
            let label = match &frame {
                Some(frame) => format!("{} — {}", browser_core::permissions::kind_label(&kind), Url::parse(frame).ok()?.host_str()?),
                None => browser_core::permissions::kind_label(&kind),
            };
            Some((frame.is_some(), PermissionRow { origin, kind, label: bounded_title(&label), decision }))
        })
        .collect();
    rows.sort_by_key(|(embedded, _)| *embedded);
    rows.into_iter().map(|(_, row)| row).collect()
}

/// `zaps` counts the host's hidden elements, one row each under the CSS row.
fn card_height(permissions: usize, routable: bool, zaps: usize) -> f32 {
    let permissions = if permissions == 0 { 0. } else { SECTION + ROW * permissions as f32 };
    let route = if routable { ROW } else { 0. };
    HEADER + SHIELD + 6. * ROW + route + ROW * zaps as f32 + permissions + ROW + PAD
}

impl Browser {
    /// The active tab and the page it shows, when it has a host.
    pub(crate) fn site(&self) -> Option<(TabKey, Url)> {
        let key = self.model.active?;
        let url = Url::parse(&self.tab_url(key)?).ok()?;
        url.host_str()?;
        Some((key, url))
    }

    fn site_permissions(&self, key: TabKey, url: &Url) -> Vec<PermissionRow> {
        let Some(profile_id) = self.profile_of(key) else { return Vec::new() };
        let stored = self.store(key.private).permissions(profile_id).unwrap_or_default();
        permission_rows(stored.into_iter().filter(|(_, kind, _)| kind != crate::blocking::POPUPS).collect(), &url.origin().ascii_serialization())
    }

    /// The current profile's remembered permissions, for Settings › Permissions.
    pub fn profile_permissions(&self) -> Vec<browser_core::permissions::SitePermission> {
        let Some((private, profile_id)) = self.model.profile() else { return Vec::new() };
        browser_core::permissions::list(self.store(private), profile_id)
    }

    /// Allows, blocks or (with `None`) forgets a permission of the current profile.
    pub fn set_profile_permission(&mut self, origin: &str, kind: &str, allow: Option<bool>, cx: &mut Context<Self>) {
        let Some((private, profile_id)) = self.model.profile() else { return };
        if self.core.set_permission(private, profile_id, origin, kind, allow).is_ok() && kind == crate::blocking::POPUPS {
            self.core.sync_shield();
        }
        cx.notify();
    }

    pub fn set_site_permission(&mut self, key: TabKey, origin: &str, kind: &str, decision: Option<&str>, cx: &mut Context<Self>) {
        let Some(profile_id) = self.profile_of(key) else { return };
        let store = self.store(key.private);
        match decision {
            Some(decision) => store.set_permission(profile_id, origin, kind, decision),
            None => store.delete_permission(profile_id, origin, kind),
        }
        .ok();
        cx.notify();
    }

    /// Sets or removes the site's Shield override in the current profile and rebuilds its open pages.
    pub fn set_site_shield(&mut self, host: &str, level: Option<ShieldLevel>, window: &mut Window, cx: &mut Context<Self>) {
        let host = host.trim_end_matches('.');
        let Some(space) = self.model.current_space() else { return };
        let (private, profile_id) = (space.private, space.space.profile_id);
        if self.store(private).set_site_override(profile_id, host, level).is_err() {
            return;
        }
        if let Some(rt) = self.loaded(private, profile_id) {
            match level {
                Some(level) => rt.overrides.borrow_mut().insert(host.to_owned(), level),
                None => rt.overrides.borrow_mut().remove(host),
            };
        }
        self.core.apply_extensions(private, profile_id);
        // Frames and back/forward-cache pages of the site may live in any Chromium view.
        let in_chromium = |b: &Browser| b.views.iter().any(|(k, v)| v.is_chromium() && k.private == private && b.profile_of(*k) == Some(profile_id));
        let mut chromium = in_chromium(self);
        crate::shared::others(cx, |b, _, _| chromium |= in_chromium(b));
        let restart = (level != Some(ShieldLevel::Lockdown) || !chromium).then_some(host);
        self.restart_profile_views(private, profile_id, restart, false, window, cx);
        #[cfg(feature = "chromium")]
        if level == Some(ShieldLevel::Lockdown)
            && let Some(own_store) = self.loaded(private, profile_id).map(|rt| rt.own_store.clone())
        {
            self.clear_chromium_site(&own_store, host);
        }
    }

    /// The site's own style sheet in the current profile.
    pub fn site_css(&self, host: &str) -> String {
        let Some((private, profile_id)) = self.model.profile() else { return String::new() };
        self.store(private).site_css_for(profile_id, host).unwrap_or_default()
    }

    /// Saves the site's style sheet in the current profile and rebuilds its open pages on that
    /// host, Chromium views included, which take their style sheets when created; Windows restyles
    /// open pages in place instead.
    pub fn set_site_css(&mut self, host: &str, css: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), store::Error> {
        let Some((private, profile_id)) = self.model.profile() else { return Ok(()) };
        self.store(private).set_site_css(profile_id, host, css)?;
        self.apply_site_styles(private, profile_id);
        if !cfg!(windows) {
            self.restart_profile_views(private, profile_id, Some(host), false, window, cx);
        }
        Ok(())
    }

    pub fn site_dark(&self, host: &str) -> bool {
        self.model.profile().is_some_and(|(private, profile_id)| self.store(private).site_is_dark(profile_id, host).unwrap_or(false))
    }

    /// Turns forced dark mode on or off for `host` in the current profile and rebuilds its open pages
    /// (in place on Windows).
    pub fn set_site_dark(&mut self, host: &str, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((private, profile_id)) = self.model.profile() else { return };
        if self.store(private).set_site_dark(profile_id, host, dark).is_ok() {
            self.apply_site_styles(private, profile_id);
            if !cfg!(windows) {
                self.restart_profile_views(private, profile_id, Some(host), false, window, cx);
            }
        }
    }

    pub(crate) fn apply_site_styles(&self, private: bool, profile_id: store::Id) {
        apply_site_styles(&self.core, private, profile_id);
    }

    /// The non-private spaces by id and name, with the one a rule for exactly `host` names.
    pub(crate) fn route_choices(&self, host: &str) -> (Vec<(Id, String)>, Option<Id>) {
        let spaces = self.model.spaces.iter().filter(|s| !s.private).map(|s| (s.space.id, bounded_title(&s.space.name))).collect();
        let rules = self.core.disk.space_rules().unwrap_or_default();
        (spaces, rules.into_iter().find(|r| r.host_pattern == host).map(|r| r.space_id))
    }

    /// Rules as (id, host pattern, space name) for Settings.
    pub(crate) fn space_rule_rows(&self) -> Vec<(Id, String, String)> {
        let name = |id| self.model.spaces.iter().find(|s| !s.private && s.space.id == id).map(|s| bounded_title(&s.space.name));
        self.core.disk.space_rules().unwrap_or_default().into_iter().filter_map(|r| Some((r.id, r.host_pattern, name(r.space_id)?))).collect()
    }

    /// Opens new tabs of `host` in `space_id`, or wherever they are opened when `None`.
    pub fn route_site(&mut self, host: &str, space_id: Option<Id>, cx: &mut Context<Self>) {
        let disk = &self.core.disk;
        match space_id {
            Some(id) if disk.set_space_rule(host, id).is_err() => return self.show_toast("Tarn could not save this rule", None, cx),
            Some(_) => {}
            None => {
                for rule in disk.space_rules().unwrap_or_default().into_iter().filter(|r| r.host_pattern == host) {
                    disk.delete_space_rule(rule.id).ok();
                }
            }
        }
        cx.notify();
    }

    pub fn delete_space_rule(&mut self, id: Id, cx: &mut Context<Self>) {
        self.core.disk.delete_space_rule(id).ok();
        cx.notify();
    }

    pub fn toggle_site_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.site_popover.window.take() {
            Some(handle) => {
                cx.defer(move |cx| drop(handle.update(cx, |popover, window, cx| popover.close(true, window, cx))));
                cx.notify();
            }
            // The click that took focus from the popover already closed it; it must not reopen it.
            None if self.site_popover.closed.is_some_and(|t| t.elapsed() < REOPEN_GUARD) => {}
            None => self.show_site_settings(window, cx),
        }
    }

    /// Opens the popover under the shield button, or under the toolbar's centre when it is hidden.
    fn show_site_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let Some((key, url)) = self.site() else { return };
        self.commit_rename(cx);
        let rows = self.site_permissions(key, &url).len();
        let zaps = self.zapped(url.host_str()).len();
        let anchor = self.site_popover.anchor.get();
        let origin = window.bounds();
        let center = if anchor.size.width > px(0.) { anchor.center().x } else { origin.size.width / 2. };
        let x = (center - px(WIDTH / 2. + POPOVER_INSET)).clamp(px(0.), (origin.size.width - px(WIDTH + 2. * POPOVER_INSET)).max(px(0.)));
        let top = if anchor.size.height > px(0.) { anchor.bottom() + px(GAP) } else { px(ui::theme::TOOLBAR_HEIGHT) };
        // Many hidden elements or permissions scroll inside the card rather than run off the window.
        let room = (origin.size.height - top - px(GAP)).max(px(200.));
        let bounds = Bounds::new(
            point(origin.origin.x + x, origin.origin.y + top),
            size(px(WIDTH + 2. * POPOVER_INSET), px(card_height(rows, !self.model.private(), zaps) + POPOVER_INSET).min(room)),
        );
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let inset = [0., POPOVER_INSET, POPOVER_INSET, POPOVER_INSET];
            let (handle, _) =
                open_popup(cx, bounds, display, true, inset, Theme::radius_panel(), |window, cx| SiteSettingsWindow::new(browser.clone(), main, window, cx));
            browser
                .update(cx, |b, cx| {
                    b.site_popover.window = handle;
                    cx.notify();
                })
                .ok();
        });
    }
}

pub struct SiteSettingsWindow {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    focus: FocusHandle,
    armed: Option<String>,
    closing: bool,
    css: Entity<TextInput>,
    /// The host the CSS field was filled for.
    css_host: Option<String>,
    /// Why the last save failed.
    css_error: Option<String>,
    /// A native menu is open; it takes activation from the card without closing it.
    holding: bool,
    /// The "Always open in" button, as last painted, so its menu drops from it.
    route_anchor: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl SiteSettingsWindow {
    fn new(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut subscriptions = vec![cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && !this.holding && !this.css.read(cx).menu_open() {
                this.close(false, window, cx);
            }
        })];
        subscriptions.extend(browser.upgrade().map(|b| {
            cx.observe(&b, |this, b, cx| {
                this.follow_site(&b, cx);
                cx.notify();
            })
        }));
        let host = browser.upgrade().and_then(|b| b.read(cx).site()).and_then(|(_, url)| url.host_str().map(str::to_owned));
        let current = browser.upgrade().zip(host.as_ref()).map(|(b, host)| b.read(cx).site_css(host)).unwrap_or_default();
        let css = cx.new(|cx| {
            let mut input = TextInput::new("body { font-size: 18px }", cx);
            input.set_text(current, cx);
            input
        });
        subscriptions.push(cx.subscribe_in(&css, window, |this, _, event, window, cx| match event {
            TextInputEvent::Submit => this.save_css(cx),
            TextInputEvent::Cancel => this.close(true, window, cx),
            TextInputEvent::Changed => this.css_error = None,
            TextInputEvent::MenuClosed if !window.is_window_active() && !this.holding => this.close(false, window, cx),
            _ => {}
        }));
        SiteSettingsWindow {
            browser,
            main,
            focus,
            armed: None,
            closing: false,
            css,
            css_host: host,
            css_error: None,
            holding: false,
            route_anchor: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The page may move to another host while the card is open; the CSS field follows it, as
    /// the rest of the card does, so a save never lands on the host the card opened for.
    fn follow_site(&mut self, browser: &gpui::Entity<Browser>, cx: &mut Context<Self>) {
        let Some(host) = browser.read(cx).site().and_then(|(_, url)| url.host_str().map(str::to_owned)) else { return };
        if self.css_host.as_deref() == Some(host.as_str()) {
            return;
        }
        let current = browser.read(cx).site_css(&host);
        self.css.update(cx, |input, cx| input.set_text(current, cx));
        self.css_host = Some(host);
        self.css_error = None;
    }

    fn save_css(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.css_host.clone() else { return };
        let css = self.css.read(cx).text().to_owned();
        self.css_error = (css.len() > store::MAX_SITE_CSS).then(|| format!("CSS over {} KiB", store::MAX_SITE_CSS / 1024));
        if self.css_error.is_none() {
            self.on_main(cx, move |b, w, cx| {
                if let Err(e) = b.set_site_css(&host, &css, w, cx) {
                    b.warning = Some(format!("Could not save the site's CSS ({e})."));
                }
            });
        }
        cx.notify();
    }

    fn css_row(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let label = self.css_error.clone().unwrap_or_else(|| "Site CSS".into());
        let focused = self.css.focus_handle(cx).is_focused(window);
        Self::row(label)
            .text_color(if self.css_error.is_some() { Theme::warning() } else { Theme::text() })
            .child(field(focused).w(px(150.)).py_0p5().overflow_hidden().child(self.css.clone()))
            .child(tab_stop(button("save-css", "Save", false)).on_click(cx.listener(|this, _, _, cx| this.save_css(cx))))
    }

    fn close(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.closing, true) {
            return;
        }
        let reduce = self.browser.upgrade().is_some_and(|b| b.read(cx).pref(Pref::ReduceMotion));
        remove_after_exit(reduce, window, cx);
        self.browser
            .update(cx, |b, cx| {
                b.site_popover.window = None;
                b.site_popover.closed = (!refocus).then(Instant::now);
                cx.notify();
            })
            .ok();
        if refocus {
            self.main.update(cx, |_, window, _| window.activate_window()).ok();
        }
        cx.notify();
    }

    /// Runs `f` in the main window, where the browser's actions expect to run.
    fn on_main(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) + 'static) {
        let main = self.main;
        cx.defer(move |cx| drop(main.update(cx, f)));
    }

    fn header(url: &Url, host: &str) -> impl IntoElement {
        let secure = url.scheme() == "https";
        let (glyph, color, text) = if secure {
            (Icon::ShieldCheck, Theme::success(), "Connection is secure")
        } else {
            (Icon::Shield, Theme::warning(), "Not Secure — this page is not encrypted")
        };
        div()
            .h(px(HEADER))
            .flex_none()
            .px_3()
            .flex()
            .flex_col()
            .justify_center()
            .gap_0p5()
            .border_b_1()
            .border_color(Theme::hairline())
            .child(div().font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().child(SharedString::from(host.to_owned())))
            .child(div().flex().items_center().gap_1p5().text_size(px(TEXT_SMALL)).text_color(color).child(icon(glyph, 12., color)).child(text))
    }

    fn shield(&self, host: &str, site: Option<ShieldLevel>, default: ShieldLevel, cx: &mut Context<Self>) -> gpui::Div {
        let choices = [
            (None, "Default".to_owned()),
            (Some(ShieldLevel::Standard), "Standard".into()),
            (Some(ShieldLevel::Strict), "Strict".into()),
            (Some(ShieldLevel::Lockdown), "Lockdown".into()),
        ];
        let segments = choices.into_iter().enumerate().map(|(ix, (level, text))| {
            let host = host.to_owned();
            tab_stop(segment(("shield-level", ix), text, site == level)).on_click(cx.listener(move |this, _, _, cx| {
                let host = host.clone();
                this.on_main(cx, move |b, w, cx| b.set_site_shield(&host, level, w, cx));
            }))
        });
        div()
            .h(px(SHIELD))
            .flex_none()
            .px_3()
            .flex()
            .flex_col()
            .justify_center()
            .gap_1p5()
            .child(
                div()
                    .text_size(px(TEXT_SMALL))
                    .text_color(Theme::muted())
                    .child(SharedString::from(format!("Shield for this site · profile default {}", shield_badge(default)))),
            )
            .child(segmented().children(segments))
    }

    fn switch_row(id: &'static str, label: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
        switch_row(id, true, div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label), on).h(px(ROW)).py_0().rounded_none()
    }

    fn row(label: impl Into<SharedString>) -> gpui::Div {
        div()
            .h(px(ROW))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label.into()))
    }

    fn zoom(&self, percent: f64, cx: &mut Context<Self>) -> gpui::Div {
        let zoom = |id: &'static str, glyph: Icon, tip: &'static str, action: &dyn gpui::Action, factor: Option<f64>| {
            tab_stop(icon_button(id, glyph, true))
                .tooltip(tooltip(tip, Some(crate::shortcuts::shown(action, cx).into())))
                .on_click(cx.listener(move |this, _, _, cx| this.on_main(cx, move |b, _, cx| b.zoom(factor, cx))))
        };
        Self::row("Zoom")
            .child(zoom("zoom-out", Icon::ChevronLeft, "Zoom Out", &crate::ZoomOut, Some(1. / 1.1)))
            .child(div().w(px(44.)).flex().justify_center().text_color(Theme::muted()).child(SharedString::from(format!("{percent}%"))))
            .child(zoom("zoom-in", Icon::ChevronRight, "Zoom In", &crate::ZoomIn, Some(1.1)))
            .child(tab_stop(button("zoom-reset", "Reset", false)).on_click(cx.listener(|this, _, _, cx| this.on_main(cx, |b, _, cx| b.zoom(None, cx)))))
    }

    fn popups(&self, origin: &str, rule: PopupRule, cx: &mut Context<Self>) -> gpui::Div {
        let choices = [(PopupRule::Default, "Default"), (PopupRule::Allow, "Allow"), (PopupRule::Block, "Block")];
        let segments = choices.into_iter().enumerate().map(|(ix, (choice, text))| {
            let origin = origin.to_owned();
            tab_stop(segment(("popups", ix), text, rule == choice)).on_click(cx.listener(move |this, _, _, cx| {
                this.browser.update(cx, |b, cx| b.set_site_popups(&origin, choice, cx)).ok();
            }))
        });
        Self::row("Pop-ups").child(segmented().children(segments))
    }

    fn route(&self, host: &str, spaces: Vec<(Id, String)>, current: Option<Id>, cx: &mut Context<Self>) -> gpui::Div {
        let shown = current.and_then(|id| spaces.iter().find(|(s, _)| *s == id)).map_or("Any Space".to_owned(), |(_, name)| name.clone());
        let host = host.to_owned();
        let anchor = self.route_anchor.clone();
        Self::row("Always open in").child(tab_stop(button("route-site", shown, false)).child(measure(anchor.clone())).on_click(cx.listener(
            move |this, _, window, cx| {
                let Some(menu_host) = engine::Host::from_window(window) else { return };
                let at = anchor.get();
                let at = (at.size.width > px(0.)).then(|| (f32::from(at.origin.x).into(), f32::from(at.bottom()).into()));
                let mut items = vec![Item::action("Any Space", None).checked(current.is_none())];
                items.extend(spaces.iter().map(|(id, name)| Item::action(name.clone(), Some(*id)).checked(current == Some(*id))));
                let host = host.clone();
                this.holding = true;
                cx.spawn_in(window, async move |this, cx| {
                    let (tx, rx) = oneshot::channel();
                    menu::show(&menu_host, items, at, move |space| {
                        tx.send(space).ok();
                    });
                    let chosen = rx.await.ok();
                    this.update_in(cx, |this, window, cx| {
                        this.holding = false;
                        if let Some(space) = chosen {
                            this.browser.update(cx, |b, cx| b.route_site(&host, space, cx)).ok();
                        }
                        if !window.is_window_active() {
                            this.close(false, window, cx);
                        }
                    })
                    .ok();
                })
                .detach();
            },
        )))
    }

    fn permission(&self, key: TabKey, ix: usize, row: &PermissionRow, cx: &mut Context<Self>) -> gpui::Div {
        let segments = DECISIONS.into_iter().enumerate().map(|(n, (decision, text))| {
            let (origin, kind) = (row.origin.clone(), row.kind.clone());
            tab_stop(segment(("permission", ix * DECISIONS.len() + n), text, row.decision == decision)).on_click(cx.listener(move |this, _, _, cx| {
                this.browser.update(cx, |b, cx| b.set_site_permission(key, &origin, &kind, Some(decision), cx)).ok();
            }))
        });
        let (origin, kind) = (row.origin.clone(), row.kind.clone());
        Self::row(row.label.clone()).child(segmented().children(segments)).child(
            tab_stop(icon_button(("permission-remove", ix), Icon::Close, true)).tooltip(tooltip("Remove", None)).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.browser.update(cx, |b, cx| b.set_site_permission(key, &origin, &kind, None, cx)).ok();
                },
            )),
        )
    }
}

impl Render for SiteSettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let reduce = b.pref(Pref::ReduceMotion);
        let Some((key, url)) = b.site() else { return div().into_any_element() };
        let host = url.host_str().unwrap_or_default().to_owned();
        let shown = host_tail(host.trim_start_matches("www."), crate::toolbar::MAX_HOST);
        let permissions = b.site_permissions(key, &url);
        let muted = b.tab_muted(key);
        let pause_host = crate::blocking::site_host(&url).unwrap_or_default().to_owned();
        let paused = b.site_paused(&pause_host);
        let origin = crate::blocking::popup_origin(&url);
        let popups = b.site_popups(&origin);
        let dark = b.site_dark(&host);
        let site_level = b.has_override(&host).then(|| b.level_for(url.as_str()));
        let default = b.current_profile().map(|p| p.shield_level).unwrap_or_default();
        let percent = b.active_view().map_or(100., |v| (v.zoom() * 100.).round());
        let storable = host.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'));
        let route = (!b.model.private() && storable).then(|| b.route_choices(host.trim_end_matches('.')));
        let zapped = b.zapped(Some(&host));
        let permission_rows: Vec<_> = permissions.iter().enumerate().map(|(ix, row)| self.permission(key, ix, row, cx)).collect();
        let armed = self.armed.as_deref() == Some(host.as_str());
        let clear_host = host.clone();
        let css_row = self.css_row(window, cx);
        let zap_rows: Vec<_> = zapped
            .into_iter()
            .enumerate()
            .map(|(ix, (host, selector))| {
                Self::row(format!("Hidden: {selector}")).child(tab_stop(button(("unzap", ix), "Undo", false)).on_click(cx.listener(move |this, _, _, cx| {
                    this.browser.update(cx, |b, cx| b.unzap(&host, Some(&selector), cx)).ok();
                })))
            })
            .collect();
        let dark_host = host.clone();
        let typing = self.css.focus_handle(cx).is_focused(window);
        let rows = div()
            .id("site-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(PAD))
            .child(self.shield(&host, site_level, default, cx))
            .child(self.zoom(percent, cx))
            .child(Self::switch_row("mute-site", "Mute this site", muted).on_click(cx.listener(move |this, _, _, cx| {
                this.browser.update(cx, |b, cx| b.toggle_site_muted(key, cx)).ok();
            })))
            .child(Self::switch_row("block-site", "Block ads, trackers and cookie banners", !paused).on_click(cx.listener(move |this, _, _, cx| {
                let host = pause_host.clone();
                this.on_main(cx, move |b, w, cx| b.set_site_paused(&host, !paused, w, cx));
            })))
            .child(self.popups(&origin, popups, cx))
            .child(Self::switch_row("dark-site", "Dark mode for this site", dark).on_click(cx.listener(move |this, _, _, cx| {
                let host = dark_host.clone();
                this.on_main(cx, move |b, w, cx| b.set_site_dark(&host, !dark, w, cx));
            })))
            .child(css_row)
            .children(zap_rows)
            .when_some(route, |d, (spaces, current)| d.child(self.route(host.trim_end_matches('.'), spaces, current, cx)))
            .when(!permissions.is_empty(), |d| d.child(caption("Permissions").h(px(SECTION)).flex_none().px_3().pt_2()).children(permission_rows))
            .child(
                Self::row(if armed { "Removes cookies, storage and caches" } else { "Stored data" })
                    .text_color(if armed { Theme::warning() } else { Theme::text() })
                    .child(tab_stop(button("clear-site", if armed { "Clear Now" } else { "Clear Site Data…" }, armed)).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if crate::settings::confirm_twice(&mut this.armed, clear_host.clone()) {
                                let host = clear_host.clone();
                                this.browser.update(cx, |b, cx| b.clear_site(key, host, cx)).ok();
                            }
                            cx.notify();
                        },
                    ))),
            );
        let card = popover(Theme::radius_panel(), 0.).flex().flex_col().child(Self::header(&url, &shown)).child(rows);
        let content = div().relative().size_full().text_size(px(TEXT)).text_color(Theme::text()).child(card);
        let content = popover_motion(content, "site", self.closing, reduce);
        div()
            .track_focus(&self.focus)
            .key_context("SiteSettings")
            .on_action(cx.listener(|this, _: &SiteSettings, window, cx| this.close(true, window, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.on_main(cx, |b, _, cx| b.zoom(Some(1.1), cx))))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.on_main(cx, |b, _, cx| b.zoom(Some(1. / 1.1), cx))))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.on_main(cx, |b, _, cx| b.zoom(None, cx))))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => this.close(true, window, cx),
                "tab" if e.keystroke.modifiers.shift => window.focus_prev(),
                "tab" => window.focus_next(),
                "m" if !typing && !e.keystroke.modifiers.modified() => {
                    this.browser.update(cx, |b, cx| b.toggle_site_muted(key, cx)).ok();
                }
                _ => {}
            }))
            .size_full()
            .child(content)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(origin: &str, kind: &str, decision: &str) -> (String, String, String) {
        (origin.into(), kind.into(), decision.into())
    }

    #[test]
    fn rows_belong_to_the_page_and_its_frames() {
        let rows = permission_rows(
            vec![
                stored("https://meet.test in https://a.test", "camera", "allow"),
                stored("https://a.test", "microphone", "deny"),
                stored("https://other.test", "camera", "allow"),
                stored("https://a.test in https://other.test", "camera", "allow"),
                stored("https://a.test", "camera+microphone", "ask"),
            ],
            "https://a.test",
        );
        let labels: Vec<(&str, &str)> = rows.iter().map(|r| (r.label.as_str(), r.decision.as_str())).collect();
        assert_eq!(labels, [("Microphone", "deny"), ("Camera and Microphone", "ask"), ("Camera — meet.test", "allow")]);
        assert_eq!(rows[2].origin, "https://meet.test in https://a.test");
    }

    #[test]
    fn the_card_grows_with_its_permissions() {
        assert_eq!(card_height(1, false, 0) - card_height(0, false, 0), SECTION + ROW);
        assert_eq!(card_height(3, false, 0) - card_height(1, false, 0), 2. * ROW);
        assert_eq!(card_height(0, true, 0) - card_height(0, false, 0), ROW);
        assert_eq!(card_height(0, false, 2) - card_height(0, false, 0), 2. * ROW);
    }
}
