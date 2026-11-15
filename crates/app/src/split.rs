use std::cell::Cell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, AppContext, Context, Div, DragMoveEvent, Empty, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Task, Window, actions, canvas, div, px, relative,
};
use ui::theme::Theme;

use crate::browser::{Browser, TabKey};

actions!(tarn, [ToggleSplitView]);

pub const GAP: f32 = 8.;
const MIN_RATIO: f32 = 0.2;

/// x, y, width, height in window points.
type Rect = (f32, f32, f32, f32);

/// Two tabs of a space shown side by side for this session; the active tab is the focused pane.
pub struct Split {
    pub panes: [TabKey; 2],
    pub(crate) ratio: f32,
    rects: Rc<Cell<[Rect; 2]>>,
    _clicks: Option<engine::menu::EventMonitor>,
    _task: Task<()>,
}

impl Split {
    pub fn partner(&self, key: TabKey) -> Option<TabKey> {
        partner(self.panes, key)
    }
}

fn partner(panes: [TabKey; 2], key: TabKey) -> Option<TabKey> {
    match panes {
        [a, b] if a == key => Some(b),
        [a, b] if b == key => Some(a),
        _ => None,
    }
}

/// The left pane's share for a divider dragged to `x` in a split `width` wide.
pub fn ratio_at(x: f32, width: f32) -> f32 {
    ((x - GAP / 2.) / width.max(1.)).clamp(MIN_RATIO, 1. - MIN_RATIO)
}

pub fn pane_at(rects: &[Rect; 2], x: f32, y: f32) -> Option<usize> {
    rects.iter().position(|(rx, ry, w, h)| x >= *rx && x < rx + w && y >= *ry && y < ry + h)
}

#[derive(Clone)]
struct DividerDrag;

impl Render for DividerDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ToggleSplitView, w, cx| match b.split.is_some() {
        true => b.close_split(cx),
        false => b.open_split(None, w, cx),
    }))
}

impl Browser {
    pub fn in_split(&self, key: TabKey) -> bool {
        self.split.as_ref().is_some_and(|s| s.panes.contains(&key))
    }

    /// Splits the active tab with `with`, else with the space's most recently used other tab,
    /// else with a copy of the active tab, and focuses the new pane.
    pub fn open_split(&mut self, with: Option<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        self.close_split(cx);
        let Some(active) = self.model.active else { return };
        let recent = self.model.tabs.iter().filter(|t| t.id != active.id).max_by_key(|t| t.last_active).map(|t| self.model.key(t));
        let partner = match with.filter(|k| *k != active).or(recent) {
            Some(partner) => partner,
            None => {
                self.duplicate_tab(active, window, cx);
                match self.model.active.filter(|k| *k != active) {
                    Some(copy) => copy,
                    None => return,
                }
            }
        };
        let rects: Rc<Cell<[Rect; 2]>> = Rc::default();
        let (tx, mut rx) = mpsc::unbounded();
        let hit = rects.clone();
        let clicks = self.host.as_ref().and_then(|h| {
            h.on_mouse_down(move |x, y| {
                if let Some(ix) = pane_at(&hit.get(), x as f32, y as f32) {
                    tx.unbounded_send(ix).ok();
                }
            })
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(ix) = rx.next().await {
                let focused = this.update_in(cx, |b, window, cx| {
                    let key = b.split.as_ref().map(|s| s.panes[ix]).filter(|k| b.model.active != Some(*k) && !b.overlay_open() && b.active_json().is_none());
                    if let Some(key) = key {
                        b.activate(key, window, cx);
                    }
                });
                if focused.is_err() {
                    break;
                }
            }
        });
        self.split = Some(Split { panes: [active, partner], ratio: 0.5, rects, _clicks: clicks, _task: task });
        self.activate(partner, window, cx);
        self.page_fade = None;
        if let Some(view) = self.views.get(&active) {
            view.set_visible(!self.overlay_open());
        }
    }

    pub fn close_split(&mut self, cx: &mut Context<Self>) {
        let Some(split) = self.split.take() else { return };
        let fading = self.page_fade.as_ref().map(|(view, _)| view);
        for key in split.panes.into_iter().filter(|k| Some(*k) != self.model.active) {
            if let Some(view) = self.views.get(&key).filter(|v| !fading.is_some_and(|f| Rc::ptr_eq(f, v))) {
                view.set_visible(false);
            }
        }
        cx.notify();
    }

    /// Ends the split once the focus leaves it or a pane's page is gone; moves the other pane out
    /// of sight while the page is covered.
    pub(crate) fn settle_split(&mut self, cx: &mut Context<Self>) {
        let Some(split) = &self.split else { return };
        let other = self.model.active.and_then(|a| split.partner(a));
        let Some(view) = other.filter(|_| split.panes.iter().all(|k| self.views.contains_key(k))).and_then(|k| self.views.get(&k)) else {
            return self.close_split(cx);
        };
        let blocked = self.model.active.is_some_and(|k| self.pages.get(&k).is_some_and(|p| p.blocked.is_some()));
        if self.overlay_open() || self.active_json().is_some() || blocked {
            view.set_frame(0., 0., 0., 0.);
        }
    }
}

pub fn render(b: &Browser, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let split = b.split.as_ref()?;
    let pane = |ix: usize| {
        let key = split.panes[ix];
        let view = b.views.get(&key).cloned();
        // A pane whose page was stopped by the gate stays out of sight; focusing it shows the warning.
        let blocked = b.pages.get(&key).is_some_and(|p| p.blocked.is_some());
        let rects = split.rects.clone();
        let page = canvas(
            move |bounds, _, _| {
                let Some(view) = &view else { return };
                let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                view.set_alpha(1.);
                if blocked {
                    view.set_frame(0., 0., 0., 0.);
                } else {
                    view.set_frame(x.into(), y.into(), w.into(), h.into());
                }
                let mut all = rects.get();
                all[ix] = (x, y, w, h);
                rects.set(all);
            },
            |_, _, _, _| {},
        )
        .size_full();
        div()
            .h_full()
            .min_w_0()
            .rounded(px(Theme::radius_page()))
            .bg(Theme::page())
            .border_1()
            .border_color(if b.model.active == Some(key) { Theme::accent() } else { Theme::hairline() })
            .shadow(Theme::elevation(6., 24.))
            .child(page)
    };
    let divider = div()
        .id("split-divider")
        .flex_none()
        .w(px(GAP))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_col_resize()
        .child(div().w(px(3.)).h(px(32.)).rounded_full().bg(Theme::faint()))
        .on_drag(DividerDrag, |d, _, _, cx| cx.new(|_| d.clone()));
    let row = div()
        .id("split")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_row()
        .on_drag_move(cx.listener(|b, e: &DragMoveEvent<DividerDrag>, _, cx| {
            let ratio = ratio_at(f32::from(e.event.position.x - e.bounds.origin.x), f32::from(e.bounds.size.width));
            if let Some(split) = b.split.as_mut() {
                split.ratio = ratio;
                cx.notify();
            }
        }))
        .child(pane(0).flex_none().w(relative(split.ratio)))
        .child(divider)
        .child(pane(1).flex_1());
    Some(row.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divider_ratio_follows_the_pointer_within_limits() {
        assert_eq!(ratio_at(500. + GAP / 2., 1000.), 0.5);
        assert_eq!(ratio_at(300. + GAP / 2., 1000.), 0.3);
        assert_eq!(ratio_at(10., 1000.), MIN_RATIO);
        assert_eq!(ratio_at(990., 1000.), 1. - MIN_RATIO);
        assert_eq!(ratio_at(0., 0.), MIN_RATIO);
    }

    #[test]
    fn clicks_pick_the_pane_under_the_pointer() {
        let rects = [(10., 50., 400., 600.), (418., 50., 400., 600.)];
        assert_eq!(pane_at(&rects, 10., 50.), Some(0));
        assert_eq!(pane_at(&rects, 409.9, 649.), Some(0));
        assert_eq!(pane_at(&rects, 414., 300.), None, "the divider belongs to neither pane");
        assert_eq!(pane_at(&rects, 418., 300.), Some(1));
        assert_eq!(pane_at(&rects, 500., 20.), None, "the toolbar is outside both panes");
    }

    #[test]
    fn partner_is_the_other_pane() {
        let (a, b, c) = (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 }, TabKey { private: true, id: 1 });
        assert_eq!(partner([a, b], a), Some(b));
        assert_eq!(partner([a, b], b), Some(a));
        assert_eq!(partner([a, b], c), None, "a private tab with the same id is another tab");
    }
}
