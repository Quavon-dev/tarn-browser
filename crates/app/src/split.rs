use std::cell::RefCell;
use std::rc::Rc;

use browser_core::split;
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

/// x, y, width, height in window points.
type Rect = (f32, f32, f32, f32);

/// Two to four tabs of a space shown at once for this session; the active tab is the focused pane.
pub struct Split {
    pub panes: Vec<TabKey>,
    pub(crate) ratio: f32,
    rects: Rc<RefCell<Vec<Rect>>>,
    _clicks: Option<engine::menu::EventMonitor>,
    _task: Task<()>,
}

impl Split {
    /// The other panes of the split that holds `key`.
    pub fn others(&self, key: TabKey) -> Vec<TabKey> {
        others(&self.panes, key)
    }
}

fn others(panes: &[TabKey], key: TabKey) -> Vec<TabKey> {
    if panes.contains(&key) { panes.iter().copied().filter(|k| *k != key).collect() } else { Vec::new() }
}

pub fn pane_at(rects: &[Rect], x: f32, y: f32) -> Option<usize> {
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

    /// Whether a tab dropped on `target`'s row may split with it: not inside one split, and not
    /// into a full one.
    pub(crate) fn drop_splits(&self, dragged: TabKey, target: TabKey) -> bool {
        dragged != target && self.split.as_ref().is_none_or(|s| !s.panes.contains(&target) || split::with_pane(&s.panes, dragged).is_some())
    }

    /// Whether `key` can join the split the active tab is in.
    pub(crate) fn can_join_split(&self, key: TabKey) -> bool {
        self.split.as_ref().is_some_and(|s| self.model.active.is_some_and(|a| s.panes.contains(&a)) && split::with_pane(&s.panes, key).is_some())
    }

    /// The tab menu's split entry for `key`.
    pub(crate) fn split_label(&self, key: TabKey) -> &'static str {
        match self.split.as_ref() {
            Some(s) if s.panes.contains(&key) && s.panes.len() > 2 => "Remove from Split View",
            Some(s) if s.panes.contains(&key) => "Close Split View",
            _ if self.can_join_split(key) => "Add to Split View",
            _ => "Open in Split View",
        }
    }

    /// Splits the active tab with `with`, else with the space's most recently used other tab,
    /// else with a copy of the active tab, and focuses the new pane.
    pub fn open_split(&mut self, with: Option<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.model.active else { return };
        let recent = self.model.tabs.iter().filter(|t| t.id != active.id).max_by_key(|t| t.last_active).map(|t| self.model.key(t));
        let partner = match with.filter(|k| *k != active).or(recent) {
            Some(partner) => partner,
            None => {
                self.close_split(cx);
                self.duplicate_tab(active, window, cx);
                match self.model.active.filter(|k| *k != active) {
                    Some(copy) => copy,
                    None => return,
                }
            }
        };
        self.show_split(vec![active, partner], 0.5, window, cx);
        self.activate(partner, window, cx);
    }

    /// Adds `key` to the active tab's split, or opens a split of the active tab and `key`.
    pub fn split_with(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.active == Some(key) {
            return self.open_split(None, window, cx);
        }
        if self.in_split(key) {
            return self.activate(key, window, cx);
        }
        match self.split.as_ref().filter(|_| self.can_join_split(key)).and_then(|s| split::with_pane(&s.panes, key).map(|p| (p, s.ratio))) {
            Some((panes, ratio)) => {
                self.show_split(panes, ratio, window, cx);
                self.activate(key, window, cx);
            }
            None => self.open_split(Some(key), window, cx),
        }
    }

    /// Takes `key` out of its split; a split left with one pane ends.
    pub fn leave_split(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panes, ratio)) = self.split.as_ref().map(|s| (others(&s.panes, key), s.ratio)) else { return };
        if panes.len() < 2 {
            return self.close_split(cx);
        }
        if let Some(view) = self.views.get(&key).filter(|_| self.model.active != Some(key)) {
            view.set_visible(false);
        }
        let focus = if self.model.active == Some(key) { panes.first().copied() } else { None };
        self.show_split(panes, ratio, window, cx);
        if let Some(focus) = focus {
            self.activate(focus, window, cx);
        }
    }

    /// Shows `panes` as the split, replacing any split before.
    pub(crate) fn show_split(&mut self, panes: Vec<TabKey>, ratio: f32, window: &mut Window, cx: &mut Context<Self>) {
        let kept: Vec<TabKey> = panes.clone();
        if let Some(old) = self.split.take() {
            for key in old.panes.into_iter().filter(|k| !kept.contains(k) && Some(*k) != self.model.active) {
                if let Some(view) = self.views.get(&key) {
                    view.set_visible(false);
                }
            }
        }
        let rects: Rc<RefCell<Vec<Rect>>> = Rc::default();
        let (tx, mut rx) = mpsc::unbounded();
        let hit = rects.clone();
        let clicks = self.host.as_ref().and_then(|h| {
            h.on_mouse_down(move |x, y| {
                if let Some(ix) = pane_at(&hit.borrow(), x as f32, y as f32) {
                    tx.unbounded_send(ix).ok();
                }
            })
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(ix) = rx.next().await {
                let focused = this.update_in(cx, |b, window, cx| {
                    let key = b
                        .split
                        .as_ref()
                        .and_then(|s| s.panes.get(ix).copied())
                        .filter(|k| b.model.active != Some(*k) && !b.overlay_open() && b.active_json().is_none());
                    if let Some(key) = key {
                        b.activate(key, window, cx);
                    }
                });
                if focused.is_err() {
                    break;
                }
            }
        });
        self.split = Some(Split { panes, ratio, rects, _clicks: clicks, _task: task });
        self.page_fade = None;
        let shown = !self.overlay_open();
        for key in kept {
            if let Some(view) = self.views.get(&key) {
                view.set_visible(shown);
            }
        }
        cx.notify();
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

    /// Ends the split once the focus leaves it or a pane's page is gone; moves the other panes out
    /// of sight while the page is covered.
    pub(crate) fn settle_split(&mut self, cx: &mut Context<Self>) {
        let Some(split) = &self.split else { return };
        // The start page hides its tab for a moment; Esc brings it back with its partners.
        let focus = self.model.active.or(self.launcher_from).filter(|a| split.panes.contains(a));
        let gone: Vec<TabKey> = split.panes.iter().copied().filter(|k| !self.views.contains_key(k)).collect();
        if focus.is_none() || split.panes.len() - gone.len() < 2 {
            return self.close_split(cx);
        }
        if !gone.is_empty() {
            // A closed pane leaves the others in the split.
            if let Some(split) = self.split.as_mut() {
                split.panes.retain(|k| !gone.contains(k));
                split.rects.borrow_mut().truncate(split.panes.len());
            }
            cx.notify();
        }
        let Some(split) = &self.split else { return };
        let blocked = self.model.active.is_some_and(|k| self.pages.get(&k).is_some_and(|p| p.blocked.is_some()));
        if self.overlay_open() || self.active_json().is_some() || blocked {
            for key in split.others(focus.unwrap_or(split.panes[0])) {
                if let Some(view) = self.views.get(&key) {
                    view.set_frame(0., 0., 0., 0.);
                }
            }
        }
    }
}

pub fn render(b: &Browser, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let split = b.split.as_ref()?;
    let count = split.panes.len();
    split.rects.borrow_mut().resize(count, (0., 0., 0., 0.));
    let pane = |ix: usize, key: TabKey| {
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
                if let Some(rect) = rects.borrow_mut().get_mut(ix) {
                    *rect = (x, y, w, h);
                }
            },
            |_, _, _, _| {},
        )
        .size_full();
        div()
            .size_full()
            .rounded(px(Theme::radius_page()))
            .bg(Theme::page())
            .border_1()
            .border_color(if b.model.active == Some(key) { Theme::accent() } else { Theme::hairline() })
            .shadow(Theme::elevation(6., 24.))
            .child(page)
    };
    let cells = split.panes.iter().zip(split::layout(count, split.ratio)).enumerate().map(|(ix, (key, (x, y, w, h)))| {
        div().absolute().left(relative(x)).top(relative(y)).w(relative(w)).h(relative(h)).p(px(GAP / 2.)).child(pane(ix, *key))
    });
    let divider = div()
        .id("split-divider")
        .absolute()
        .top_0()
        .bottom_0()
        .left(relative(split::layout(count, split.ratio)[0].2))
        .ml(px(-GAP / 2.))
        .w(px(GAP))
        .flex()
        .items_center()
        .justify_center()
        .cursor_col_resize()
        .child(div().w(px(3.)).h(px(32.)).rounded_full().bg(Theme::faint()))
        .on_drag(DividerDrag, |d, _, _, cx| cx.new(|_| d.clone()));
    let area = div()
        .id("split")
        .flex_1()
        .min_h_0()
        .relative()
        .mx(px(-GAP / 2.))
        .my(px(-GAP / 2.))
        .on_drag_move(cx.listener(|b, e: &DragMoveEvent<DividerDrag>, _, cx| {
            let ratio = split::ratio_at(f32::from(e.event.position.x - e.bounds.origin.x), f32::from(e.bounds.size.width));
            if let Some(split) = b.split.as_mut() {
                split.ratio = ratio;
                cx.notify();
            }
        }))
        .children(cells)
        .child(divider);
    Some(area.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_pick_the_pane_under_the_pointer() {
        let rects = [(10., 50., 400., 600.), (418., 50., 400., 600.)];
        assert_eq!(pane_at(&rects, 10., 50.), Some(0));
        assert_eq!(pane_at(&rects, 409.9, 649.), Some(0));
        assert_eq!(pane_at(&rects, 414., 300.), None, "the gap belongs to neither pane");
        assert_eq!(pane_at(&rects, 418., 300.), Some(1));
        assert_eq!(pane_at(&rects, 500., 20.), None, "the toolbar is outside both panes");
    }

    #[test]
    fn others_are_the_rest_of_the_split() {
        let (a, b, c, d) =
            (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 }, TabKey { private: true, id: 1 }, TabKey { private: false, id: 3 });
        assert_eq!(others(&[a, b, d], a), [b, d]);
        assert_eq!(others(&[a, b], b), [a]);
        assert!(others(&[a, b], c).is_empty(), "a private tab with the same id is another tab");
    }
}
