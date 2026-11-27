use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser_core::switcher::Switcher;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ObjectFit, ParentElement, RenderImage, StatefulInteractiveElement,
    Styled, StyledImage, Subscription, Window, div, img, px,
};
use ui::controls::surface;
use ui::text::OneLine;
use ui::theme::{TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};

const CARD_WIDTH: f32 = 168.;
const SHOT_HEIGHT: f32 = 105.;
/// A quick press and release switches without the strip flashing over the page.
const SHOW_DELAY: Duration = Duration::from_millis(150);

/// The open switcher and stills of its awake tabs, taken when the strip shows and dropped when it closes.
pub struct Open {
    pub state: Switcher,
    shown: bool,
    opened: Instant,
    shots: HashMap<TabKey, Arc<RenderImage>>,
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
        let Some(state) = self.model.open_switcher(forward) else { return };
        if !window.modifiers().modified() {
            return self.switch_to(state.selected(), window, cx);
        }
        self.close_command(window, cx);
        // A release that happens in another window or app never reaches this one.
        let opened = Instant::now();
        let deactivated = cx.observe_window_activation(window, |b, window, cx| {
            if !window.is_window_active() {
                b.cancel_switcher(window, cx);
            }
        });
        self.tab_switcher = Some(Open { state, shown: false, opened, shots: HashMap::new(), _deactivated: deactivated });
        if let Some(host) = &self.host {
            host.focus();
        }
        window.focus(&self.focus_handle(cx));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOW_DELAY).await;
            this.update(cx, |b, cx| b.show_switcher(opened, cx)).ok();
        })
        .detach();
    }

    fn show_switcher(&mut self, opened: Instant, cx: &mut Context<Self>) {
        let Some(open) = self.tab_switcher.as_mut().filter(|o| !o.shown && o.opened == opened) else { return };
        open.shown = true;
        let order = open.state.order.clone();
        for view in self.views.values() {
            view.set_visible(false);
        }
        for key in order {
            self.request_switcher_shot(key, cx);
        }
        cx.notify();
    }

    pub(crate) fn switcher_shown(&self) -> bool {
        self.tab_switcher.as_ref().is_some_and(|o| o.shown)
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

pub fn render(b: &Browser, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let open = b.tab_switcher.as_ref().filter(|o| o.shown)?;
    let cards = open.state.order.iter().enumerate().filter_map(|(ix, key)| {
        let tab = b.model.tab(*key)?;
        let selected = ix == open.state.index;
        let still = div()
            .w(px(CARD_WIDTH))
            .h(px(SHOT_HEIGHT))
            .rounded(px(10.))
            .overflow_hidden()
            .bg(Theme::panel())
            .border_2()
            .border_color(if selected { Theme::accent() } else { Theme::surface_edge() })
            .flex()
            .items_center()
            .justify_center();
        let still = match open.shots.get(key) {
            Some(shot) => still.child(img(shot.clone()).size_full().object_fit(ObjectFit::Cover)),
            None => still.child(crate::sidebar::site_icon(b, &tab.url, 32.)),
        };
        let key = *key;
        Some(
            div()
                .id(("switcher-tab", ix))
                .w(px(CARD_WIDTH))
                .flex()
                .flex_col()
                .gap_1()
                .cursor_pointer()
                .on_click(cx.listener(move |b, _, window, cx| {
                    if let Some(open) = &mut b.tab_switcher {
                        open.state.pick(key);
                    }
                    b.commit_switcher(window, cx);
                }))
                .child(still)
                .child(
                    div()
                        .px_1()
                        .text_size(px(TEXT_SMALL))
                        .text_color(if selected { Theme::text() } else { Theme::muted() })
                        .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                        .one_line(crate::tabs::display_title(tab)),
                ),
        )
    });
    let strip = surface(16.).p_4().flex().flex_wrap().justify_center().gap_4().max_w(px((CARD_WIDTH + 16.) * 5. + 32.)).children(cards);
    Some(div().id("tab-switcher").flex_1().min_h_0().flex().items_center().justify_center().p_6().child(strip).into_any_element())
}
