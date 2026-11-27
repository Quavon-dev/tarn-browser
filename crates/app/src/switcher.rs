use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser_core::switcher::Switcher;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyWindowHandle, AppContext, Bounds, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ModifiersChangedEvent, MouseMoveEvent, ObjectFit,
    ParentElement, Pixels, Render, RenderImage, Rgba, StatefulInteractiveElement, Styled, StyledImage, Subscription, WeakEntity, Window, WindowHandle, div,
    img, point, px, size,
};
use ui::motion::{EXIT, fade_in, pop_in, progress};
use ui::text::OneLine;
use ui::theme::{Elevation, TEXT, TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};
use crate::prefs::Pref;

/// Cards grow with the page between these widths, filling one row.
const MIN_CARD: f32 = 150.;
const MAX_CARD: f32 = 240.;
/// The selected card's ring stands this far off its edge.
const RING: f32 = 4.;
const GAP: f32 = 20.;
const MARGIN: f32 = 40.;
/// So few pixels that the GPU's smoothing, stretching them over the page, reads as a blur.
const BACKDROP_WIDTH: f32 = 24.;
/// A quick press and release switches without the strip flashing over the page.
const SHOW_DELAY: Duration = Duration::from_millis(100);
const RELEASE_POLL: Duration = Duration::from_millis(30);

type Shots = HashMap<TabKey, Arc<RenderImage>>;

/// The open switcher and stills of its awake tabs, taken when the strip shows and dropped when it closes.
pub struct Open {
    pub state: Switcher,
    opened: Instant,
    shots: Shots,
    /// With the Glass design, a tiny still of the page that, stretched, frosts it behind the strip.
    backdrop: Option<Arc<RenderImage>>,
    pub(crate) strip: Option<WindowHandle<Strip>>,
    _deactivated: Subscription,
}

/// A card's width when `n` of them share one row of a page `width` wide.
fn card_width(width: f32, n: usize) -> f32 {
    let n = n.max(1) as f32;
    ((width - 2. * MARGIN - (n - 1.) * GAP) / n - 2. * RING).clamp(MIN_CARD, MAX_CARD)
}

fn open_strip(b: &Browser) -> Option<WindowHandle<Strip>> {
    b.tab_switcher.as_ref().and_then(|o| o.strip)
}

fn redraw(strip: Option<WindowHandle<Strip>>, cx: &mut gpui::App) {
    if let Some(strip) = strip {
        cx.defer(move |cx| drop(strip.update(cx, |_, _, cx| cx.notify())));
    }
}

impl Browser {
    pub fn recent_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        // The peek window gets the modifier release, so from there switch without the strip.
        if self.peek.is_window(window) {
            if let Some(key) = self.model.recent(forward) {
                self.switch_to(key, window, cx);
            }
            return;
        }
        if let Some(open) = &mut self.tab_switcher {
            open.state.step(forward);
            return redraw(open.strip, cx);
        }
        // As many cards as fit on one row of this window.
        let fits = ((f32::from(self.page_area(window).size.width) - 2. * MARGIN + GAP) / (MIN_CARD + 2. * RING + GAP)).floor().max(2.) as usize;
        let Some(state) = self.model.open_switcher(forward, fits) else { return };
        // A focused web view takes the modifier's key events, so ask the system what is held.
        if !engine::modifier_held() {
            return self.switch_to(state.selected(), window, cx);
        }
        self.close_command(window, cx);
        // A release that happens in another window or app never reaches this one.
        let opened = Instant::now();
        let deactivated = cx.observe_window_activation(window, |_, window, cx| {
            // AppKit names the new key window only after this call, and a click on the strip makes it key.
            cx.on_next_frame(window, |b, window, cx| {
                let strip = b.tab_switcher.as_ref().and_then(|o| o.strip).map(AnyWindowHandle::from);
                if !window.is_window_active() && (strip.is_none() || cx.active_window() != strip) {
                    b.cancel_switcher(window, cx);
                }
            });
        });
        self.tab_switcher = Some(Open { state, opened, shots: HashMap::new(), backdrop: None, strip: None, _deactivated: deactivated });
        if let Some(host) = &self.host {
            host.focus();
        }
        window.focus(&self.focus_handle(cx));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SHOW_DELAY).await;
            this.update_in(cx, |b, window, cx| b.show_switcher(opened, window, cx)).ok();
        })
        .detach();
        // The release can go to a web view or another window, so watch the system's state as well.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(RELEASE_POLL).await;
                let open = this.update_in(cx, |b, window, cx| {
                    let open = b.tab_switcher.as_ref().is_some_and(|o| o.opened == opened);
                    if open && !engine::modifier_held() {
                        b.commit_switcher(window, cx);
                        return false;
                    }
                    open
                });
                if !matches!(open, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The page's area in the window, where the strip is centred; the whole window when no page shows.
    fn page_area(&self, window: &Window) -> Bounds<Pixels> {
        match self.link_status.page.get() {
            (x, y, w, h) if w > 0. && h > 0. => Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
            _ => Bounds::new(point(px(0.), px(0.)), window.viewport_size()),
        }
    }

    /// The strip floats in a window of its own over the page, so the page stays in view.
    fn show_switcher(&mut self, opened: Instant, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.tab_switcher.as_ref().filter(|o| o.strip.is_none() && o.opened == opened) else { return };
        let mut bounds = self.page_area(window);
        // WebKit encodes each still on the main thread, so take it no larger than its card.
        let card = card_width(f32::from(bounds.size.width), open.state.order.len());
        for key in open.state.order.clone() {
            self.request_switcher_shot(opened, key, Some(key), card, cx);
        }
        if let Some(active) = self.model.active.filter(|_| self.core.themes.borrow().glass) {
            self.request_switcher_shot(opened, active, None, BACKDROP_WIDTH, cx);
        }
        bounds.origin += window.bounds().origin;
        let (display, main) = (window.display(cx).map(|d| d.id()), window.window_handle());
        let (browser, reduce) = (cx.entity().downgrade(), self.pref(Pref::ReduceMotion));
        cx.defer(move |cx| {
            let Some(entity) = browser.upgrade() else { return };
            let (handle, _) =
                ui::controls::open_popup(cx, bounds, display, false, [0.; 4], 0., |window, cx| Strip::new(&entity, opened, main, reduce, window, cx));
            let Some(handle) = handle else { return };
            let kept = entity.update(cx, |b, _| b.tab_switcher.as_mut().filter(|o| o.opened == opened).map(|o| o.strip = Some(handle)).is_some());
            if !kept {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    pub fn commit_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.tab_switcher.take() else { return };
        self.close_command(window, cx);
        let key = open.state.selected();
        if self.model.tab(key).is_some() {
            self.switch_to(key, window, cx);
        }
    }

    pub fn cancel_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_switcher.take().is_some() {
            self.close_command(window, cx);
        }
    }

    /// A still of `key` at `width` points, for card `card`, or the backdrop when `None`.
    fn request_switcher_shot(&self, opened: Instant, key: TabKey, card: Option<TabKey>, width: f32, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key) else { return };
        let (tx, mut rx) = mpsc::unbounded();
        view.snapshot_png(Some(f64::from(width)), move |png| drop(tx.unbounded_send(png)));
        cx.spawn(async move |this, cx| {
            let Some(Some(png)) = rx.next().await else { return };
            let Some(image) = cx.background_spawn(async move { crate::hover_card::decode(&png) }).await else { return };
            this.update(cx, |b, cx| {
                // A still that arrives after its switcher closed and another opened is dropped.
                if let Some(open) = b.tab_switcher.as_mut().filter(|o| o.opened == opened) {
                    match card {
                        Some(key) => drop(open.shots.insert(key, image)),
                        None => open.backdrop = Some(image),
                    }
                    redraw(open.strip, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

pub struct Strip {
    browser: WeakEntity<Browser>,
    main: AnyWindowHandle,
    reduce: bool,
    /// What the strip last showed, so its cards can fade out over the page after the switch.
    shown: Option<(Switcher, Shots, Option<Arc<RenderImage>>)>,
    leaving: Option<Instant>,
    _watch: Subscription,
}

impl Strip {
    fn new(browser: &gpui::Entity<Browser>, opened: Instant, main: AnyWindowHandle, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let watch = cx.observe_in(browser, window, move |this: &mut Self, browser, window, cx| {
            if browser.read(cx).tab_switcher.as_ref().is_some_and(|o| o.opened == opened) {
                return;
            }
            if this.reduce || this.shown.is_none() {
                return window.remove_window();
            }
            if this.leaving.is_none() {
                this.leaving = Some(Instant::now());
                cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor().timer(EXIT).await;
                    cx.update(|window, _| window.remove_window()).ok();
                })
                .detach();
                cx.notify();
            }
        });
        Self { browser: browser.downgrade(), main, reduce, shown: None, leaving: None, _watch: watch }
    }

    /// Selects card `ix` (the current one when `None`) in the main window's switcher, and switches
    /// to it when `commit`, giving the keyboard back to the main window.
    fn select(&self, ix: Option<usize>, commit: bool, cx: &mut Context<Self>) {
        if self.leaving.is_some() {
            return;
        }
        let (browser, main) = (self.browser.clone(), self.main);
        cx.defer(move |cx| {
            main.update(cx, |_, window, cx| {
                browser
                    .update(cx, |b, cx| {
                        let Some(open) = &mut b.tab_switcher else { return };
                        if let Some(key) = ix.and_then(|ix| open.state.order.get(ix).copied()) {
                            open.state.pick(key);
                        }
                        if commit {
                            b.commit_switcher(window, cx);
                            window.activate_window();
                        } else {
                            redraw(open_strip(b), cx);
                        }
                    })
                    .ok();
            })
            .ok();
        });
    }
}

impl Render for Strip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        if let Some(open) = b.tab_switcher.as_ref().filter(|_| self.leaving.is_none()) {
            self.shown = Some((open.state.clone(), open.shots.clone(), open.backdrop.clone()));
        }
        let Some((Switcher { order, index, .. }, shots, backdrop)) = &self.shown else { return div().into_any_element() };
        let glass = b.core.themes.borrow().glass;
        let fade = self.leaving.map_or(1., |at| 1. - progress(at, EXIT));
        if self.leaving.is_some() {
            window.request_animation_frame();
        }
        let text = |a: f32| Rgba { a, ..Theme::text() };
        let card = card_width(f32::from(window.viewport_size().width), order.len());
        let radius = (card * 0.06).clamp(10., 14.);
        let badge = (card * 0.15).clamp(26., 36.);
        let cards: Vec<_> = order
            .iter()
            .enumerate()
            .filter_map(|(ix, key)| {
                let tab = b.model.tab(*key)?;
                let selected = ix == *index;
                let still = div()
                    .relative()
                    .w(px(card))
                    .h(px(card * 10. / 16.))
                    .rounded(px(radius))
                    .overflow_hidden()
                    .bg(Theme::panel())
                    .border_1()
                    .border_color(text(0.12))
                    .shadow(Elevation::Page.shadow());
                let still = match shots.get(key) {
                    Some(shot) => still.child(img(shot.clone()).size_full().object_fit(ObjectFit::Cover)).child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .bottom(px(8.))
                            .size(px(badge))
                            .rounded(px(badge * 0.3))
                            .bg(Theme::surface())
                            .shadow(Elevation::Popover.shadow())
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::sidebar::site_icon(b, &tab.url, badge * 0.6)),
                    ),
                    None => still.flex().items_center().justify_center().child(crate::sidebar::site_icon(b, &tab.url, badge * 1.2)),
                };
                // A ring that stands off the card, so the still keeps its own edge.
                let ring = div()
                    .p(px(RING))
                    .rounded(px(radius + RING))
                    .border_2()
                    .border_color(if selected { Theme::accent() } else { Rgba { a: 0., ..Theme::accent() } })
                    .child(still);
                let title = div()
                    .mt(px(8.))
                    .px(px(RING + 2.))
                    .text_size(px(TEXT))
                    .text_color(if selected { Theme::text() } else { text(0.7) })
                    .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .one_line(crate::tabs::display_title(tab));
                Some(
                    div()
                        .id(("switcher-tab", ix))
                        .w(px(card + 2. * RING))
                        .flex()
                        .flex_col()
                        .cursor_pointer()
                        .when(!selected, |d| d.opacity(0.85))
                        // Moving the pointer selects, so a strip opening under a still pointer keeps Tab's choice.
                        .when(!selected, |d| d.on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.select(Some(ix), false, cx))))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(Some(ix), true, cx)))
                        .child(ring)
                        .child(title),
                )
            })
            .collect();
        let modifier = if cfg!(target_os = "macos") { "⌃" } else { "Ctrl" };
        let hint = div().mt(px(28.)).text_size(px(TEXT_SMALL)).text_color(text(0.55)).child(format!("Release {modifier} to switch  ·  Esc to stay"));
        // The page shows faintly through, while cards and titles stay clear of it in either theme.
        let scrim = div()
            .id("tab-switcher-scrim")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px(px(MARGIN))
            .relative()
            .bg(Rgba { a: if glass { 0.3 } else { 0.8 }, ..Theme::page() })
            .opacity(fade)
            .font_family(Theme::font())
            // A click makes this window key, so the Ctrl release can arrive here instead of the main window.
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, cx| {
                if !e.modifiers.modified() {
                    this.select(None, true, cx);
                }
            }))
            .children(backdrop.clone().filter(|_| glass).map(|image| {
                div()
                    .absolute()
                    .inset_0()
                    .child(img(image).size_full().object_fit(ObjectFit::Cover))
                    .child(div().absolute().inset_0().bg(Rgba { a: 0.45, ..Theme::page() }))
            }))
            .child(pop_in(div().flex().gap(px(GAP)).children(cards), "tab-switcher", self.reduce))
            .child(hint);
        fade_in(scrim, "tab-switcher-fade", self.reduce)
    }
}
