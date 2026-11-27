use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser_core::switcher::Switcher;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyWindowHandle, AppContext, Bounds, Context, Focusable, InteractiveElement, IntoElement, ModifiersChangedEvent, MouseMoveEvent, ObjectFit, ParentElement,
    Pixels, Render, RenderImage, Rgba, StatefulInteractiveElement, Styled, StyledImage, Subscription, WeakEntity, Window, WindowHandle, div, img, point, px,
    size,
};
use ui::motion::{fade_in, pop_in};
use ui::text::OneLine;
use ui::theme::{Elevation, TEXT, Theme};

use crate::browser::{Browser, TabKey};
use crate::prefs::Pref;

const CARD_WIDTH: f32 = 184.;
const SHOT_HEIGHT: f32 = CARD_WIDTH * 10. / 16.;
const RADIUS: f32 = 10.;
const RING: f32 = 5.;
const GAP: f32 = 20.;
/// A quick press and release switches without the strip flashing over the page.
const SHOW_DELAY: Duration = Duration::from_millis(150);

/// The open switcher and stills of its awake tabs, taken when the strip shows and dropped when it closes.
pub struct Open {
    pub state: Switcher,
    opened: Instant,
    shots: HashMap<TabKey, Arc<RenderImage>>,
    pub(crate) strip: Option<WindowHandle<Strip>>,
    _deactivated: Subscription,
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
            return cx.notify();
        }
        // As many cards as fit on one row of this window.
        let fits = ((f32::from(self.page_area(window).size.width) - 2. * GAP) / (CARD_WIDTH + 2. * RING + GAP)).floor().max(2.) as usize;
        let Some(state) = self.model.open_switcher(forward, fits) else { return };
        if !window.modifiers().modified() {
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
        self.tab_switcher = Some(Open { state, opened, shots: HashMap::new(), strip: None, _deactivated: deactivated });
        if let Some(host) = &self.host {
            host.focus();
        }
        window.focus(&self.focus_handle(cx));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SHOW_DELAY).await;
            this.update_in(cx, |b, window, cx| b.show_switcher(opened, window, cx)).ok();
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
        for key in open.state.order.clone() {
            self.request_switcher_shot(key, cx);
        }
        let mut bounds = self.page_area(window);
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

    fn request_switcher_shot(&self, key: TabKey, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key) else { return };
        let (tx, mut rx) = mpsc::unbounded();
        view.snapshot_png(Some(f64::from(CARD_WIDTH) * 2.), move |png| drop(tx.unbounded_send(png)));
        cx.spawn(async move |this, cx| {
            let Some(Some(png)) = rx.next().await else { return };
            let Some(image) = cx.background_spawn(async move { crate::hover_card::decode(&png) }).await else { return };
            this.update(cx, |b, cx| {
                if let Some(open) = &mut b.tab_switcher {
                    open.shots.insert(key, image);
                    cx.notify();
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
    _watch: Subscription,
}

impl Strip {
    fn new(browser: &gpui::Entity<Browser>, opened: Instant, main: AnyWindowHandle, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let watch = cx.observe_in(browser, window, move |_, browser, window, cx| {
            if browser.read(cx).tab_switcher.as_ref().is_none_or(|o| o.opened != opened) {
                window.remove_window();
            } else {
                cx.notify();
            }
        });
        Self { browser: browser.downgrade(), main, reduce, _watch: watch }
    }

    /// Selects card `ix` (the current one when `None`) in the main window's switcher, and switches
    /// to it when `commit`, giving the keyboard back to the main window.
    fn select(&self, ix: Option<usize>, commit: bool, cx: &mut Context<Self>) {
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
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .ok();
        });
    }
}

impl Render for Strip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tint = |a: f32| Rgba { a, ..Theme::text() };
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let Some(open) = &b.tab_switcher else { return div().into_any_element() };
        let cards: Vec<_> = open
            .state
            .order
            .iter()
            .enumerate()
            .filter_map(|(ix, key)| {
                let tab = b.model.tab(*key)?;
                let selected = ix == open.state.index;
                let badge = div()
                    .absolute()
                    .left(px(8.))
                    .bottom(px(8.))
                    .size(px(28.))
                    .rounded(px(7.))
                    .bg(Rgba { a: 0.85, ..Theme::surface() })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::sidebar::site_icon(b, &tab.url, 16.));
                let still = div()
                    .relative()
                    .w(px(CARD_WIDTH))
                    .h(px(SHOT_HEIGHT))
                    .rounded(px(RADIUS))
                    .overflow_hidden()
                    .bg(Theme::panel())
                    .border_1()
                    .border_color(tint(0.12))
                    .shadow(Elevation::Floating.shadow());
                let still = match open.shots.get(key) {
                    Some(shot) => still.child(img(shot.clone()).size_full().object_fit(ObjectFit::Cover)).child(badge),
                    None => still.flex().items_center().justify_center().child(crate::sidebar::site_icon(b, &tab.url, 28.)),
                };
                let ring = div()
                    .p(px(RING))
                    .rounded(px(RADIUS + RING))
                    .border_1()
                    .border_color(if selected { tint(0.3) } else { tint(0.) })
                    .when(selected, |d| d.bg(tint(0.06)))
                    .child(still);
                let title = div()
                    .mt(px(6.))
                    .px(px(RING + 4.))
                    .text_size(px(TEXT))
                    .text_color(if selected { Theme::text() } else { tint(0.6) })
                    .one_line(crate::tabs::display_title(tab));
                Some(
                    div()
                        .id(("switcher-tab", ix))
                        .w(px(CARD_WIDTH + 2. * RING))
                        .flex()
                        .flex_col()
                        .cursor_pointer()
                        // Moving the pointer selects, so a strip opening under a still pointer keeps Tab's choice.
                        .when(!selected, |d| d.on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.select(Some(ix), false, cx))))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(Some(ix), true, cx)))
                        .child(ring)
                        .child(title),
                )
            })
            .collect();
        // A click makes this window key, so the Ctrl release can arrive here instead of the main window.
        let scrim = div()
            .id("tab-switcher-scrim")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(Rgba { a: 0.3, ..Theme::surface() })
            .font_family(Theme::font())
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, cx| {
                if !e.modifiers.modified() {
                    this.select(None, true, cx);
                }
            }))
            .child(pop_in(div().flex().gap(px(GAP)).children(cards), "tab-switcher", self.reduce));
        fade_in(scrim, "tab-switcher-fade", self.reduce)
    }
}
