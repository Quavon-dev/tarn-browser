use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, SharedString, Stateful, StatefulInteractiveElement,
    Styled, div, prelude::FluentBuilder, px,
};
use shield::ShieldLevel;
use std::time::Duration;
use store::WebEngine;
use ui::controls::{icon_button, icon_frame, spinner, tooltip};
use ui::icon::{ICON_SMALL, Icon, icon};
use ui::text::bounded_title;
use ui::theme::{SYSTEM_FONT, Secure, TEXT, TEXT_SMALL, TOOLBAR_HEIGHT, TRAFFIC_LIGHT_INSET, Theme};
use url::Url;

use crate::address::measure;
use crate::browser::{Browser, CommandMode, PageState};
use crate::prefs::Pref;
use crate::sidebar::SIDEBAR_MARGIN;

pub fn shield_badge(level: ShieldLevel) -> &'static str {
    match level {
        ShieldLevel::Standard => "Standard",
        ShieldLevel::Strict => "Strict",
        ShieldLevel::Lockdown => "Lockdown",
    }
}

pub fn render(b: &Browser, cx: &mut Context<Browser>) -> AnyElement {
    let inset = (TRAFFIC_LIGHT_INSET - (b.sidebar_width + SIDEBAR_MARGIN) * b.sidebar_shown()).max(8.);
    let bar = if b.pref(Pref::CompactToolbar) { render_compact(b, cx).pl(px(inset)) } else { render_classic(b, inset, cx).pl_2() };
    bar.h(px(TOOLBAR_HEIGHT))
        .flex_none()
        .pr_2()
        .flex()
        .items_center()
        .text_size(px(TEXT))
        .text_color(Theme::text())
        .on_mouse_down(MouseButton::Right, cx.listener(|b, _, w, cx| b.toolbar_menu(w, cx)))
        .on_scroll_wheel(cx.listener(|b, e, w, cx| b.toolbar_swipe(e, false, w, cx)))
        .relative()
        .child(measure(b.address.bar.clone()))
        .when(b.inline_editing(), |d| d.child(hidden_field(b, cx)))
        .window_control_area(gpui::WindowControlArea::Drag)
        .children(ui::controls::window_controls())
        .into_any_element()
}

/// The shortest time the reload arrow spins after Reload, so a page served from cache still shows it.
pub(crate) const SPIN: Duration = Duration::from_millis(600);

/// Reload, whose arrow spins while the page loads; a click then stops loading. With Reduce Motion,
/// or while Tarn is in the background, a stop button stands in for the spin.
fn reload_button(b: &Browser, page: &PageState, cx: &mut Context<Browser>) -> Stateful<Div> {
    let spinning = page.loading || b.reload_spin.is_some_and(|(key, at)| b.model.active == Some(key) && at.elapsed() < SPIN);
    if !spinning {
        let enabled = b.model.active.is_some();
        return icon_button("reload", Icon::Reload, enabled)
            .tooltip(crate::shortcuts::tip("Reload", &crate::Reload))
            .on_click(cx.listener(|b, _, _, cx| b.reload(cx)));
    }
    // Past a finished cached reload the arrow still turns for `SPIN`; a click then reloads again.
    let stop = cx.listener(|b, _, _, cx| {
        if !b.model.active.is_some_and(|k| b.loading(k)) {
            return b.reload(cx);
        }
        b.reload_spin = None;
        if let Some(view) = b.active_view() {
            view.stop();
        }
        cx.notify();
    });
    let tip = if page.loading { "Stop Loading" } else { "Reload" };
    if b.still() {
        return icon_button("stop", Icon::Close, true).tooltip(tooltip(tip, None)).on_click(stop);
    }
    icon_frame("stop", true).child(spinner(16., Theme::text())).tooltip(tooltip(tip, None)).on_click(stop)
}

/// With the sidebar hidden nothing else says the window is private, so a quiet amber mark does.
fn private_mark(b: &Browser) -> Option<impl IntoElement> {
    let private = b.model.current_space().is_some_and(|s| s.private);
    (private && !b.sidebar_open).then(|| {
        let (glyph, tip) = if b.model.burner_shown() {
            (Icon::Flame, "Burner tab: nothing is kept after it closes")
        } else {
            (Icon::Hand, "Private space: kept in memory only")
        };
        div().id("private-mark").ml_1().size(px(26.)).flex().items_center().justify_center().tooltip(tooltip(tip, None)).child(icon(
            glyph,
            14.,
            Theme::warning(),
        ))
    })
}

fn navigation(b: &Browser, page: &PageState, cx: &mut Context<Browser>) -> Div {
    let reload = reload_button(b, page, cx);
    let sidebar = if b.sidebar_open { "Hide Sidebar" } else { "Show Sidebar" };
    div()
        .occlude()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .child(
            icon_button("toggle-sidebar", Icon::Sidebar, true)
                .tooltip(crate::shortcuts::tip(sidebar, &crate::ToggleSidebar))
                .on_click(cx.listener(|b, _, _, cx| b.toggle_sidebar(cx))),
        )
        .child(icon_button("back", Icon::Back, page.can_back).tooltip(crate::shortcuts::tip("Back", &crate::Back)).on_click(cx.listener(|b, _, _, _| b.back())))
        .child(
            icon_button("forward", Icon::Forward, page.can_forward)
                .tooltip(crate::shortcuts::tip("Forward", &crate::Forward))
                .on_click(cx.listener(|b, _, _, _| b.forward())),
        )
        .child(reload)
        .children(private_mark(b))
}

/// Split view and Share, for the page on screen. Share dispatches one action, so the share
/// menu can take its place.
fn page_actions(b: &Browser) -> Div {
    let page = b.model.active.is_some();
    let split = if b.model.active.is_some_and(|a| b.splits.contains(a)) { "Close Split View" } else { "Open Split View" };
    div()
        .occlude()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .child(
            icon_button("split-view", Icon::SplitAdd, page)
                .tooltip(crate::shortcuts::tip(split, &crate::split::ToggleSplitView))
                .on_click(|_, w, cx| w.dispatch_action(Box::new(crate::split::ToggleSplitView), cx)),
        )
        .child(
            icon_button("share", Icon::Share, page)
                .tooltip(crate::shortcuts::tip("Share", &crate::qr::ShareLink))
                .on_click(|_, w, cx| w.dispatch_action(Box::new(crate::qr::ShareLink), cx)),
        )
}

/// Windows has no menu bar; this button opens the same menus.
fn app_menu_button(b: &Browser, cx: &mut Context<Browser>) -> Option<Stateful<Div>> {
    cfg!(windows).then(|| {
        icon_button("app-menu", Icon::Ellipsis, true)
            .child(measure(b.app_menu_anchor.clone()))
            .occlude()
            .tooltip(crate::shortcuts::tip("Menu", &crate::OpenAppMenu))
            .on_click(cx.listener(|b, _, w, cx| b.open_app_menu(w, cx)))
    })
}

/// At rest both sides share the free width equally, so the address chip sits in the middle of
/// the bar; `inset` is the traffic lights' room, kept inside the left side. Open, the field takes
/// all the width between them.
fn render_classic(b: &Browser, inset: f32, cx: &mut Context<Browser>) -> Div {
    let page = b.model.active.map(|k| b.page(k)).unwrap_or_default();
    let side = || div().flex().items_center().flex_1();
    div()
        .gap_2()
        .child(side().child(div().flex_none().w(px(inset - 8.))).child(navigation(b, &page, cx)))
        .child(div().w(px(560.)).min_w_0().relative().flex().child(measure(b.address.slot.clone())).child(origin_chip(b, cx)))
        .child(
            side()
                .justify_end()
                .gap_2()
                .when(b.devtools_on(), |d| d.child(devtools(b, cx)))
                .child(crate::extensions::buttons(b, cx))
                .child(page_actions(b))
                .children(app_menu_button(b, cx)),
        )
}

pub(crate) struct Site {
    url: Option<Url>,
    pub(crate) host: Option<String>,
    pub(crate) insecure: bool,
    /// Shield and bookmark buttons, which act on a plain host.
    controls: bool,
}

fn site(b: &Browser) -> Site {
    site_of(b.active_view().and_then(|v| v.url()).or_else(|| b.model.active_tab().and_then(|t| Url::parse(&t.url).ok())))
}

pub(crate) fn site_of(url: Option<Url>) -> Site {
    let address = url.as_ref().map(browser_core::command::address);
    Site { insecure: address.as_ref().is_some_and(|a| a.1), host: address.map(|a| a.0), controls: url.as_ref().is_some_and(|u| u.host_str().is_some()), url }
}

pub(crate) use browser_core::command::{MAX_HOST, host_tail};

fn address_text(site: &Site) -> (String, gpui::Rgba) {
    match (&site.host, site.insecure) {
        (Some(h), true) => (format!("Not Secure — {h}"), Theme::warning()),
        (Some(h), false) => (h.clone(), Secure::text()),
        (None, _) => ("Search or enter address".to_owned(), Secure::faint()),
    }
}

/// What Shield stopped on the site on screen, for the current profile.
fn site_count(b: &Browser, site: &Site) -> Option<i64> {
    let host = site.url.as_ref().and_then(browser_core::command::page_host)?;
    let space = b.model.current_space()?;
    b.store(space.private).shield_site_total(space.space.profile_id, &host).ok().filter(|n| *n > 0)
}

/// The Shield chip at the start of the address: the level's glyph and what Shield stopped on
/// the site; a click opens Site Settings.
fn shield_button(b: &Browser, site: &Site, cx: &mut Context<Browser>) -> impl IntoElement {
    let level = site.url.as_ref().map(|u| b.level_for(u.as_str())).unwrap_or_default();
    let overridden = site.host.as_deref().is_some_and(|h| b.has_override(h));
    let shield_color = if level == ShieldLevel::Standard { Secure::muted() } else { Secure::shield_raised() };
    let open = b.site_popover.window.is_some();
    let count = site_count(b, site);
    div()
        .id("shield")
        .occlude()
        .h(px(24.))
        .px_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .rounded(px(Theme::radius_small()))
        .bg(if open || overridden { Secure::pressed() } else { Secure::control() })
        .when(!open, |d| d.hover(|s| s.bg(Secure::hover())).active(|s| s.bg(Secure::pressed())))
        .cursor_pointer()
        .text_size(px(TEXT_SMALL))
        .text_color(shield_color)
        .relative()
        .child(measure(b.site_popover.anchor.clone()))
        .when(!open, |d| d.tooltip(tooltip(format!("Shield: {} — Site Settings", shield_badge(level)), None)))
        .child(icon(if level == ShieldLevel::Standard { Icon::Shield } else { Icon::ShieldCheck }, 14., shield_color))
        .when_some(count, |d, n| d.child(browser_core::shield_card::thousands(n.unsigned_abs())))
        .on_click(cx.listener(|b, _, window, cx| {
            cx.stop_propagation();
            b.toggle_site_settings(window, cx);
        }))
}

/// A button inside the origin chip: shorter than the chip, so its hover fill keeps clear of the chip's edge.
fn chip_frame(id: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px_1p5()
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(Theme::radius_small()))
        .when(enabled, |d| d.hover(|s| s.bg(Secure::hover())).active(|s| s.bg(Secure::pressed())).cursor_pointer())
}

fn chip_button(id: &'static str, glyph: Icon, enabled: bool) -> Stateful<Div> {
    chip_frame(id, enabled).child(icon(glyph, 14., if enabled { Secure::muted() } else { Secure::faint() }))
}

fn bookmark_button(b: &Browser, site: &Site, cx: &mut Context<Browser>) -> impl IntoElement {
    let url = site.url.as_ref().map(Url::to_string);
    let marked = url.as_deref().is_some_and(|u| b.is_bookmarked(u));
    let (glyph, tip) = if marked { (Icon::StarFill, "Remove Bookmark") } else { (Icon::Star, "Bookmark This Tab") };
    chip_button("bookmark", glyph, url.is_some())
        .occlude()
        .tooltip(tooltip(tip, (!marked).then(|| crate::shortcuts::shown(&crate::BookmarkTab, cx).into())))
        .on_click(cx.listener(move |b, _, _, cx| {
            cx.stop_propagation();
            if marked {
                b.unbookmark_tab(None, cx);
            } else {
                b.bookmark_tab(None, cx);
            }
        }))
}

fn reader_button(b: &Browser, cx: &mut Context<Browser>) -> Option<impl IntoElement> {
    b.model.active.filter(|k| b.pages.get(k).is_some_and(|p| p.article))?;
    Some(chip_button("reader", Icon::BookOpen, true).tooltip(crate::shortcuts::tip("Reader", &crate::reader::ToggleReader)).on_click(cx.listener(
        |b, _, window, cx| {
            cx.stop_propagation();
            b.toggle_reader(window, cx);
        },
    )))
}

fn mute_button(b: &Browser, cx: &mut Context<Browser>) -> Option<impl IntoElement> {
    let key = b.model.active.filter(|k| b.tab_muted(*k))?;
    Some(chip_button("unmute", Icon::VolumeX, true).occlude().tooltip(tooltip("Unmute Site", None)).on_click(cx.listener(move |b, _, _, cx| {
        cx.stop_propagation();
        b.toggle_site_muted(key, cx);
    })))
}

/// The active page's engine, unless it is WebKit and WebKit is the default.
fn engine_badge(b: &Browser) -> Option<Div> {
    let engine = b.tab_web_engine(b.model.active?);
    (engine != WebEngine::WebKit || b.core.default_web_engine.get() != WebEngine::WebKit)
        .then(|| div().px_1p5().text_size(px(TEXT_SMALL)).text_color(Secure::muted()).child(crate::tabs::web_engine_name(engine)))
}

fn origin_chip(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let site = site(b);
    let (text, color) = address_text(&site);
    div()
        .id("origin")
        .occlude()
        .group("origin")
        .w_full()
        .max_w(px(560.))
        .h(px(30.))
        .px_1()
        .flex()
        .items_center()
        .rounded(px(Theme::radius_row()))
        .bg(Secure::chip())
        .border_1()
        .border_color(Secure::hairline())
        .font_family(SYSTEM_FONT)
        .cursor_pointer()
        .hover(|s| s.bg(Secure::hover()))
        .active(|s| s.bg(Secure::pressed()))
        .when(b.inline_editing(), |d| d.bg(Secure::pressed()))
        .on_click(cx.listener(|b, _, window, cx| open_address(b, window, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(|b, _, w, cx| b.address_menu(w, cx)))
        .on_scroll_wheel(cx.listener(|b, e, w, cx| b.toolbar_swipe(e, true, w, cx)))
        .relative()
        .child(measure(b.address.field.clone()))
        // Both sides share the spare room equally, so the host stays centred in the chip.
        .child(div().flex_1().flex().items_center().pl_0p5().map(|d| match site.controls {
            true => d.child(shield_button(b, &site, cx)),
            false => d.child(div().w(px(28.)).flex().justify_center().child(icon(Icon::Search, ICON_SMALL, Secure::faint()))),
        }))
        .child(tail_first(div(), text, color))
        .child(
            div()
                .flex_1()
                .min_w(px(28.))
                .flex()
                .items_center()
                .justify_end()
                .children(reader_button(b, cx))
                .children(mute_button(b, cx))
                .children(engine_badge(b))
                .when(site.controls, |d| d.child(bookmark_button(b, &site, cx))),
        )
}

fn render_compact(b: &Browser, cx: &mut Context<Browser>) -> Div {
    let page = b.model.active.map(|k| b.page(k)).unwrap_or_default();
    let site = site(b);
    let (text, color) = address_text(&site);
    let title = b.model.active_tab().map(|t| bounded_title(&t.title)).filter(|t| !t.is_empty() && site.controls);
    div()
        .gap(px(2.))
        .child(navigation(b, &page, cx))
        .child(
            div().flex_1().min_w_0().relative().flex().child(measure(b.address.slot.clone())).child(
                div()
                    .id("address")
                    .occlude()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .mx_1()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded(px(Theme::radius_row()))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::hover()))
                    .active(|s| s.bg(Theme::pressed()))
                    .on_click(cx.listener(|b, _, window, cx| open_address(b, window, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(|b, _, w, cx| b.address_menu(w, cx)))
                    .on_scroll_wheel(cx.listener(|b, e, w, cx| b.toolbar_swipe(e, true, w, cx)))
                    .relative()
                    .child(measure(b.address.field.clone()))
                    .when(site.controls, |d| d.child(shield_button(b, &site, cx)))
                    .child(tail_first(div().flex_shrink_0().max_w_full(), text, color))
                    .when_some(title, |d, title| {
                        d.child(div().flex_none().text_color(Theme::faint()).child("—")).child(
                            div().min_w_0().max_w(px(360.)).overflow_hidden().text_ellipsis().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(title),
                        )
                    }),
            ),
        )
        .children(reader_button(b, cx))
        .children(mute_button(b, cx))
        .children(engine_badge(b))
        .when(site.controls, |d| d.child(bookmark_button(b, &site, cx)))
        .when(b.devtools_on(), |d| d.child(devtools(b, cx)))
        .child(crate::extensions::buttons(b, cx))
        .child(page_actions(b))
        .children(app_menu_button(b, cx))
}

/// A click on the resting address opens the palette; while it is open, the click keeps it.
fn open_address(b: &mut Browser, window: &mut gpui::Window, cx: &mut Context<Browser>) {
    if !b.inline_editing() {
        b.open_command(CommandMode::Current, window, cx);
    }
}

/// The address field the palette shows: out of sight in the toolbar, where it keeps the
/// keyboard focus and the text input of the main window.
fn hidden_field(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_0()
        .overflow_hidden()
        .on_key_down(cx.listener(|b, e: &KeyDownEvent, _, cx| b.suggestion_key(e, cx)))
        .child(b.command.clone())
}

/// Clips the host from the left, so a host wider than its box still shows its registrable domain.
pub(crate) fn tail_first(clip: Div, text: impl IntoElement, color: gpui::Rgba) -> Div {
    clip.min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .flex()
        .flex_row_reverse()
        .child(div().flex_1())
        .child(div().flex_none().text_color(color).child(text))
        .child(div().flex_1())
}

fn devtools(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let preset = b.responsive_preset();
    let has_page = b.model.active.is_some();
    div()
        .occlude()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(
            div()
                .id("responsive")
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .gap_1p5()
                .rounded(px(Theme::radius_row()))
                .cursor_pointer()
                .text_size(px(TEXT_SMALL))
                .text_color(if preset.is_some() { Theme::accent() } else { Theme::muted() })
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .tooltip(tooltip("Responsive Size", None))
                .child(icon(Icon::Smartphone, 14., if preset.is_some() { Theme::accent() } else { Theme::muted() }))
                .when_some(preset, |d, p| d.child(SharedString::from(format!("{} {}×{}", p.name, p.width, p.height))))
                .on_click(cx.listener(|b, _, _, cx| b.cycle_responsive(cx))),
        )
        .child(
            icon_button("clear-site-data", Icon::Trash, has_page)
                .tooltip(tooltip("Clear Site Data", None))
                .on_click(cx.listener(|b, _, _, cx| b.clear_site_data(cx))),
        )
}

#[cfg(test)]
mod tests {
    use super::{address_text, host_tail, site_of};
    use url::Url;

    /// Security UI draws only from `Secure` and the colours that delegate to it (threat model
    /// S29), so no theme reaches it; layout values from `Theme` are fine.
    #[test]
    fn security_ui_uses_no_themable_colour() {
        let allowed = ["radius_", "elevation(", "warning(", "danger(", "success(", "on_accent("];
        let sources = [
            (include_str!("toolbar.rs"), ["fn address_text(", "fn shield_button(", "fn origin_chip("].as_slice()),
            (include_str!("content.rs"), ["fn floating(", "fn banner(", "fn prompt_card(", "fn start_shield("].as_slice()),
            (include_str!("blocking.rs"), ["pub fn render_blocked("].as_slice()),
            (include_str!("link_peek.rs"), ["fn peek_origin("].as_slice()),
            (include_str!("settings/appearance.rs"), ["fn badge_line("].as_slice()),
        ];
        let mut checked = 0;
        for (source, functions) in sources {
            let source = source.replace("\r\n", "\n");
            for name in functions {
                let start = source.find(name).unwrap_or_else(|| panic!("{name} not found"));
                let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
                for call in body.split("Theme::").skip(1) {
                    assert!(allowed.iter().any(|a| call.starts_with(a)), "{name} calls Theme::{}", &call[..call.len().min(24)]);
                    checked += 1;
                }
                assert!(!body.contains(" surface(") && !body.contains(" button("), "{name} uses a themable control");
                checked += 1;
            }
        }
        assert!(checked > 7);
    }

    #[test]
    fn pages_without_a_plain_host_still_name_their_origin() {
        let shown = |url: &str| address_text(&site_of(Url::parse(url).ok())).0;
        assert_eq!(shown("blob:https://www.evil.test/0b1c"), "evil.test");
        assert_eq!(shown("blob:http://evil.test/0b1c"), "Not Secure — evil.test");
        assert_eq!(shown("about:blank#bank.test"), "about:blank");
        assert_eq!(shown("data:text/html,bank.test"), "data:");
        assert_eq!(address_text(&site_of(None)).0, "Search or enter address");
    }

    #[test]
    fn the_other_shells_show_the_same_address() {
        let long = format!("https://{}.attacker.example/", "a".repeat(60));
        for url in [
            "https://www.example.com/a",
            "http://localhost:8765/x",
            "https://example.com./",
            "blob:http://evil.test/0b1c",
            "about:blank",
            "about:srcdoc",
            "data:text/html,x",
            "mailto:a@b.test",
            long.as_str(),
        ] {
            assert_eq!(address_text(&site_of(Url::parse(url).ok())).0, browser_core::command::display_host(url), "{url}");
        }
    }

    #[test]
    fn long_hosts_keep_their_registrable_domain() {
        assert_eq!(host_tail("example.com", 40), "example.com");
        let spoof = "accounts.google.com.login.verify.session.attacker.example";
        let shown = host_tail(spoof, 20);
        assert_eq!(shown.chars().count(), 20);
        assert!(shown.starts_with('…') && shown.ends_with("attacker.example"));
    }
}
