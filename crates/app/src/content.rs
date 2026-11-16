use std::time::Instant;

use gpui::{
    AnyElement, Context, Focusable, FontWeight, HighlightStyle, InteractiveElement, IntoElement, ParentElement, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledText, Window, canvas, div, img, prelude::FluentBuilder, px, relative,
};
use ui::controls::{button, field, icon_button, list_row, secure_button, secure_ghost_button, secure_surface, surface, tooltip};
use ui::icon::{ICON_MEDIUM, ICON_SMALL, Icon, MARK, icon};
use ui::motion::{EXIT, FADE, pop_in, pop_out, progress};
use ui::theme::{Elevation, Secure, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

use engine::PromptKind;

use crate::browser::{Browser, CommandMode, Suggestion, Target};
use crate::prefs::Pref;
use crate::profiles::profile_dot;
use crate::sidebar::site_icon;
use crate::toolbar::shield_badge;

fn floating() -> gpui::Div {
    secure_surface(Theme::radius_panel()).mb_2().flex_none().shadow(Elevation::Floating.shadow()).text_size(px(TEXT))
}

const BANNER_HEIGHT: f32 = 42.;

type Ghost = (&'static str, SharedString, Rgba);

#[derive(Default)]
pub struct Floating {
    shown: Vec<Ghost>,
    leaving: Vec<(usize, Ghost, Instant)>,
}

impl Floating {
    fn update(&mut self, present: Vec<Ghost>, animate: bool) {
        let gone = self.shown.iter().enumerate().filter(|(_, g)| !present.iter().any(|p| p.0 == g.0));
        let gone: Vec<_> = gone.map(|(ix, g)| (ix, g.clone(), Instant::now())).collect();
        self.leaving.extend(gone.into_iter().filter(|_| animate));
        self.leaving.retain(|(_, g, at)| at.elapsed() < EXIT && !present.iter().any(|p| p.0 == g.0));
        self.shown = present;
    }
}

impl Browser {
    pub fn sidebar_shown(&self) -> f32 {
        let t = self.sidebar_toggled.filter(|_| !self.pref(Pref::ReduceMotion)).map_or(1., |at| progress(at, FADE));
        if self.sidebar_open { t } else { 1. - t }
    }

    pub fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inline_editing() {
            return self.close_command(window, cx);
        }
        let animate = !self.pref(Pref::ReduceMotion) && (self.profile_editor.is_some() || self.model.active.is_some());
        if !animate || self.overlay_exit.is_some() {
            let back = self.launcher_from.filter(|k| self.model.active.is_none() && self.model.tab(*k).is_some());
            self.close_command(window, cx);
            if let Some(key) = back {
                self.activate(key, window, cx);
            }
            return;
        }
        let at = Instant::now();
        self.overlay_exit = Some(at);
        window.focus(&self.focus_handle(cx));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(EXIT).await;
            this.update_in(cx, |b, window, cx| {
                if b.overlay_exit == Some(at) {
                    b.close_command(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn settle_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_suggestions(window, cx);
        self.sync_toast(window, cx);
        let fade_done = self.page_fade.as_ref().is_some_and(|(_, at)| at.elapsed() >= FADE);
        if fade_done || self.overlay_open() || self.model.active.is_none() || self.active_json().is_some() {
            let active = self.active_view();
            if let Some((outgoing, _)) = self.page_fade.take().filter(|(v, _)| !active.as_ref().is_some_and(|a| std::rc::Rc::ptr_eq(a, v))) {
                outgoing.set_visible(false);
            }
        }
        let running = |at: Option<Instant>, d| at.is_some_and(|at: Instant| at.elapsed() < d);
        let sidebar = running(self.sidebar_toggled, FADE) && !self.pref(Pref::ReduceMotion);
        if self.settle_slide()
            || sidebar
            || self.page_fade.is_some()
            || running(self.overlay_exit, EXIT)
            || !self.floating.leaving.is_empty()
            || self.address_moving()
        {
            window.request_animation_frame();
        }
    }
}

fn banner(text: impl Into<SharedString>) -> gpui::Div {
    floating()
        .min_h(px(BANNER_HEIGHT))
        .pl_3p5()
        .pr_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().min_w_0().line_clamp(3).text_ellipsis().child(text.into()))
}

pub fn render(b: &mut Browser, window: &mut Window, cx: &mut Context<Browser>) -> AnyElement {
    b.settle_motion(window, cx);
    b.settle_split();
    let reduce = b.pref(Pref::ReduceMotion);
    let page = b.model.active.map(|k| b.page(k)).unwrap_or_default();
    let error = page.error;
    let mut items: Vec<(Ghost, gpui::Div)> = Vec::new();
    let mut push = |id: &'static str, text: String, color: Rgba, card: gpui::Div| items.push(((id, text.into(), color), card));

    let rules_warning = engine::builtin_rules_failed()
        .then(|| "Tarn's built-in protections failed to load: local-network blocking and HTTPS upgrades are off until Tarn restarts.".to_owned());
    for (id, warning) in [("banner-storage", &b.core.storage_warning), ("banner-window", &b.warning), ("banner-rules", &rules_warning)] {
        if let Some(warning) = warning {
            push(id, warning.clone(), Theme::warning(), banner(warning.clone()).text_color(Theme::warning()));
        }
    }
    if let Some(pending) = b.pending_external.as_ref().filter(|p| p.tab.is_none_or(|t| b.model.active == Some(t))) {
        let url = ui::text::bounded_url(pending.url.as_str());
        let text = match (&pending.tab, &pending.origin) {
            (None, _) => format!("Open “{url}” in another app?"),
            (Some(_), Some(origin)) => format!("{} wants to open “{url}” in another app", ui::text::bounded_origin(origin)),
            (Some(_), None) => format!("A page wants to open “{url}” in another app"),
        };
        push(
            "banner-external",
            text.clone(),
            Secure::text(),
            banner(text)
                .child(secure_button("ext-open", "Open", true).on_click(cx.listener(|b, _, _, cx| b.resolve_external(true, cx))))
                .child(secure_button("ext-cancel", "Cancel", false).on_click(cx.listener(|b, _, _, cx| b.resolve_external(false, cx)))),
        );
    }
    match b.current_download().map(|d| &d.result) {
        Some(Ok((path, quarantined))) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let reveal = path.clone();
            let executable = engine::downloads::is_dangerous(&name);
            let name = ui::text::bounded_title(&name);
            let dangerous = executable || !quarantined;
            let text = match (quarantined, executable) {
                (false, _) => format!(
                    "Downloaded “{name}”, but {} could not be set. Do not open it unless you trust it.",
                    if cfg!(windows) { "the mark of the web" } else { "macOS quarantine" }
                ),
                (true, true) => format!("Downloaded “{name}”. This file type can run code; open it only if you trust the site."),
                (true, false) => format!("Downloaded “{name}”"),
            };
            let color = if dangerous { Secure::warning() } else { Secure::text() };
            push(
                "banner-download",
                text.clone(),
                color,
                banner(text)
                    .text_color(color)
                    .child(secure_button("dl-show", ui::controls::REVEAL, true).on_click(move |_, _, _| engine::reveal_in_finder(&reveal)))
                    .child(secure_button("dl-dismiss", "Dismiss", false).on_click(cx.listener(|b, _, _, cx| {
                        b.download = None;
                        cx.notify();
                    }))),
            );
        }
        Some(Err(message)) => {
            let text = format!("Download failed: {}", ui::text::bounded_title(message));
            push(
                "banner-download",
                text.clone(),
                Theme::danger(),
                banner(text).text_color(Theme::danger()).child(secure_button("dl-dismiss", "Dismiss", false).on_click(cx.listener(|b, _, _, cx| {
                    b.download = None;
                    cx.notify();
                }))),
            );
        }
        None => {}
    }
    if let Some(message) = page.notice {
        let text = ui::text::bounded_title(&message);
        let offer = page.offer.as_ref().map(|o| match o {
            crate::blocking::Offer::Open(_) => "Open Anyway",
            crate::blocking::Offer::NewTab(_) => "Open Once",
            crate::blocking::Offer::WebKit(_) => "Open in WebKit",
            #[cfg(feature = "chromium")]
            crate::blocking::Offer::SwitchToWebKit => "Open in WebKit",
            crate::blocking::Offer::Reload => "Turn On",
            #[cfg(feature = "translation")]
            crate::blocking::Offer::Translate(..) => "Translate",
        });
        push(
            "banner-notice",
            text.clone(),
            Theme::warning(),
            banner(text)
                .text_color(Theme::warning())
                .when_some(offer, |d, label| d.child(secure_button("notice-offer", label, false).on_click(cx.listener(|b, _, w, cx| b.take_offer(w, cx)))))
                .child(secure_button("notice-dismiss", "Dismiss", false).on_click(cx.listener(|b, _, _, cx| b.dismiss_error(cx)))),
        );
    }
    if let Some(message) = error {
        let text = format!("This page could not be loaded: {}", ui::text::bounded_title(&message));
        push(
            "banner-error",
            text.clone(),
            Theme::danger(),
            banner(text)
                .text_color(Theme::danger())
                .child(secure_button("err-retry", "Retry", true).on_click(cx.listener(|b, _, _, cx| {
                    b.dismiss_error(cx);
                    b.reload(cx);
                })))
                .child(secure_button("err-dismiss", "Dismiss", false).on_click(cx.listener(|b, _, _, cx| b.dismiss_error(cx)))),
        );
    }

    if !b.overlay_open() {
        if let Some((title, card)) = prompt_card(b, window, cx) {
            push("prompt-card", title, Theme::text(), card);
        }
        if b.find_open && b.active_json().is_none() {
            let focused = b.find_input.focus_handle(cx).is_focused(window);
            push(
                "find-bar",
                String::new(),
                Theme::text(),
                div().mb_2().flex_none().flex().justify_end().child(
                    surface(12.)
                        .w(px(400.))
                        .flex_none()
                        .shadow(Elevation::Floating.shadow())
                        .when(focused, |d| d.border_color(Theme::accent()))
                        .text_size(px(TEXT))
                        .h(px(40.))
                        .pl_3()
                        .pr_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(Icon::Search, 14., Theme::muted()))
                        .child(div().flex_1().min_w_0().overflow_hidden().child(b.find_input.clone()))
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(TEXT_SMALL))
                                .text_color(if b.find_count == Some(0) { Theme::danger() } else { Theme::faint() })
                                .child(browser_core::find::count_label(b.find_count)),
                        )
                        .child(div().w(px(1.)).h(px(18.)).flex_none().bg(Theme::separator()))
                        .child(
                            icon_button("find-prev", Icon::ChevronUp, true)
                                .tooltip(crate::shortcuts::tip("Previous", &crate::FindPrevious))
                                .on_click(cx.listener(|b, _, _, cx| b.find(true, cx))),
                        )
                        .child(
                            icon_button("find-next", Icon::ChevronDown, true)
                                .tooltip(crate::shortcuts::tip("Next", &crate::FindNext))
                                .on_click(cx.listener(|b, _, _, cx| b.find(false, cx))),
                        )
                        .child(
                            icon_button("find-close", Icon::Close, true)
                                .tooltip(tooltip("Done", Some("Esc".into())))
                                .on_click(cx.listener(|b, _, w, cx| b.close_find(w, cx))),
                        ),
                ),
            );
        }
    }

    let present: Vec<Ghost> = items.iter().map(|(g, _)| g.clone()).collect();
    if !present.iter().map(|g| g.0).eq(b.floating.shown.iter().map(|g| g.0)) {
        b.layout_changed();
    }
    b.floating.update(present, !reduce);
    let mut cards: Vec<AnyElement> = items.into_iter().map(|((id, ..), card)| pop_in(card, id, reduce)).collect();
    for (ix, (_, text, color), at) in &b.floating.leaving {
        let t = progress(*at, EXIT);
        let ghost = banner(text.clone()).text_color(*color).min_h(px(0.)).h(px(BANNER_HEIGHT * (1. - t))).mb(px(8. * (1. - t))).opacity(1. - t);
        cards.insert((*ix).min(cards.len()), ghost.into_any_element());
    }
    let panel = div().flex_1().min_h_0().flex().flex_col().p(px(8.)).pt_0().pl(px(8. - 2. * b.sidebar_shown())).text_color(Theme::text()).children(cards);

    let exit = b.overlay_exit.map(|at| progress(at, EXIT));
    let leaving = |body: AnyElement| match exit {
        Some(t) => pop_out(div().flex_1().flex().flex_col().child(body), t).into_any_element(),
        None => body,
    };

    let body: AnyElement = if let Some(editor) = crate::profiles::render_editor(b, window, cx) {
        leaving(editor)
    } else if b.shortcuts_open {
        leaving(pop_in(div().flex_1().flex().flex_col().child(crate::shortcuts::render(cx)), "shortcuts", reduce))
    } else if let Some(reader) = crate::reader::render(b, cx) {
        leaving(pop_in(div().flex_1().min_h_0().flex().flex_col().child(reader), "reader", reduce))
    } else if (b.command_mode.is_some() && !b.inline_editing()) || b.model.active.is_none() {
        leaving(start(b, reduce, cx))
    } else if let Some(split) = crate::split::render(b, window, cx) {
        split
    } else if let Some(blocked) = &page.blocked {
        if let Some(view) = b.active_view() {
            view.set_frame(0., 0., 0., 0.);
        }
        crate::blocking::render_blocked(blocked, cx)
    } else if let Some((key, json)) = b.active_json() {
        if let Some(view) = b.active_view() {
            view.set_frame(0., 0., 0., 0.);
        }
        crate::devtools::render_json(key, json, cx)
    } else {
        let view = b.active_view();
        let preset = b.responsive_preset();
        let alpha = b.page_fade.as_ref().map_or(1., |(_, at)| progress(*at, FADE));
        let page = canvas(
            move |bounds, _, _| {
                let Some(view) = view else { return };
                view.set_alpha(alpha.into());
                let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                match preset {
                    Some(preset) => {
                        let fit = devkit::responsive::fit(&preset, w, h);
                        view.set_frame((x + fit.x).into(), (y + fit.y).into(), fit.width.into(), fit.height.into());
                        // WebKit's page zoom stops at 0.3; below that the CSS viewport is narrower than the preset.
                        let scale = f64::from(fit.scale).max(engine::MIN_ZOOM);
                        if (view.zoom() - scale).abs() > 0.001 {
                            view.set_zoom(scale);
                        }
                    }
                    None => view.set_frame(x.into(), y.into(), w.into(), h.into()),
                }
            },
            |_, _, _, _| {},
        )
        .size_full();
        let frame = div()
            .flex_1()
            .min_h_0()
            .rounded(px(Theme::radius_page()))
            .bg(Theme::page())
            .border_1()
            .border_color(Theme::hairline())
            .shadow(Elevation::Page.shadow());
        frame.child(page).into_any_element()
    };

    panel.child(body).into_any_element()
}

const MAX_PROMPT_MESSAGE: usize = 600;

/// The prompt's kind as `browser_core::glyphs` and `browser_core::prompts` name it.
fn kind_name(kind: &PromptKind) -> &'static str {
    match kind {
        PromptKind::Alert | PromptKind::Confirm | PromptKind::Text { .. } => "confirm",
        PromptKind::Camera => "camera",
        PromptKind::Microphone => "microphone",
        PromptKind::CameraAndMicrophone => "camera+microphone",
        PromptKind::Credentials { .. } => "credentials",
        PromptKind::ExtensionAccess { .. } => "extension",
        PromptKind::MaliciousDownload { .. } => "malicious download",
    }
}

fn prompt_card(b: &Browser, window: &Window, cx: &mut Context<Browser>) -> Option<(String, gpui::Div)> {
    let prompt = b.active_prompt()?;
    let origin = match &prompt.top_origin {
        Some(top) => format!("{} (embedded in {})", ui::text::bounded_origin(&prompt.origin), ui::text::bounded_origin(top)),
        None => ui::text::bounded_origin(&prompt.origin),
    };
    let mut message: String = prompt.message.chars().filter(|c| *c == '\n' || (!c.is_control() && !ui::text::is_bidi(*c))).take(MAX_PROMPT_MESSAGE).collect();
    if prompt.message.chars().count() > MAX_PROMPT_MESSAGE {
        message.push('…');
    }
    let (lead, ask) = match &prompt.kind {
        PromptKind::Alert | PromptKind::Confirm | PromptKind::Text { .. } => (origin, "says".to_owned()),
        PromptKind::Camera => (origin, "wants to use your camera".into()),
        PromptKind::Microphone => (origin, "wants to use your microphone".into()),
        PromptKind::CameraAndMicrophone => (origin, "wants to use your camera and microphone".into()),
        PromptKind::Credentials { realm } => {
            (origin, format!("asks you to sign in{}", if realm.is_empty() { String::new() } else { format!(" ({})", ui::text::bounded_title(realm)) }))
        }
        PromptKind::ExtensionAccess { extension, key } => (
            format!("The extension \u{201C}{}\u{201D}", crate::extensions::shown(extension)),
            format!("({}) wants to read and change {origin}", b.extension_badge(key)),
        ),
        PromptKind::MaliciousDownload { file } => (format!("Keep \u{201C}{}\u{201D}", ui::text::bounded_title(file)), format!("from {origin}?")),
    };
    let title = format!("{lead} {ask}");
    let kind = kind_name(&prompt.kind);
    let detail = if message.is_empty() { browser_core::prompts::meaning(kind).map(str::to_owned) } else { Some(message) };
    let remember_allowed = matches!(prompt.kind, PromptKind::ExtensionAccess { .. })
        || b.model.active.is_some_and(|key| {
            let page = prompt.top_origin.as_deref().unwrap_or(&prompt.origin);
            b.level_for_tab(key, page).max(b.level_for_tab(key, &prompt.origin)) == shield::ShieldLevel::Standard
        });
    let field = |input: gpui::Entity<ui::text_input::TextInput>| field(input.focus_handle(cx).is_focused(window)).w(px(320.)).py_0p5().child(input);
    let warn = matches!(prompt.kind, PromptKind::MaliciousDownload { .. } | PromptKind::Alert | PromptKind::Confirm | PromptKind::Text { .. });
    let glyph = Icon::named(browser_core::glyphs::glyph(kind)).unwrap_or(Icon::Info);
    let tile = div().size(px(30.)).flex_none().flex().items_center().justify_center().rounded(px(Theme::radius_row())).bg(Secure::control()).child(icon(
        glyph,
        16.,
        if warn { Secure::warning() } else { Secure::accent() },
    ));
    let answer = |remember: bool, accept: bool| {
        cx.listener(move |b: &mut Browser, _, w, cx| {
            if accept && b.prompt_guarded() {
                return;
            }
            b.remember_permission = remember;
            b.answer_prompt(accept, false, w, cx);
        })
    };
    let buttons = div().flex().flex_none().gap_2().items_center();
    let buttons = match &prompt.kind {
        k if k.is_permission() => buttons
            .child(secure_ghost_button("prompt-silence", "Block requests from this page").on_click(cx.listener(|b, _, w, cx| {
                b.remember_permission = false;
                b.answer_prompt(false, true, w, cx);
            })))
            .child(secure_ghost_button("prompt-decline", "Deny").on_click(answer(false, false)))
            .child(secure_button("prompt-accept", "Allow Once", !remember_allowed).on_click(answer(false, true)))
            .when(remember_allowed, |d| d.child(secure_button("prompt-remember", "Always Allow", true).on_click(answer(true, true)))),
        PromptKind::Alert => buttons.child(secure_button("prompt-accept", "OK", true).on_click(answer(false, true))),
        PromptKind::MaliciousDownload { .. } => buttons
            .child(secure_button("prompt-accept", "Keep Anyway", false).text_color(Secure::danger()).on_click(answer(false, true)))
            .child(secure_button("prompt-decline", "Cancel Download", true).on_click(answer(false, false))),
        PromptKind::Credentials { .. } => buttons
            .child(secure_ghost_button("prompt-decline", "Cancel").on_click(answer(false, false)))
            .child(secure_button("prompt-accept", "Sign In", true).on_click(answer(false, true))),
        _ => buttons
            .child(secure_ghost_button("prompt-decline", "Cancel").on_click(answer(false, false)))
            .child(secure_button("prompt-accept", "OK", true).on_click(answer(false, true))),
    };
    let card = floating()
        .px_3()
        .py_2p5()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(tile)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(div().child(div().flex().flex_wrap().gap_x_1().child(div().font_weight(FontWeight::BOLD).child(lead)).child(ask)))
                        .when_some(detail, |d, detail| d.child(div().text_size(px(TEXT_SMALL)).text_color(Secure::muted()).child(detail))),
                )
                .child(buttons),
        )
        .when(prompt.insecure, |d| d.child(div().text_color(Secure::warning()).child("This connection is not secure: the password would be sent unencrypted.")))
        .when(matches!(prompt.kind, PromptKind::Text { .. }), |d| d.child(field(b.prompt_input.clone())))
        .when(matches!(prompt.kind, PromptKind::Credentials { .. }), |d| d.child(field(b.auth_user.clone())).child(field(b.auth_pass.clone())));
    Some((title, card))
}

fn suggestion_icon(b: &Browser, target: &Target) -> AnyElement {
    let url = match target {
        Target::Tab(key) | Target::Find(key, _) => b.model.tab(*key).map(|t| t.url.clone()),
        Target::Url(u) | Target::Input(ui::command::Input::Navigate(u)) => Some(u.to_string()),
        Target::Input(_) | Target::Action(_) => None,
    };
    let glyph = if matches!(target, Target::Input(ui::command::Input::Command(_)) | Target::Action(_)) { Icon::Command } else { Icon::Search };
    match url {
        Some(url) => site_icon(b, &url, 18.),
        None => div().w(px(18.)).flex().justify_center().child(icon(glyph, 15., Theme::muted())).into_any_element(),
    }
}

pub const SUGGESTION_ROW: f32 = ui::theme::ROW_HEIGHT;
/// Rows a suggestion list shows before it scrolls.
pub const SHOWN_SUGGESTIONS: usize = 10;

pub fn suggestion_row(b: &Browser, ix: usize, s: &Suggestion) -> Stateful<gpui::Div> {
    let selected = ix == b.selected;
    let rejected = matches!(s.target, Target::Input(ui::command::Input::Rejected));
    list_row(("suggestion", ix), selected)
        .mx_1p5()
        .gap_3()
        .child(suggestion_icon(b, &s.target))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(if rejected { Theme::danger() } else { Theme::text() })
                .child(s.label.clone()),
        )
        .child(
            div()
                .max_w(px(260.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(TEXT_SMALL))
                .text_color(Theme::muted())
                .child(s.detail.clone()),
        )
}

fn start(b: &Browser, reduce: bool, cx: &mut Context<Browser>) -> AnyElement {
    let typing = b.command_mode.is_some() && !b.inline_editing();
    let input: AnyElement =
        if typing { b.command.clone().into_any_element() } else { div().text_color(Theme::faint()).child("Search or enter address").into_any_element() };
    let space = b.model.current_space().map(|s| SharedString::from(ui::text::bounded_title(&s.space.name))).unwrap_or_default();
    let (level, color) = b.current_loaded().map(|rt| (rt.profile.shield_level, rt.profile.color.clone())).unwrap_or_default();
    let chip = |content: gpui::Div| {
        content
            .h(px(28.))
            .px_2p5()
            .flex()
            .items_center()
            .gap_1p5()
            .rounded_full()
            .border_1()
            .border_color(Theme::surface_edge())
            .text_size(px(TEXT_SMALL))
            .text_color(Theme::muted())
    };
    let suggestions = div()
        .id("start-suggestions")
        .track_scroll(&b.suggestions_scroll)
        .max_h(px(SUGGESTION_ROW * SHOWN_SUGGESTIONS as f32 + 12.))
        .overflow_y_scroll()
        .py_1p5()
        .border_t_1()
        .border_color(Theme::hairline())
        .children(
            b.suggestions
                .iter()
                .enumerate()
                .map(|(ix, s)| suggestion_row(b, ix, s).on_click(cx.listener(move |b, _, window, cx| b.submit_command(ix, window, cx)))),
        );
    let card = surface(Theme::radius_card())
        .id("start-card")
        .border_color(Theme::accent_soft())
        .w_full()
        .flex()
        .flex_col()
        .shadow(Elevation::Modal.shadow())
        .text_size(px(TEXT))
        .when(!typing, |d| d.cursor_pointer().on_click(cx.listener(|b, _, w, cx| b.open_command(CommandMode::NewTab, w, cx))))
        .child(
            div()
                .h(px(SEARCH_ROW))
                .px_4()
                .flex()
                .items_center()
                .gap_3()
                .text_size(px(TEXT_TITLE))
                .on_key_down(cx.listener(|b, e: &gpui::KeyDownEvent, _, cx| b.suggestion_key(e, cx)))
                .child(icon(Icon::Search, ICON_MEDIUM, Theme::accent()))
                .child(div().flex_1().overflow_hidden().child(input)),
        )
        .when(typing && !b.suggestions.is_empty(), |d| d.child(suggestions))
        .child(
            div()
                .px_3()
                .pb_3()
                .when(typing && !b.suggestions.is_empty(), |d| d.pt_1())
                .flex()
                .items_center()
                .justify_between()
                .child(chip(div()).child(profile_dot(&color, 8.)).child(icon(Icon::Folder, 12., Theme::muted())).child(space))
                .child(start_shield(level)),
        );
    let servers = b.devtools_on().then(|| {
        let ports = b.local_servers.clone().unwrap_or_default();
        surface(Theme::radius_panel())
            .occlude()
            .w(px(680.))
            .max_w_full()
            .p_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_size(px(TEXT_SMALL))
            .text_color(Theme::muted())
            .child(icon(Icon::Code, ICON_SMALL, Theme::muted()))
            .child(match &b.local_servers {
                None => "Looking for local servers…",
                Some(p) if p.is_empty() => "No local servers on common dev ports",
                Some(_) => "Local servers",
            })
            .children(ports.into_iter().map(|port| {
                chip(div())
                    .id(("server", port as usize))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::hover()))
                    .active(|s| s.bg(Theme::pressed()))
                    .child(SharedString::from(format!("localhost:{port}")))
                    .on_click(cx.listener(move |b, _, w, cx| b.open_local_server(port, w, cx)))
            }))
            .child(button("servers-refresh", "Refresh", false).on_click(cx.listener(|b, _, _, cx| b.refresh_local_servers(cx))))
    });
    let column = div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(relative(0.22))
        .px_4()
        .gap_7()
        .child(img(MARK).w(px(64.)).h(px(64.)))
        .child(pop_in(div().relative().w(px(680.)).max_w_full().child(b.search_fx.clone()).child(card.occlude()), ("start-card", typing as usize), reduce))
        .children(servers);
    let left = (b.sidebar_width + crate::sidebar::SIDEBAR_MARGIN) * b.sidebar_shown();
    b.fog.update(cx, |fog, _| (fog.reduce, fog.left) = (reduce, left));
    b.search_fx.update(cx, |fx, _| fx.reduce = reduce);
    div()
        .flex_1()
        .relative()
        .when(typing, |d| d.on_mouse_down(gpui::MouseButton::Left, cx.listener(|b, _, w, cx| b.dismiss_overlay(w, cx))))
        .child(b.fog.clone())
        .child(column)
        .into_any_element()
}

#[cfg(target_os = "macos")]
const FOG_FONT: (&str, f32) = ("Menlo", 6.02);
#[cfg(windows)]
const FOG_FONT: (&str, f32) = ("Consolas", 5.5);
const FOG_LINE: f32 = 11.;

type FogLine = (SharedString, Vec<(std::ops::Range<usize>, HighlightStyle)>);
/// Columns, rows, animation step and theme generation of a cached frame.
type FogKey = (usize, usize, u32, u32);

/// The ASCII cloud bank behind the start card, redrawn every `fog::FRAME`; its own view, so
/// windows that observe the `Browser` are not notified. One still frame while motion is reduced
/// or the theme asks for it; none when the theme turns it off. Paused unless its window is the
/// active one of the active app, so a frame redraws one window only.
#[derive(Default)]
pub struct Fog {
    pub(crate) reduce: bool,
    /// The window's width left of the content area, which the fog does not cover.
    pub(crate) left: f32,
    started: Option<Instant>,
    ticking: bool,
    frame: Option<(FogKey, Vec<FogLine>)>,
}

impl gpui::Render for Fog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use browser_core::fog;
        let size = window.viewport_size();
        let (cols, rows) = (((f32::from(size.width) - self.left).max(0.) / FOG_FONT.1) as usize, (f32::from(size.height) / FOG_LINE) as usize);
        let mode = Theme::fog_mode();
        let still = self.reduce || mode != browser_core::theme::Fog::On;
        let active = engine::app_active() && window.is_window_active();
        let step = match (still, active) {
            (true, _) => 0,
            (false, true) => fog::step(self.started.get_or_insert_with(Instant::now).elapsed()),
            (false, false) => self.frame.as_ref().map_or(0, |(key, _)| key.2),
        };
        if !still && !self.ticking && active {
            self.ticking = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs_f32(fog::FRAME)).await;
                this.update(cx, |fog, cx| {
                    fog.ticking = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let key = (cols, rows, step, Theme::generation());
        if mode == browser_core::theme::Fog::Off {
            self.frame = None;
        } else if self.frame.as_ref().is_none_or(|(k, _)| *k != key) {
            let rows = fog::frame(cols, rows, step).into_iter().map(|row| {
                let shades =
                    row.shades.into_iter().map(|(range, shade)| (range, HighlightStyle { color: Some(Theme::fog(shade).into()), ..Default::default() }));
                (SharedString::from(row.text), shades.collect())
            });
            self.frame = Some((key, rows.collect()));
        }
        let lines = self.frame.iter().flat_map(|(_, rows)| rows).map(|(text, shades)| StyledText::new(text.clone()).with_highlights(shades.clone()));
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .whitespace_nowrap()
            .font_family(FOG_FONT.0)
            .text_size(px(10.))
            .line_height(px(FOG_LINE))
            .children(lines)
    }
}

/// The start card's Shield level, in the fixed security palette (threat model S29).
fn start_shield(level: shield::ShieldLevel) -> gpui::Div {
    div()
        .h(px(28.))
        .px_2p5()
        .flex()
        .items_center()
        .gap_1p5()
        .rounded_full()
        .border_1()
        .border_color(Secure::hairline())
        .bg(Secure::chip())
        .font_family(ui::theme::SYSTEM_FONT)
        .text_size(px(TEXT_SMALL))
        .text_color(Secure::muted())
        .child(icon(Icon::Shield, 12., Secure::muted()))
        .child(shield_badge(level))
}

/// Height of the search row the effect frames; flames rise from its top edge.
const SEARCH_ROW: f32 = 54.;

/// The theme's search bar effect behind the start and command card (`browser_core::effects`),
/// redrawn at `effects::FPS` while its window is the active one of the active app; its own view,
/// like `Fog`. One still frame while motion is reduced or the window is in the background.
#[derive(Default)]
pub struct SearchFx {
    pub(crate) reduce: bool,
    started: Option<Instant>,
    ticking: bool,
}

impl gpui::Render for SearchFx {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use browser_core::effects::{self, Effect};
        let effect = Theme::search_effect();
        let animate = !self.reduce && effect.fps() > 0. && engine::app_active() && window.is_window_active();
        let t = if animate { effects::clock(self.started.get_or_insert_with(Instant::now).elapsed()) } else { 0.4 };
        if animate && !self.ticking {
            self.ticking = true;
            let frame = std::time::Duration::from_secs_f32(1. / effect.fps());
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(frame).await;
                this.update(cx, |fx, cx| {
                    fx.ticking = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let (side, rise) = effects::reach(effect, SEARCH_ROW);
        let color = Theme::search_color();
        let layer = div().absolute().top(px(-rise)).left(px(-side)).right(px(-side)).bottom(px(-side)).overflow_hidden();
        // The card sits on an opaque copy of the page colour, so the effect never shows through a
        // translucent card behind its text.
        let card = || div().absolute().top(px(rise)).left(px(side)).right(px(side)).bottom(px(side)).rounded(px(Theme::radius_card()));
        let underlay = card().bg(Rgba { a: 1., ..Theme::page() });
        match effect {
            Effect::None => layer,
            Effect::Glow => layer
                .child(card().shadow(vec![gpui::BoxShadow {
                    color: Rgba { a: 0.55 * effects::pulse(t), ..color }.into(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(effects::GLOW_BLUR),
                    spread_radius: px(effects::GLOW_SPREAD),
                }]))
                .child(underlay),
            _ => layer
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), window, _| {
                            let w = f32::from(bounds.size.width) - 2. * side;
                            // Sparkle runs around the whole card; flames and aurora grow from the search row.
                            let h = if effect == Effect::Sparkle { f32::from(bounds.size.height) - rise - side } else { SEARCH_ROW };
                            for p in effects::frame(effect, w, h, t).into_iter().filter(|p| p.alpha > 0.01) {
                                let (r, g, b) = effects::tint((color.r, color.g, color.b), p.heat);
                                // A faint halo under each dot softens the hard edge of a quad.
                                for (scale, alpha) in [(effects::HALO, 0.22), (1., 1.)] {
                                    let radius = p.radius * scale;
                                    let at = gpui::point(bounds.origin.x + px(side + p.x - radius), bounds.origin.y + px(rise + p.y - radius));
                                    let fill = Rgba { r, g, b, a: p.alpha * alpha };
                                    window.paint_quad(
                                        gpui::fill(gpui::Bounds::new(at, gpui::size(px(radius * 2.), px(radius * 2.))), fill).corner_radii(px(radius)),
                                    );
                                }
                            }
                        },
                    )
                    .size_full(),
                )
                .child(underlay),
        }
    }
}
