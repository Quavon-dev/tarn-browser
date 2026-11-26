use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, WindowHandle, div, point, prelude::FluentBuilder, px, size,
};
use ui::controls::{POPOVER_INSET, list_row, open_popup, popover};
use ui::icon::{Icon, icon};
use ui::motion::{REOPEN_GUARD, popover_motion, rise_in};
use ui::text::OneLine;
use ui::theme::{TEXT, TEXT_SMALL, Theme};

use crate::browser::Browser;
use crate::peek::remove_after_exit;
use crate::prefs::Pref;

const WIDTH: f32 = 272.;
const HEADER: f32 = 52.;
const ROW: f32 = 30.;
const PAD: f32 = 6.;
const GAP: f32 = 4.;
const COPIED: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    CopyLink,
    CopyCleanLink,
    QrCode,
    More,
    SavePdf,
    Print,
}

const GROUPS: [&[(Entry, &str)]; 3] = [
    &[(Entry::CopyLink, "Copy Link"), (Entry::CopyCleanLink, "Copy Clean Link"), (Entry::QrCode, "Show QR Code")],
    &[(Entry::More, "More…")],
    &[(Entry::SavePdf, "Save Page as PDF"), (Entry::Print, "Print…")],
];

fn entries() -> impl Iterator<Item = (Entry, &'static str)> {
    GROUPS.iter().flat_map(|g| g.iter().copied())
}

fn height() -> f32 {
    let rows = entries().count() as f32;
    HEADER + ROW * rows + PAD * 2. * GROUPS.len() as f32 + (GROUPS.len() - 1) as f32
}

#[derive(Default)]
pub struct ShareMenu {
    window: Option<WindowHandle<ShareMenuWindow>>,
    pub(crate) anchor: Rc<Cell<Bounds<Pixels>>>,
    closed: Option<Instant>,
}

impl ShareMenu {
    pub(crate) fn is_open(&self) -> bool {
        self.window.is_some()
    }
}

impl Browser {
    pub(crate) fn toggle_share_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.share_menu.window.take() {
            Some(handle) => cx.defer(move |cx| drop(handle.update(cx, |menu, window, cx| menu.close(true, window, cx)))),
            None if self.share_menu.closed.is_some_and(|t| t.elapsed() < REOPEN_GUARD) => {}
            None => self.show_share_menu(window, cx),
        }
    }

    fn show_share_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let Some(url) = self.tab_url(key).filter(|u| !u.is_empty()) else { return };
        let title = self.model.tab(key).map(crate::tabs::display_title).unwrap_or_default();
        let main = window.window_handle().downcast::<Browser>().or(self.peek.main);
        let (frame, button, field) = (window.bounds(), self.share_menu.anchor.get(), self.address.field.get());
        let (right, top) = if button.size.width > px(0.) {
            (button.right(), button.bottom())
        } else if field.size.width > px(0.) {
            (field.right(), field.bottom())
        } else {
            (frame.size.width, px(ui::theme::TOOLBAR_HEIGHT))
        };
        let x = (right - px(WIDTH)).clamp(px(0.), (frame.size.width - px(WIDTH)).max(px(0.))) - px(POPOVER_INSET);
        let bounds = Bounds::new(point(frame.origin.x + x, frame.origin.y + top + px(GAP)), size(px(WIDTH + 2. * POPOVER_INSET), px(height() + POPOVER_INSET)));
        let display = window.display(cx).map(|d| d.id());
        let favicon_url = url.clone();
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            let inset = [0., POPOVER_INSET, POPOVER_INSET, POPOVER_INSET];
            let (handle, _) = open_popup(cx, bounds, display, true, inset, Theme::radius_panel(), |window, cx| {
                ShareMenuWindow::new(browser.clone(), main, title.into(), favicon_url.into(), window, cx)
            });
            browser
                .update(cx, |b, cx| {
                    b.share_menu.window = handle;
                    cx.notify();
                })
                .ok();
        });
    }

    pub(crate) fn system_share(&mut self, url: String, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone().filter(|_| !url.is_empty()) else { return };
        let (button, field) = (self.share_menu.anchor.get(), self.address.field.get());
        let at = if button.size.width > px(0.) { button } else { field };
        let (x, y, w, h) = if at.size.width > px(0.) {
            (f32::from(at.origin.x), f32::from(at.origin.y), f32::from(at.size.width), f32::from(at.size.height))
        } else {
            let slot = self.address.slot.get();
            (f32::from(slot.origin.x), 0., f32::from(slot.size.width).max(1.), ui::theme::TOOLBAR_HEIGHT)
        };
        cx.spawn(async move |_, _| host.share(&url, x.into(), y.into(), w.into(), h.into())).detach();
    }
}

pub struct ShareMenuWindow {
    browser: WeakEntity<Browser>,
    main: Option<WindowHandle<Browser>>,
    title: SharedString,
    url: SharedString,
    focus: FocusHandle,
    selected: Option<usize>,
    copied: Option<Entry>,
    closing: bool,
    _activation: Subscription,
}

impl ShareMenuWindow {
    fn new(
        browser: WeakEntity<Browser>,
        main: Option<WindowHandle<Browser>>,
        title: SharedString,
        url: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(false, window, cx);
            }
        });
        ShareMenuWindow { browser, main, title, url, focus, selected: None, copied: None, closing: false, _activation: activation }
    }

    fn reduce(&self, cx: &gpui::App) -> bool {
        self.browser.upgrade().is_some_and(|b| b.read(cx).pref(Pref::ReduceMotion))
    }

    fn close(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.closing, true) {
            return;
        }
        remove_after_exit(self.reduce(cx), window, cx);
        self.browser
            .update(cx, |b, cx| {
                b.share_menu.window = None;
                b.share_menu.closed = (!refocus).then(Instant::now);
                cx.notify();
            })
            .ok();
        if let Some(main) = self.main.filter(|_| refocus) {
            main.update(cx, |_, window, _| window.activate_window()).ok();
        }
        cx.notify();
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>) + 'static) {
        self.close(true, window, cx);
        if let Some(main) = self.main {
            cx.defer(move |cx| drop(main.update(cx, f)));
        }
    }

    /// The QR code and the system share sheet send the address to another device or app.
    fn enabled(&self, entry: Entry) -> bool {
        !matches!(entry, Entry::QrCode | Entry::More) || crate::qr::web_link(&self.url)
    }

    fn choose(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(entry) {
            return;
        }
        match entry {
            Entry::CopyLink | Entry::CopyCleanLink => {
                let clean = entry == Entry::CopyCleanLink;
                if let Some(main) = self.main {
                    cx.defer(move |cx| {
                        main.update(cx, |b, _, cx| {
                            if let Some(key) = b.model.active {
                                b.copy_link(key, clean, cx);
                            }
                        })
                        .ok();
                    });
                }
                self.copied = Some(entry);
                cx.notify();
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(COPIED).await;
                    this.update_in(cx, |this, window, cx| this.close(true, window, cx)).ok();
                })
                .detach();
            }
            Entry::QrCode => self.run(window, cx, |b, w, cx| b.show_qr_code(w, cx)),
            Entry::More => {
                let url = self.url.to_string();
                self.run(window, cx, move |b, _, cx| b.system_share(url, cx));
            }
            Entry::SavePdf => self.run(window, cx, |b, _, cx| b.capture(false, cx)),
            Entry::Print => self.run(window, cx, |b, _, _| b.print()),
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = entries().count() as isize;
        let next = match self.selected {
            Some(ix) => (ix as isize + by).rem_euclid(count),
            None if by > 0 => 0,
            None => count - 1,
        };
        self.selected = Some(next as usize);
        cx.notify();
    }

    fn shortcut(entry: Entry, cx: &gpui::App) -> String {
        match entry {
            Entry::CopyLink => crate::shortcuts::shown(&crate::CopyLink, cx),
            Entry::CopyCleanLink => crate::shortcuts::shown(&crate::CopyCleanLink, cx),
            Entry::QrCode => crate::shortcuts::shown(&crate::qr::ShowQrCode, cx),
            Entry::SavePdf => crate::shortcuts::shown(&crate::capture::SavePdf, cx),
            Entry::Print => crate::shortcuts::shown(&crate::Print, cx),
            Entry::More => String::new(),
        }
    }

    fn row(&self, ix: usize, entry: Entry, label: &'static str, reduce: bool, cx: &mut Context<Self>) -> AnyElement {
        let glyph = if entry == Entry::More { Icon::Ellipsis } else { Icon::named(browser_core::command::glyph(label)).unwrap_or(Icon::Command) };
        let glyph = match self.copied {
            Some(copied) if copied == entry => rise_in(div().child(icon(Icon::Check, 15., Theme::accent())), ("copied", ix), 5., reduce),
            _ => icon(glyph, 15., if self.enabled(entry) { Theme::muted() } else { Theme::faint() }).into_any_element(),
        };
        let keys = Self::shortcut(entry, cx);
        list_row(("share", ix), self.selected == Some(ix))
            .h(px(ROW))
            .mx(px(PAD))
            .px_2()
            .child(glyph)
            .when(!self.enabled(entry), |d| d.text_color(Theme::faint()))
            .child(div().flex_1().one_line(label))
            .when(!keys.is_empty(), |d| d.child(div().text_size(px(TEXT_SMALL)).text_color(Theme::faint()).child(keys)))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(entry, window, cx)))
            .into_any_element()
    }

    fn header(&self, favicon: AnyElement) -> gpui::Div {
        div().h(px(HEADER)).flex_none().px_3().flex().items_center().gap_2p5().border_b_1().border_color(Theme::separator()).child(favicon).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().font_weight(FontWeight::SEMIBOLD).one_line(self.title.clone()))
                .child(div().text_size(px(TEXT_SMALL)).text_color(Theme::faint()).one_line(display_url(&self.url))),
        )
    }
}

fn display_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.strip_suffix('/').unwrap_or(rest).to_owned()
}

impl Render for ShareMenuWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reduce = self.reduce(cx);
        let favicon = self.browser.upgrade().map(|b| crate::sidebar::site_icon(b.read(cx), &self.url, 28.)).unwrap_or_else(|| div().into_any_element());
        let mut ix = 0;
        let mut list = div().flex().flex_col();
        for (n, group) in GROUPS.iter().enumerate() {
            if n > 0 {
                list = list.child(div().h(px(1.)).mx_3().bg(Theme::separator()));
            }
            let mut rows = div().py(px(PAD)).flex().flex_col();
            for (entry, label) in group.iter() {
                rows = rows.child(self.row(ix, *entry, label, reduce, cx));
                ix += 1;
            }
            list = list.child(rows);
        }
        let card = popover(Theme::radius_panel(), 0.).flex().flex_col().text_size(px(TEXT)).text_color(Theme::text()).child(self.header(favicon)).child(list);
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::qr::ShareLink, window, cx| this.close(true, window, cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => this.close(true, window, cx),
                "down" => this.step(1, cx),
                "up" => this.step(-1, cx),
                "tab" => this.step(if e.keystroke.modifiers.shift { -1 } else { 1 }, cx),
                "enter" | "space" => {
                    if let Some((entry, _)) = this.selected.and_then(|ix| entries().nth(ix)) {
                        this.choose(entry, window, cx);
                    }
                }
                _ => {}
            }))
            .size_full()
            .child(popover_motion(div().relative().size_full().child(card), "share-menu", self.closing, reduce))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_offers_no_device_sending_and_ends_with_print() {
        let labels: Vec<&str> = entries().map(|(_, label)| label).collect();
        assert_eq!(labels, ["Copy Link", "Copy Clean Link", "Show QR Code", "More…", "Save Page as PDF", "Print…"]);
        assert_eq!(height(), HEADER + 6. * ROW + 6. * PAD + 2.);
    }

    #[test]
    fn the_header_shows_the_url_without_its_scheme() {
        assert_eq!(display_url("https://tarn.example/docs/"), "tarn.example/docs");
        assert_eq!(display_url("about:blank"), "about:blank");
    }
}
