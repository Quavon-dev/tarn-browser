use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use gpui::{
    Bounds, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, PathBuilder, Render, StatefulInteractiveElement,
    Styled, Subscription, WeakEntity, Window, WindowHandle, canvas, div, point, prelude::FluentBuilder, px, size, svg,
};
use store::{Download, Id};
use ui::controls::{POPOVER_INSET, empty, icon_button, list_row, open_popup, popover, tab_stop, text_lines, tooltip};
use ui::icon::{Icon, icon};
use ui::motion::{REOPEN_GUARD, popover_motion};
use ui::text::OneLine;
use ui::text::bounded_title;
use ui::theme::{Secure, TEXT, TEXT_SMALL, Theme};

use crate::ToggleDownloads;
use crate::browser::Browser;
use crate::peek::remove_after_exit;
use crate::prefs::Pref;
use crate::sidebar::{HEADER_HEIGHT, SIDEBAR_MARGIN, TILE_GAP};

const WIDTH: f32 = 320.;
const CARET: f32 = 10.;
/// The caret's centre, from the popover's left edge.
const CARET_AT: f32 = 160.;
const HEADER: f32 = 40.;
const ROW: f32 = 48.;
const FOOTER: f32 = 37.;
const PAD: f32 = 6.;
const MAX_ROWS: usize = 5;

/// The popover's window, the number of downloads saving per profile, the downloads saving or
/// paused by (private, profile, id), and a counter that moves whenever the list may have changed.
#[derive(Default)]
pub struct Downloads {
    pub(crate) window: Option<WindowHandle<DownloadsWindow>>,
    pub(crate) active: HashMap<(bool, Id), usize>,
    pub(crate) transfers: BTreeMap<(bool, Id, u64), Transfer>,
    pub(crate) changed: u64,
    closed: Option<Instant>,
    pub(crate) button: std::rc::Rc<std::cell::Cell<gpui::Bounds<gpui::Pixels>>>,
}

impl Downloads {
    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }
}

/// A download saving or paused, as the engine last reported it.
#[derive(Clone)]
pub(crate) struct Transfer {
    pub path: PathBuf,
    pub done: u64,
    pub total: Option<u64>,
    pub paused: bool,
    /// When Tarn first heard of it, kept across progress reports.
    pub started: i64,
}

fn subtitle(d: &Download, size: Option<u64>) -> (String, bool) {
    match d.state.as_str() {
        "failed" => ("Failed".into(), true),
        "cancelled" => ("Cancelled".into(), false),
        "finished-unquarantined" => ("Not marked as downloaded · open only if you trust it".into(), true),
        _ => {
            let what = size
                .map_or_else(|| Path::new(&d.path).extension().map_or_else(|| "File".into(), |e| bounded_title(&e.to_string_lossy().to_lowercase())), bytes);
            (crate::browser::tab_host(&d.url).into_iter().fold(what, |line, host| format!("{line} · {host}")), false)
        }
    }
}

/// A finished download whose file was moved or deleted since.
fn missing(d: &Download, file: Option<Option<u64>>) -> bool {
    file == Some(None) && d.state.starts_with("finished")
}

fn tile(name: &str, color: gpui::Rgba) -> gpui::Div {
    let glyph = Icon::named(browser_core::glyphs::file_glyph(name)).unwrap_or(Icon::File);
    div().size(px(28.)).flex_none().flex().items_center().justify_center().rounded(px(Theme::radius_row())).bg(Theme::control()).child(icon(glyph, 15., color))
}

/// Under the name of a download in progress: how much of it is saved.
fn transfer_subtitle(t: &Transfer) -> String {
    let size = match t.total {
        Some(total) => format!("{} of {}", bytes(t.done), bytes(total)),
        None => bytes(t.done),
    };
    if t.paused { format!("Paused · {size}") } else { size }
}

fn bytes(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} bytes"),
        1_000..1_000_000 => format!("{:.0} KB", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.2} GB", n as f64 / 1e9),
    }
}

/// Saved share of the downloads saving with a known size, for the toolbar ring.
fn saved_share<'a>(transfers: impl Iterator<Item = &'a Transfer>) -> Option<f32> {
    let (done, total) = transfers.filter(|t| !t.paused).filter_map(|t| Some((t.done, t.total?))).fold((0, 0), |(d, s), (done, total)| (d + done, s + total));
    (total > 0).then(|| (done as f64 / total as f64) as f32)
}

/// An arc over the download icon's circle, clockwise from the top.
fn ring(share: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (center, radius) = (bounds.center(), px(16. * 10. / 24.));
            let angle = std::f32::consts::TAU * share.clamp(0.02, 0.999) - std::f32::consts::FRAC_PI_2;
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(point(center.x, center.y - radius));
            path.arc_to(point(radius, radius), px(0.), share > 0.5, true, point(center.x + radius * angle.cos(), center.y + radius * angle.sin()));
            if let Ok(path) = path.build() {
                window.paint_path(path, Theme::accent());
            }
        },
    )
    .absolute()
    .inset_0()
}

fn card_height(rows: usize) -> f32 {
    HEADER + ROW * rows.max(1) as f32 + PAD + FOOTER
}

/// How many rows the popover's window has room for.
fn rows_fit(window_height: f32) -> usize {
    ((window_height - CARET - POPOVER_INSET - card_height(0) + ROW) / ROW).round() as usize
}

impl Browser {
    pub(crate) fn set_active_downloads(&mut self, private: bool, profile_id: Id, count: usize, cx: &mut Context<Self>) {
        self.downloads_popover.active.insert((private, profile_id), count);
        self.downloads_popover.changed += 1;
        cx.notify();
    }

    fn active_downloads(&self) -> usize {
        self.model.current_space().and_then(|s| self.downloads_popover.active.get(&(s.private, s.space.profile_id))).copied().unwrap_or(0)
    }

    pub(crate) fn set_transfer(&mut self, key: (bool, Id, u64), transfer: Option<Transfer>, cx: &mut Context<Self>) {
        let started_or_ended = match transfer {
            Some(mut t) => {
                let earlier = self.downloads_popover.transfers.get(&key).map(|old| old.started);
                t.started = earlier.unwrap_or(t.started);
                self.downloads_popover.transfers.insert(key, t).is_none()
            }
            None => self.downloads_popover.transfers.remove(&key).is_some(),
        };
        if started_or_ended {
            self.downloads_popover.changed += 1;
        }
        cx.notify();
    }

    /// The current space's downloads saving or paused, by id.
    fn transfers(&self) -> Vec<(u64, Transfer)> {
        let Some(s) = self.model.current_space() else { return Vec::new() };
        let (private, profile) = (s.private, s.space.profile_id);
        self.downloads_popover.transfers.range((private, profile, 0)..=(private, profile, u64::MAX)).map(|((.., id), t)| (*id, t.clone())).collect()
    }

    fn pause_or_resume(&self, id: u64, paused: bool) {
        let Some(s) = self.model.current_space() else { return };
        let Some(rt) = self.loaded(s.private, s.space.profile_id) else { return };
        if paused {
            rt.data.resume_download(id);
        } else {
            rt.data.pause_download(id);
        }
    }

    fn cancel_download(&self, id: u64) {
        let Some(s) = self.model.current_space() else { return };
        if let Some(rt) = self.loaded(s.private, s.space.profile_id) {
            rt.data.cancel_download(id);
        }
    }

    pub fn toggle_downloads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.downloads_popover.window.take() {
            Some(handle) => {
                cx.defer(move |cx| {
                    handle.update(cx, |popover, window, cx| popover.close(true, window, cx)).ok();
                });
                cx.notify();
            }
            // The click that took focus from the popover already closed it; it must not reopen it.
            None if self.downloads_popover.closed.is_some_and(|t| t.elapsed() < REOPEN_GUARD) => {}
            None => self.show_downloads(window, cx),
        }
    }

    /// Opens the popover under the sidebar header's download button, its caret on the button, or
    /// under the toolbar while the sidebar is hidden, with room for the downloads saving now to
    /// finish into rows.
    fn show_downloads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(main) = window.window_handle().downcast::<Browser>().or(self.peek.main) else { return };
        let top = match (self.peek.is_window(window), self.sidebar_open) {
            (true, _) => HEADER_HEIGHT - 4.,
            (false, true) => SIDEBAR_MARGIN + HEADER_HEIGHT - 4.,
            (false, false) => ui::theme::TOOLBAR_HEIGHT,
        };
        // Under the toolbar there is no button for the caret to point at.
        let caret = self.peek.is_window(window) || self.sidebar_open;
        let anchor = window.bounds();
        let saving = self.transfers().len();
        let rows = (self.downloads().len() + saving).min(MAX_ROWS) + saving.min(MAX_ROWS);
        let painted = self.downloads_popover.button.get();
        let button = if painted.size.width > px(0.) && !self.peek.is_window(window) {
            f32::from(painted.center().x)
        } else {
            SIDEBAR_MARGIN + self.sidebar_width - TILE_GAP - 14.
        };
        let left = (button - CARET_AT - POPOVER_INSET).clamp(0., (f32::from(anchor.size.width) - WIDTH - 2. * POPOVER_INSET).max(0.));
        let caret = caret.then(|| (button - left - POPOVER_INSET).clamp(2. * CARET, WIDTH - 2. * CARET));
        let bounds = Bounds::new(
            point(anchor.origin.x + px(left), anchor.origin.y + px(top)),
            size(px(WIDTH + 2. * POPOVER_INSET), px(CARET + card_height(rows) + POPOVER_INSET)),
        );
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let inset = [CARET, POPOVER_INSET, POPOVER_INSET, POPOVER_INSET];
            let (handle, _) = open_popup(cx, bounds, display, true, inset, Theme::radius_panel(), |window, cx| {
                DownloadsWindow::new(browser.clone(), main, caret, window, cx)
            });
            browser
                .update(cx, |b, cx| {
                    b.downloads_popover.window = handle;
                    cx.notify();
                })
                .ok();
        });
    }
}

/// The header button; while a download of this profile is saving, a ring shows how much is saved,
/// or a dot when no size is known.
pub fn header_button(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let share = saved_share(b.transfers().iter().map(|(_, t)| t));
    icon_button("downloads", Icon::Download, true)
        .relative()
        .child(crate::address::measure(b.downloads_popover.button.clone()))
        .when(!b.downloads_popover.is_open(), |d| d.tooltip(crate::shortcuts::tip("Downloads", &crate::ToggleDownloads)))
        .when(b.downloads_popover.is_open(), |d| d.bg(Theme::selected()))
        .when_some(share, |d, share| d.child(ring(share)))
        .when(share.is_none() && b.active_downloads() > 0, |d| {
            d.child(div().absolute().top(px(5.)).right(px(5.)).size(px(6.)).rounded_full().bg(Theme::accent()))
        })
        .on_click(cx.listener(|b, _, window, cx| b.toggle_downloads(window, cx)))
}

pub struct DownloadsWindow {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    focus: FocusHandle,
    /// Each finished download with its file's size: `None` until checked, `Some(None)` once the file is gone.
    rows: Vec<(Download, Option<Option<u64>>)>,
    seen: Option<u64>,
    closing: bool,
    /// The caret's centre from the card's left edge, if it points at a button.
    caret: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl DownloadsWindow {
    fn new(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, caret: Option<f32>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut subscriptions = vec![cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(false, window, cx);
            }
        })];
        subscriptions.extend(browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify())));
        DownloadsWindow { browser, main, focus, rows: Vec::new(), seen: None, closing: false, caret, _subscriptions: subscriptions }
    }

    fn close(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        let reduce = self.browser.upgrade().is_some_and(|b| b.read(cx).pref(Pref::ReduceMotion));
        remove_after_exit(reduce, window, cx);
        self.browser
            .update(cx, |b, cx| {
                b.downloads_popover.window = None;
                b.downloads_popover.closed = (!refocus).then(Instant::now);
                cx.notify();
            })
            .ok();
        if refocus {
            self.main.update(cx, |_, window, _| window.activate_window()).ok();
        }
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browser
            .update(cx, |b, cx| {
                let Some(s) = b.model.current_space() else { return };
                let cleared = Box::new((s.private, s.space.profile_id, b.downloads()));
                b.remove_download(None);
                b.show_toast("Downloads cleared", Some(crate::toast::Action::RestoreDownloads(cleared)), cx);
            })
            .ok();
        self.close(true, window, cx);
    }

    /// The library takes focus itself; activating the main window as well would close it again.
    fn view_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(false, window, cx);
        self.main.update(cx, |b, window, cx| b.open_library(crate::library::Page::Downloads, None, window, cx)).ok();
    }

    fn row(&self, d: &Download, file: Option<Option<u64>>) -> impl IntoElement {
        let path = d.path.clone();
        let reveal = move || engine::reveal_in_finder(Path::new(&path));
        let button_reveal = reveal.clone();
        let name = Path::new(&d.path).file_name().map(|n| bounded_title(&n.to_string_lossy())).unwrap_or_default();
        let gone = missing(d, file);
        let (detail, warn) = if gone { ("Missing · moved or deleted".into(), true) } else { subtitle(d, file.flatten()) };
        let trailing = if warn {
            div().size(px(28.)).flex().flex_none().items_center().justify_center().child(icon(Icon::TriangleAlert, 15., Secure::warning())).into_any_element()
        } else {
            icon_button(("reveal", d.id as u64), Icon::FolderOpen, true)
                .tooltip(tooltip(ui::controls::REVEAL, None))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    button_reveal();
                })
                .into_any_element()
        };
        tab_stop(list_row(("download", d.id as u64), false).h(px(ROW)).px_2())
            .when(!gone, |d| d.on_click(move |_, _, _| reveal()))
            .child(tile(&name, if warn { Secure::warning() } else { Theme::muted() }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().one_line(name))
                    .child(div().text_size(px(TEXT_SMALL)).text_color(if warn { Secure::warning() } else { Theme::muted() }).one_line(detail)),
            )
            .child(trailing)
    }

    fn transfer_row(&self, id: u64, t: &Transfer, cx: &mut Context<Self>) -> impl IntoElement {
        let name = t.path.file_name().map(|n| bounded_title(&n.to_string_lossy())).unwrap_or_default();
        let paused = t.paused;
        let (glyph, label) = if paused { (Icon::Play, "Resume") } else { (Icon::Pause, "Pause") };
        let share = t.total.filter(|total| *total > 0).map(|total| (t.done as f64 / total as f64).clamp(0., 1.) as f32);
        let bar = div()
            .h(px(3.))
            .rounded_full()
            .bg(Theme::separator())
            .overflow_hidden()
            .child(div().h_full().w(gpui::relative(share.unwrap_or(0.))).rounded_full().bg(if paused { Theme::faint() } else { Theme::accent() }));
        div()
            .h(px(ROW))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap_2p5()
            .child(tile(&name, if paused { Theme::muted() } else { Theme::accent() }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(text_lines(name.clone(), transfer_subtitle(t)))
                    .when(share.is_some(), |d| d.child(bar)),
            )
            .child(tab_stop(icon_button(("transfer", id), glyph, true)).tooltip(tooltip(label, None)).on_click(cx.listener(move |this, _, _, cx| {
                this.browser.update(cx, |b, _| b.pause_or_resume(id, paused)).ok();
            })))
            .child(tab_stop(icon_button(("cancel", id), Icon::Close, true)).tooltip(tooltip("Cancel", None)).on_click(cx.listener(move |this, _, _, cx| {
                this.browser.update(cx, |b, _| b.cancel_download(id)).ok();
            })))
    }
}

impl Render for DownloadsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let fit = rows_fit(f32::from(window.bounds().size.height));
        let changed = browser.read(cx).downloads_popover.changed;
        if self.seen != Some(changed) {
            self.seen = Some(changed);
            self.rows = browser.read(cx).downloads().into_iter().take(MAX_ROWS).map(|d| (d, None)).collect();
            let paths: Vec<_> = self.rows.iter().map(|(d, _)| d.path.clone()).collect();
            cx.spawn(async move |this, cx| {
                let sizes: Vec<Option<u64>> = cx
                    .background_executor()
                    .spawn(async move { paths.iter().map(|p| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len())).collect() })
                    .await;
                this.update(cx, |w, cx| {
                    if w.seen == Some(changed) {
                        w.rows.iter_mut().zip(sizes).for_each(|(row, size)| row.1 = Some(size));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        let b = browser.read(cx);
        let (transfers, reduce) = (b.transfers(), b.pref(Pref::ReduceMotion));
        let header = div()
            .h(px(HEADER))
            .flex_none()
            .pl_3()
            .pr_2()
            .flex()
            .items_center()
            .justify_between()
            .child(div().font_weight(FontWeight::BOLD).child("Downloads"))
            .when(!self.rows.is_empty(), |d| {
                d.child(
                    tab_stop(
                        div().id("downloads-clear").px_1().rounded(px(Theme::radius_small())).cursor_pointer().text_color(Theme::accent_text()).child("Clear"),
                    )
                    .hover(|s| s.opacity(0.8))
                    .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                )
            });
        let saving: Vec<_> = transfers.iter().take(fit).map(|(id, t)| self.transfer_row(*id, t, cx).into_any_element()).collect();
        let empty = (self.rows.is_empty() && transfers.is_empty()).then(|| empty("No downloads yet"));
        let footer = tab_stop(div().id("downloads-all"))
            .mt(px(PAD))
            .h(px(FOOTER - 1.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(Theme::hairline())
            .cursor_pointer()
            .hover(|s| s.bg(Theme::hover()))
            .child("View all downloads")
            .child(icon(Icon::ArrowUpRight, 14., Theme::muted()))
            .on_click(cx.listener(|this, _, window, cx| this.view_all(window, cx)));
        let finished = fit.saturating_sub(saving.len());
        let list =
            div().px_1p5().flex().flex_col().children(saving).children(self.rows.iter().take(finished).map(|(d, size)| self.row(d, *size))).children(empty);
        let card = popover(Theme::radius_panel(), CARET).flex().flex_col().child(header).child(list).child(div().flex_1()).child(footer);
        let caret = svg()
            .path(Icon::Caret.path())
            .absolute()
            .top_0()
            .left(px(POPOVER_INSET + self.caret.unwrap_or(CARET_AT) - CARET))
            .w(px(2. * CARET))
            .h(px(CARET + 1.))
            .text_color(Theme::surface());
        let content = div().relative().size_full().text_size(px(TEXT)).text_color(Theme::text()).child(card).when(self.caret.is_some(), |d| d.child(caret));
        let content = popover_motion(content, "downloads", self.closing, reduce);
        div()
            .track_focus(&self.focus)
            .key_context("Downloads")
            .on_action(cx.listener(|this, _: &ToggleDownloads, window, cx| this.close(true, window, cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => this.close(true, window, cx),
                "tab" if e.keystroke.modifiers.shift => window.focus_prev(),
                "tab" | "down" => window.focus_next(),
                "up" => window.focus_prev(),
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

    #[test]
    fn rows_name_the_file_type() {
        let d = Download { id: 1, url: String::new(), path: "/Users/u/Downloads/ipogo-4.6.3.IPA".into(), state: "finished".into(), started_at: 0 };
        assert_eq!(subtitle(&d, None), ("ipa".into(), false));
        assert_eq!(subtitle(&Download { path: "/Users/u/Downloads/README".into(), ..d.clone() }, None), ("File".into(), false));
    }

    #[test]
    fn rows_show_size_and_site_or_a_warning() {
        let d = Download { id: 1, url: "https://tarn.example/a.zip".into(), path: "/d/a.zip".into(), state: "finished".into(), started_at: 0 };
        assert_eq!(subtitle(&d, Some(1_200_000)), ("1.2 MB · tarn.example".into(), false));
        assert!(subtitle(&Download { state: "failed".into(), ..d.clone() }, None).1);
        assert!(subtitle(&Download { state: "finished-unquarantined".into(), ..d.clone() }, Some(1)).1);
        assert_eq!(subtitle(&Download { state: "cancelled".into(), ..d }, None), ("Cancelled".into(), false));
    }

    #[test]
    fn a_finished_download_without_its_file_is_missing() {
        let d = Download { id: 1, url: String::new(), path: "/d/a.zip".into(), state: "finished".into(), started_at: 0 };
        assert!(missing(&d, Some(None)));
        assert!(!missing(&d, None), "not checked yet");
        assert!(!missing(&d, Some(Some(3))));
        assert!(!missing(&Download { state: "failed".into(), ..d }, Some(None)));
    }

    #[test]
    fn the_window_fits_the_rows_it_was_sized_for() {
        for rows in 1..=2 * MAX_ROWS {
            assert_eq!(rows_fit(CARET + card_height(rows) + POPOVER_INSET), rows);
        }
    }

    #[test]
    fn transfers_show_size_pause_and_saved_share() {
        let t = |done, total, paused| Transfer { path: PathBuf::from("/d/a.zip"), done, total, paused, started: 0 };
        assert_eq!(transfer_subtitle(&t(512, None, false)), "512 bytes");
        assert_eq!(transfer_subtitle(&t(1_500_000, Some(40_000_000), false)), "1.5 MB of 40.0 MB");
        assert_eq!(transfer_subtitle(&t(2_000, Some(3_000_000_000), true)), "Paused · 2 KB of 3.00 GB");
        assert_eq!(saved_share([t(1, None, false)].iter()), None);
        assert_eq!(saved_share([t(25, Some(100), false), t(50, Some(100), true), t(75, Some(100), false)].iter()), Some(0.5));
    }
}
