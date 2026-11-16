use std::cell::RefCell;
use std::rc::Rc;

use browser_core::split::{self, Divider};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, ClickEvent, Context, Div, DragMoveEvent, Empty, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Task, Window, actions, canvas, div, px, relative,
};
use ui::controls::{icon_button, tooltip};
use ui::icon::Icon;
use ui::theme::{Elevation, TEXT_SMALL, Theme};
use url::Url;

use crate::browser::{Browser, TabKey};
use crate::sidebar::DraggedTab;

actions!(tarn, [ToggleSplitView, FocusNextPane, FocusPreviousPane, SwapPanes, RotateSplitLayout, RemoveFromSplitView, CloseSplitView]);

pub const GAP: f32 = 8.;
const HEADER: f32 = 30.;
/// The narrowest pane whose header names the host beside the title.
const HOST_ROOM: f32 = 380.;

/// What the shell keeps of the split on screen: its panes, their page rects in window points and
/// the watcher of clicks into them.
#[derive(Default)]
pub struct Shown {
    panes: Vec<TabKey>,
    rects: Rc<RefCell<Vec<split::Rect>>>,
    _clicks: Option<(engine::menu::EventMonitor, Task<()>)>,
}

#[derive(Clone)]
struct DividerDrag(Divider);

impl Render for DividerDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ToggleSplitView, w, cx| match b.model.active.filter(|a| b.splits.contains(*a)) {
        Some(active) => b.close_split(active, cx),
        None => b.open_split(w, cx),
    }))
    .on_action(cx.listener(|b, _: &FocusNextPane, w, cx| b.focus_pane(true, w, cx)))
    .on_action(cx.listener(|b, _: &FocusPreviousPane, w, cx| b.focus_pane(false, w, cx)))
    .on_action(cx.listener(|b, _: &SwapPanes, _, cx| {
        if let Some((active, next)) = b.model.active.and_then(|a| Some((a, b.splits.of(a)?.next(a, true)?))) {
            b.swap_panes(active, next, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &RotateSplitLayout, _, cx| {
        if let Some(active) = b.model.active {
            b.rotate_split(active, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &RemoveFromSplitView, w, cx| {
        if let Some(active) = b.model.active {
            b.leave_split(active, w, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &CloseSplitView, _, cx| {
        if let Some(active) = b.model.active {
            b.close_split(active, cx);
        }
    }))
}

impl Browser {
    /// The split on screen: the active tab's, or the one the start page covers for a moment.
    pub(crate) fn shown_split(&self) -> Option<&split::Split<TabKey>> {
        self.model.active.or(self.launcher_from).and_then(|k| self.splits.of(k))
    }

    /// Whether `key` is a pane of the split on screen; a split out of sight sleeps like any tab.
    pub fn in_split(&self, key: TabKey) -> bool {
        self.shown_split().is_some_and(|s| s.panes().contains(&key))
    }

    /// The other panes of `key`'s split, once those whose tab is gone or shown in another window
    /// have left it.
    pub(crate) fn split_partners(&mut self, key: TabKey, cx: &mut Context<Self>) -> Vec<TabKey> {
        let (model, views) = (&self.model, &self.views);
        self.splits.partners(key, |pane| model.tab(pane).is_some() && (views.contains_key(&pane) || !crate::shared::live_elsewhere(pane, cx)))
    }

    /// Splits the active tab with the space's most recently used tab outside a split, else with a
    /// copy of the active tab, and focuses the new pane.
    pub fn open_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.model.active.filter(|a| !self.splits.contains(*a)) else { return };
        let tabs = self.model.tabs.iter().map(|t| (self.model.key(t), t.last_active));
        let recent = self.splits.partner(active, tabs.filter(|(k, _)| self.views.contains_key(k) || !crate::shared::live_elsewhere(*k, cx)));
        let partner = match recent {
            Some(partner) => partner,
            // The burner has one tab, and a copy would load in it instead of beside it.
            None if self.model.burner_shown() => return self.show_toast("Split View needs a second tab", None, cx),
            None => {
                self.duplicate_tab(active, window, cx);
                match self.model.active.filter(|k| *k != active) {
                    Some(copy) => copy,
                    None => return,
                }
            }
        };
        self.join_split(active, partner, window, cx);
    }

    /// Adds `key` to the active tab's split or starts one with the two; the active tab itself gets
    /// a partner.
    pub fn split_with(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        match self.model.active.filter(|a| *a != key) {
            Some(active) => self.join_split(active, key, window, cx),
            None => self.open_split(window, cx),
        }
    }

    /// Puts `key` into `anchor`'s split, or starts one with the two, and focuses `key`.
    fn join_split(&mut self, anchor: TabKey, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.splits.join(anchor, key) {
            self.activate(key, window, cx);
        }
    }

    /// A tab dropped on a pane, or on the middle of its row: two panes of one split swap, any
    /// other tab joins.
    pub(crate) fn drop_on_pane(&mut self, target: TabKey, dragged: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if !self.swap_panes(dragged, target, cx) {
            self.join_split(target, dragged, window, cx);
        }
    }

    /// Takes `key` out of its split; the tab stays open and a split left with one pane ends.
    pub fn leave_split(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        let heir = self.splits.leave(key).filter(|_| self.model.active == Some(key));
        if let Some(heir) = heir {
            self.activate(heir, window, cx);
        }
        self.split_moved(cx);
    }

    pub fn close_split(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.splits.close(key) {
            self.split_moved(cx);
        }
    }

    pub fn rotate_split(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.splits.rotate(key) {
            self.split_moved(cx);
        }
    }

    fn swap_panes(&mut self, a: TabKey, b: TabKey, cx: &mut Context<Self>) -> bool {
        let swapped = self.splits.swap(a, b);
        if swapped {
            self.split_moved(cx);
        }
        swapped
    }

    fn focus_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.model.active.and_then(|a| self.splits.of(a)?.next(a, forward)) {
            self.switch_to(next, window, cx);
        }
    }

    /// Redraws after panes moved; a blocking page's guarded button waits for the layout to rest.
    fn split_moved(&mut self, cx: &mut Context<Self>) {
        self.layout_changed();
        cx.notify();
    }

    /// Focuses the pane whose page is clicked: a web view takes the click itself, so GPUI never
    /// sees it.
    pub(crate) fn watch_split_clicks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, mut rx) = mpsc::unbounded();
        let rects = self.split_shown.rects.clone();
        let Some(clicks) = self.host.as_ref().and_then(|h| {
            h.on_mouse_down(move |x, y| {
                if let Some(ix) = split::pane_at(&rects.borrow(), x as f32, y as f32) {
                    tx.unbounded_send(ix).ok();
                }
            })
        }) else {
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(ix) = rx.next().await {
                let focused = this.update_in(cx, |b, window, cx| {
                    let key = b.shown_split().and_then(|s| s.panes().get(ix).copied()).filter(|k| b.model.active != Some(*k) && !b.overlay_open());
                    if let Some(key) = key {
                        b.activate(key, window, cx);
                    }
                });
                if focused.is_err() {
                    break;
                }
            }
        });
        self.split_shown._clicks = Some((clicks, task));
    }

    /// Hides the panes that left the screen, and moves the rest out of sight while something
    /// covers the page.
    pub(crate) fn settle_split(&mut self) {
        let shown: Vec<TabKey> = self.shown_split().map(|s| s.panes().to_vec()).unwrap_or_default();
        let focus = self.model.active.or(self.launcher_from);
        let fading = self.page_fade.as_ref().map(|(view, _)| view);
        for key in self.split_shown.panes.iter().filter(|k| !shown.contains(k) && Some(**k) != focus) {
            if let Some(view) = self.views.get(key).filter(|v| !fading.is_some_and(|f| Rc::ptr_eq(f, v))) {
                view.set_visible(false);
            }
        }
        if shown.is_empty() || self.overlay_open() {
            self.split_shown.rects.borrow_mut().clear();
            for key in shown.iter().filter(|k| Some(**k) != focus) {
                if let Some(view) = self.views.get(key) {
                    view.set_frame(0., 0., 0., 0.);
                }
            }
        }
        self.split_shown.panes = shown;
    }
}

pub fn render(b: &Browser, window: &Window, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let split = b.shown_split()?;
    b.split_shown.rects.borrow_mut().resize(split.panes().len(), (0., 0., 0., 0.));
    let width = f32::from(window.viewport_size().width) - b.sidebar_width * b.sidebar_shown();
    let splits = Rc::new(b.splits.clone());
    let cells: Vec<Div> = split
        .panes()
        .iter()
        .zip(split.layout())
        .enumerate()
        .map(|(ix, (key, (x, y, w, h)))| {
            let card = pane(b, &splits, ix, *key, w * width >= HOST_ROOM, cx);
            div().absolute().left(relative(x)).top(relative(y)).w(relative(w)).h(relative(h)).p(px(GAP / 2.)).child(card)
        })
        .collect();
    let grips: Vec<AnyElement> = split.dividers().into_iter().map(|(divider, line)| grip(divider, line, cx)).collect();
    let area = div()
        .id("split")
        .flex_1()
        .min_h_0()
        .relative()
        .mx(px(-GAP / 2.))
        .my(px(-GAP / 2.))
        .on_drag_move(cx.listener(|b, e: &DragMoveEvent<DividerDrag>, _, cx| {
            let (at, size) = (e.event.position - e.bounds.origin, e.bounds.size);
            let divider = e.drag(cx).0;
            let share = match divider {
                Divider::Column => split::ratio_at(f32::from(at.x), f32::from(size.width)),
                Divider::Row => split::ratio_at(f32::from(at.y), f32::from(size.height)),
            };
            if b.model.active.is_some_and(|a| b.splits.resize(a, divider, share)) {
                b.split_moved(cx);
            }
        }))
        .children(cells)
        .children(grips);
    Some(area.into_any_element())
}

/// The handle on a divider: dragging resizes, a double click evens the panes out again.
fn grip(divider: Divider, (x, y, w, h): split::Share, cx: &mut Context<Browser>) -> AnyElement {
    let column = divider == Divider::Column;
    let name = if column { "split-column" } else { "split-row" };
    let pill =
        div().rounded_full().bg(Theme::faint()).group_hover(name, |s| s.bg(Theme::accent())).group_drag_over::<DividerDrag>(name, |s| s.bg(Theme::accent()));
    div()
        .id(name)
        .group(name)
        .absolute()
        .left(relative(x))
        .top(relative(y))
        .flex()
        .items_center()
        .justify_center()
        .map(|d| match column {
            true => d.w(px(GAP)).h(relative(h)).ml(px(-GAP / 2.)).cursor_col_resize().child(pill.w(px(3.)).h(px(32.))),
            false => d.w(relative(w)).h(px(GAP)).mt(px(-GAP / 2.)).cursor_row_resize().child(pill.w(px(32.)).h(px(3.))),
        })
        .on_drag(DividerDrag(divider), |d, _, _, cx| cx.new(|_| d.clone()))
        .on_click(cx.listener(|b, e: &ClickEvent, _, cx| {
            if e.click_count() == 2 && b.model.active.is_some_and(|a| b.splits.equalise(a)) {
                b.split_moved(cx);
            }
        }))
        .into_any_element()
}

/// One pane's card: its header, then the page. A web view is a native view above GPUI, so the
/// header has a height of its own and the page starts below it.
fn pane(b: &Browser, splits: &Rc<split::Splits<TabKey>>, ix: usize, key: TabKey, roomy: bool, cx: &mut Context<Browser>) -> Div {
    let focused = b.model.active == Some(key);
    let tab = b.model.tab(key);
    let title: SharedString = tab.map(crate::tabs::display_title).unwrap_or_default().into();
    let host = b.tab_url(key).and_then(|u| Url::parse(&u).ok()).map(|u| browser_core::command::address(&u).0).filter(|_| roomy);
    let keys = |action: &dyn gpui::Action| focused.then(|| SharedString::from(crate::shortcuts::keys(action, cx)));
    let leave = icon_button(("pane-leave", ix), Icon::ArrowUpRight, true).tooltip(tooltip("Remove from Split View", keys(&RemoveFromSplitView)));
    let close = icon_button(("pane-close", ix), Icon::Close, true).tooltip(tooltip("Close Tab", keys(&crate::CloseTab)));
    let header = div()
        .id(("pane-header", ix))
        .h(px(HEADER))
        .flex_none()
        .pl_2p5()
        .pr_0p5()
        .flex()
        .items_center()
        .gap_2()
        .rounded_t(px(Theme::radius_page() - 1.))
        .border_b_1()
        .border_color(Theme::hairline())
        .text_color(if focused { Theme::text() } else { Theme::muted() })
        .when(focused, |d| d.bg(Theme::accent_soft()).font_weight(FontWeight::MEDIUM))
        .cursor_pointer()
        .children(tab.map(|t| crate::sidebar::site_icon(b, &t.url, 14.)))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title.clone()))
        .children(host.map(|host| crate::toolbar::tail_first(div().flex_none().max_w(relative(0.4)).text_size(px(TEXT_SMALL)), host, Theme::faint())))
        .child(
            div()
                .flex()
                .flex_none()
                .when(!focused, |d| d.opacity(0.).group_hover("pane", |s| s.opacity(1.)))
                .child(leave.on_click(cx.listener(move |b, _, window, cx| {
                    cx.stop_propagation();
                    b.leave_split(key, window, cx);
                })))
                .child(close.on_click(cx.listener(move |b, _, window, cx| {
                    cx.stop_propagation();
                    b.close_tab(key, window, cx);
                }))),
        )
        .on_click(cx.listener(move |b, _, window, cx| b.switch_to(key, window, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, window, cx| b.tab_menu(key, window, cx)))
        .on_drag(DraggedTab { key, title }, |d, _, _, cx| cx.new(|_| d.clone()));

    let view = b.views.get(&key).cloned();
    let blocked = b.pages.get(&key).and_then(|p| p.blocked.as_ref());
    let json = b.json_pages.get(&key);
    let covered = blocked.is_some() || json.is_some();
    let rects = b.split_shown.rects.clone();
    let splits = splits.clone();
    let page = canvas(
        move |bounds, _, _| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            if let Some(rect) = rects.borrow_mut().get_mut(ix) {
                *rect = (x, y, w, h);
            }
            let Some(view) = &view else { return };
            view.set_alpha(1.);
            // The pane draws a page the gate stopped, and JSON, itself; the web view stays out of sight.
            if covered {
                view.set_frame(0., 0., 0., 0.);
            } else {
                view.set_frame(x.into(), y.into(), w.into(), h.into());
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();
    let body = div()
        .relative()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(page)
        .children(match (blocked, json) {
            (Some(blocked), _) => Some(crate::blocking::render_blocked(blocked, cx)),
            (None, Some(json)) => Some(crate::devtools::render_json(key, json, cx)),
            (None, None) => None,
        })
        // A warning's buttons act on the focused pane, so a click on another pane's warning only focuses it.
        .when(blocked.is_some() && !focused, |d| {
            d.child(div().id(("pane-cover", ix)).absolute().inset_0().occlude().on_click(cx.listener(move |b, _, window, cx| b.switch_to(key, window, cx))))
        });
    div()
        .group("pane")
        .size_full()
        .flex()
        .flex_col()
        .rounded(px(Theme::radius_page()))
        .bg(Theme::page())
        .border_1()
        .border_color(if focused { Theme::accent() } else { Theme::hairline() })
        .shadow(Elevation::Page.shadow())
        .drag_over::<DraggedTab>(move |s, dragged, _, _| if splits.drop_label(key, dragged.key).is_some() { s.border_color(Theme::accent()) } else { s })
        .on_drop(cx.listener(move |b, dragged: &DraggedTab, window, cx| b.drop_on_pane(key, dragged.key, window, cx)))
        .child(header)
        .child(body)
}
