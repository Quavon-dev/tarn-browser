use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, AppContext, Background, Bounds, ClickEvent, Context, Div, DragMoveEvent, Empty, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseMoveEvent, ParentElement, Pixels, Point, Render, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledImage, Task, Transformation,
    Window, canvas, div, img, percentage, prelude::FluentBuilder, px, rgb,
};
use store::{Folder, Group, Id, Tab, TabKind};
use ui::controls::{button, favicon, field, icon_button, spinner, tooltip};
use ui::icon::{ICON, ICON_SMALL, Icon, icon};
use ui::motion::{EXIT, FADE, POP, SLIDE, animate, fade_in, progress};
use ui::text::bounded_title;
use ui::theme::{Elevation, PROFILE_COLORS, TEXT, TEXT_SMALL, TEXT_TINY, TOOLBAR_HEIGHT, TRAFFIC_LIGHT_INSET, Theme};

use crate::browser::{Browser, SpaceRef, TabKey, folder_tree, tab_host};
use crate::gestures::{Step, follow};
use crate::groups::RenameTarget;
use crate::peek::GRACE;
use crate::prefs::Pref;

/// The default; the live width is `Browser::sidebar_width`.
pub const SIDEBAR_WIDTH: f32 = 256.;
pub const SIDEBAR_MARGIN: f32 = 6.;
pub(crate) const HEADER_HEIGHT: f32 = TOOLBAR_HEIGHT - SIDEBAR_MARGIN;
/// Where the header starts, clear of the window's traffic lights.
const NOTCH_WIDTH: f32 = TRAFFIC_LIGHT_INSET - SIDEBAR_MARGIN - 4.;
pub(crate) const TILE_GAP: f32 = 8.;
const ROW_GAP: f32 = 1.;
/// Sidebar rows are tighter than `ROW_HEIGHT`, so more tabs fit.
const TAB_ROW: f32 = 30.;
const CLOSE_WIDTH: f32 = 28.;
/// The row's `gap_2p5`.
const CLOSE_GAP: f32 = 10.;
const INNER_PAD: f32 = 3.;
/// A drag the fingers stop feeding (a cancelled gesture, a hidden sidebar) springs back after this.
const DRAG_IDLE: Duration = Duration::from_secs(2);

/// Sidebar rows that are animating in or out, and the group whose open state just changed.
#[derive(Default)]
pub struct Motion {
    entering: Vec<(Id, Instant)>,
    /// Private stores number tabs on their own, so a row is told apart by store and by when it left.
    leaving: Vec<(bool, Instant, Tab)>,
    pub(crate) toggled_group: Option<(Id, Instant)>,
    resizing: bool,
    /// The row whose close button shows, since when, and how far it had grown then.
    pub(crate) hovered: Option<(TabKey, Instant, f32)>,
    /// The row the pointer last left, when, and how far its button had grown.
    pub(crate) unhovered: Option<(TabKey, Instant, f32)>,
    /// Where the pointer last moved over the list.
    pub(crate) pointer: Option<Point<Pixels>>,
    /// Where the pointer rested when rows moved under it; the row there already shows its button.
    pub(crate) rested: Option<Point<Pixels>>,
}

impl Motion {
    /// How far `key`'s close button has grown.
    pub(crate) fn grown(&self, key: TabKey) -> f32 {
        match (self.hovered, self.unhovered) {
            (Some((k, at, from)), _) if k == key => from + (1. - from) * progress(at, FADE),
            (_, Some((k, at, reached))) if k == key => reached * (1. - progress(at, FADE)),
            _ => 0.,
        }
    }

    pub(crate) fn leave(&mut self, key: TabKey, now: Instant, reached: f32) {
        self.unhovered = Some((key, now, reached));
        self.hovered = self.hovered.filter(|(k, ..)| *k != key);
    }

    /// Rows moved while the pointer rested: hover events now lag behind what is under it.
    pub(crate) fn forget_pointer(&mut self) {
        if let Some((k, ..)) = self.hovered {
            self.leave(k, Instant::now(), self.grown(k));
        }
        self.rested = self.pointer;
    }

    pub(crate) fn near_rest(&self, at: Point<Pixels>) -> bool {
        self.rested.is_some_and(|p| (p.x - at.x).abs() + (p.y - at.y).abs() < px(TAB_ROW / 2.))
    }

    /// A row laid out under the resting pointer takes the hover no event will report, its button already grown.
    pub(crate) fn settle(&mut self, key: TabKey, row: Bounds<Pixels>, at: Point<Pixels>, now: Instant) -> bool {
        if self.hovered.is_some() || !self.near_rest(at) || !row.contains(&at) {
            return false;
        }
        self.hovered = Some((key, now, 1.));
        self.rested = None;
        true
    }
}

/// Another space's lists beside the current ones during a space switch or a swipe. `offset` is
/// where the current list sits: `base` plus the fingers' travel while dragging, else where its
/// spring to 0 began.
pub struct Slide {
    base: f32,
    offset: f32,
    since: Option<Instant>,
    _expire: Option<Task<()>>,
    ix: Option<usize>,
    tabs: Vec<Tab>,
    folders: Vec<Folder>,
    groups: Vec<Group>,
}

#[derive(Clone)]
struct SidebarResize;

impl Render for SidebarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
pub struct DraggedTab {
    pub key: TabKey,
    pub title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        drag_chip(self.title.clone())
    }
}

#[derive(Clone)]
pub struct DraggedGroup {
    id: Id,
    name: SharedString,
}

impl Render for DraggedGroup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        drag_chip(self.name.clone())
    }
}

#[derive(Clone)]
pub struct DraggedFolder {
    id: Id,
    name: SharedString,
}

impl Render for DraggedFolder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        drag_chip(self.name.clone())
    }
}

fn drag_chip(title: SharedString) -> Div {
    div()
        .px_2p5()
        .py_1()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::surface())
        .border_1()
        .border_color(Theme::surface_edge())
        .text_color(Theme::text())
        .text_size(px(TEXT))
        .max_w(px(220.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .shadow(Elevation::Popover.shadow())
        .child(title)
}

use crate::tabs::display_title as label;

/// The site's own icon once fetched, otherwise its tinted initial.
pub fn site_icon(b: &Browser, url: &str, size: f32) -> AnyElement {
    tab_icon(b, url, size, false)
}

/// A sleeping tab's icon is greyed and faded, so awake tabs stand out at a glance.
fn tab_icon(b: &Browser, url: &str, size: f32, asleep: bool) -> AnyElement {
    let dim = if asleep { 0.4 } else { 1. };
    match b.favicon(url) {
        Some(image) => img(image).w(px(size)).h(px(size)).flex_none().rounded(px(size * 0.22)).grayscale(asleep).opacity(dim).into_any_element(),
        None => favicon(&tab_host(url).unwrap_or_default(), size).opacity(dim).into_any_element(),
    }
}

/// Columns of the tile grid for `count` tiles in a sidebar `sidebar` points wide, and the tiles' height.
fn tile_grid(sidebar: f32, count: usize) -> (u16, f32) {
    let width = sidebar - 2. - 2. * TILE_GAP;
    let columns = browser_core::tabs::tile_row(width, TILE_GAP, count).0;
    (u16::try_from(columns).unwrap_or(u16::MAX), browser_core::tabs::tile_grid(width, TILE_GAP).1)
}

fn space_tint(s: &SpaceRef) -> Option<Rgba> {
    (!s.private).then(|| Theme::profile_color(&s.space.color)).flatten()
}

/// Blends two optional tints; a missing one is the other made transparent, so only the wash fades.
fn mix(from: Option<Rgba>, to: Option<Rgba>, t: f32) -> Option<Rgba> {
    let clear = |c: Rgba| Rgba { a: 0., ..c };
    let (a, b) = (from.or(to.map(clear))?, to.or(from.map(clear))?);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Some(Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a) })
}

fn background(b: &Browser) -> Background {
    let tint = |ix: usize| b.model.spaces.get(ix).and_then(space_tint);
    let (other, shown) = b.slide_other().map_or((None, 0.), |(ix, _, shown)| (tint(ix), shown));
    mix(tint(b.model.space_ix), other, shown).map_or_else(Theme::sidebar, Theme::sidebar_tinted)
}

pub fn group_color(g: &Group) -> Rgba {
    rgb(group_rgb(g))
}

/// A group's colour as `0xRRGGBB`: its pick, or one fixed by its id.
pub fn group_rgb(g: &Group) -> u32 {
    browser_core::tabs::group_color(g, &PROFILE_COLORS)
}

impl Browser {
    /// Called with the previous tab list after a reload; tabs added to the same space slide in.
    pub(crate) fn note_new_tabs(&mut self, old: &[Tab]) {
        self.motion.entering.retain(|(_, at)| at.elapsed() < POP);
        let row = |tabs: &[Tab], id: Id| tabs.iter().position(|t| t.id == id);
        if self.motion.hovered.is_some_and(|(k, ..)| row(old, k.id) != row(&self.model.tabs, k.id)) {
            self.motion.forget_pointer();
        }
        let same_space = old.first().zip(self.model.tabs.first()).is_some_and(|(a, b)| a.space_id == b.space_id);
        if same_space && !self.pref(Pref::ReduceMotion) {
            let known: HashSet<Id> = old.iter().map(|t| t.id).collect();
            let now = Instant::now();
            self.motion.entering.extend(self.model.tabs.iter().filter(|t| !known.contains(&t.id)).map(|t| (t.id, now)));
        }
    }

    /// Slides from the space at `from`, whose lists are given, to the current one, picking up
    /// wherever the old list was, mid-drag or mid-slide.
    pub(crate) fn slide_from(&mut self, from: usize, (tabs, folders, groups): (Vec<Tab>, Vec<Folder>, Vec<Group>)) {
        let dir = if self.model.space_ix > from { 1. } else { -1. };
        let start = self.slide_offset().unwrap_or(0.) + dir * self.sidebar_width;
        self.slide = (from != self.model.space_ix && from < self.model.spaces.len() && !self.pref(Pref::ReduceMotion)).then(|| Slide {
            base: 0.,
            offset: start,
            since: Some(Instant::now()),
            _expire: None,
            ix: Some(from),
            tabs,
            folders,
            groups,
        });
    }

    pub(crate) fn follow_drag(&mut self, dx: f32, cx: &mut Context<Self>) {
        if self.pref(Pref::ReduceMotion) {
            return;
        }
        let width = self.sidebar_width;
        let dragging = self.slide.as_ref().filter(|s| s.since.is_none());
        let base = dragging.map_or_else(|| self.slide_offset().unwrap_or(0.) - dx, |s| s.base);
        let ix = self.neighbour(if base + dx < 0. { Step::Next } else { Step::Previous });
        let offset = follow(base + dx, width, ix.is_some());
        let slide = match self.slide.take() {
            Some(s) if s.since.is_none() && s.ix == ix => s,
            _ => {
                let (tabs, folders, groups) = ix.map(|ix| self.space_lists(ix)).unwrap_or_default();
                Slide { base, offset, since: None, _expire: None, ix, tabs, folders, groups }
            }
        };
        let expire = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAG_IDLE).await;
            this.update(cx, |b, cx| b.cancel_drag(cx)).ok();
        });
        self.slide = Some(Slide { base, offset, _expire: Some(expire), ..slide });
        cx.notify();
    }

    pub(crate) fn cancel_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = self.slide.as_mut().filter(|s| s.since.is_none()) {
            s.since = Some(Instant::now());
            cx.notify();
        }
    }

    fn slide_offset(&self) -> Option<f32> {
        let s = self.slide.as_ref()?;
        match s.since {
            None => Some(s.offset),
            Some(at) => (at.elapsed() < SLIDE).then(|| s.offset * (1. - progress(at, SLIDE))),
        }
    }

    /// The other space's index, its list's offset and the share of the sidebar it covers.
    fn slide_other(&self) -> Option<(usize, f32, f32)> {
        let offset = self.slide_offset()?;
        let ix = self.slide.as_ref()?.ix?;
        let side = if ix > self.model.space_ix { self.sidebar_width } else { -self.sidebar_width };
        Some((ix, offset + side, (offset.abs() / self.sidebar_width).min(1.)))
    }

    /// Drops a finished slide, so the other space's tabs go with it; true while one animates.
    pub(crate) fn settle_slide(&mut self) -> bool {
        let animating = self.slide.as_ref().is_some_and(|s| s.since.is_some());
        if animating && self.slide_offset().is_none() {
            self.slide = None;
        }
        animating && self.slide.is_some()
    }

    fn as_other<R>(&mut self, f: impl FnOnce(&Browser) -> R) -> R {
        self.swap_other();
        let out = f(self);
        self.swap_other();
        out
    }

    fn swap_other(&mut self) {
        let Some(s) = self.slide.as_mut() else { return };
        let Some(ix) = s.ix.as_mut() else { return };
        std::mem::swap(&mut self.model.space_ix, ix);
        std::mem::swap(&mut self.model.tabs, &mut s.tabs);
        std::mem::swap(&mut self.model.folders, &mut s.folders);
        std::mem::swap(&mut self.model.groups, &mut s.groups);
    }

    /// Keeps a closed tab's row on screen while it collapses.
    /// Rows close up at once under a pointer over the list, so the next row's close button is
    /// where the last one was.
    pub(crate) fn tab_leaving(&mut self, tab: &Tab, cx: &mut Context<Self>) {
        if self.pref(Pref::ReduceMotion) || tab.kind != TabKind::Ephemeral || self.motion.pointer.is_some() {
            return;
        }
        let at = Instant::now();
        self.motion.leaving.push((self.model.private(), at, tab.clone()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EXIT).await;
            this.update(cx, |b, cx| {
                b.motion.leaving.retain(|(_, left, _)| *left != at);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub fn render(b: &mut Browser, cx: &mut Context<Browser>) -> AnyElement {
    if !cx.has_active_drag() {
        b.split_drop = None;
    }
    let bg = background(b);
    content(b, NOTCH_WIDTH + 10., cx)
        .relative()
        .w(px(b.sidebar_width))
        .flex_none()
        .m(px(SIDEBAR_MARGIN))
        .mr_0()
        .rounded(px(Theme::radius_tile()))
        .border_1()
        .border_color(Theme::sidebar_edge())
        .bg(bg)
        .shadow(Elevation::Page.shadow())
        .child(resize_handle(cx))
        .into_any_element()
}

fn resize_handle(cx: &mut Context<Browser>) -> impl IntoElement {
    let save = |b: &mut Browser, cx: &mut Context<Browser>| {
        if std::mem::take(&mut b.motion.resizing) {
            b.set_sidebar_width(b.sidebar_width, true, cx);
        }
    };
    div()
        .id("sidebar-resize")
        .absolute()
        .top(px(Theme::radius_tile()))
        .bottom(px(Theme::radius_tile()))
        .right(px(-SIDEBAR_MARGIN))
        .w(px(SIDEBAR_MARGIN + 3.))
        .cursor_col_resize()
        .on_drag(SidebarResize, |d, _, _, cx| cx.new(|_| d.clone()))
        .on_drag_move(cx.listener(|b, e: &DragMoveEvent<SidebarResize>, _, cx| {
            b.motion.resizing = true;
            b.set_sidebar_width(f32::from(e.event.position.x) - SIDEBAR_MARGIN, false, cx);
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(move |b, _, _, cx| save(b, cx)))
        .on_mouse_up_out(MouseButton::Left, cx.listener(move |b, _, _, cx| save(b, cx)))
        .on_click(cx.listener(|b, e: &ClickEvent, _, cx| {
            if e.click_count() == 2 {
                b.set_sidebar_width(SIDEBAR_WIDTH, true, cx);
            }
        }))
}

/// The sidebar as a card over the page below the toolbar, so the window's traffic lights stay
/// visible above it. It slides in from the window's left edge and fully back out, fading as it goes.
pub fn floating(b: &mut Browser, height: Pixels, closing: bool, cx: &mut Context<Browser>) -> AnyElement {
    let width = b.sidebar_width;
    let bg = background(b);
    let card = content(b, TILE_GAP + 4., cx)
        .absolute()
        .top_0()
        .left(px(SIDEBAR_MARGIN))
        .w(px(width))
        .h(height - px(SIDEBAR_MARGIN))
        .rounded(px(Theme::radius_tile()))
        .border_1()
        .border_color(Theme::sidebar_edge())
        .bg(bg)
        .shadow(Theme::elevation(2., GRACE / 3.));
    let away = SIDEBAR_MARGIN + width + GRACE;
    let reduce = b.pref(Pref::ReduceMotion);
    if closing {
        animate(card, "peek-out", EXIT, reduce, move |el, t| el.left(px(SIDEBAR_MARGIN - away * t)).opacity(1. - t))
    } else {
        animate(card, "peek-in", POP, reduce, move |el, t| el.left(px(SIDEBAR_MARGIN - away * (1. - t))).opacity(t))
    }
}

/// `inset` keeps the header clear of the traffic lights when they share its row.
fn content(b: &mut Browser, inset: f32, cx: &mut Context<Browser>) -> Div {
    let offset = b.slide_offset().unwrap_or(0.);
    // The other space is only shown, so a blocker on top keeps clicks and drops from reaching it.
    let blocker = || div().absolute().size_full().block_mouse_except_scroll();
    let other = b.slide_other().map(|(_, left, shown)| {
        let (title, list) = b.as_other(|b| (title(b, cx).into_any_element(), list(b, cx)));
        (
            div().absolute().inset_0().flex().items_center().opacity(shown).child(title).child(blocker()),
            shown,
            [list.left(px(left)).into_any_element(), blocker().left(px(left)).into_any_element()],
        )
    });
    let shown = other.as_ref().map_or(0., |(_, shown, _)| *shown);
    let (other_title, other_list) = other.map(|(title, _, list)| (title, list)).unzip();
    let titles = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .relative()
        .flex()
        .child(div().size_full().flex().items_center().opacity(1. - shown).child(title(b, cx)))
        .children(other_title);
    let header = div()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .pl(px(inset))
        .pr(px(TILE_GAP))
        .flex()
        .items_center()
        .gap_1()
        .child(titles)
        .child(crate::downloads::header_button(b, cx));
    div()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Right, cx.listener(|b, _, w, cx| b.sidebar_menu(w, cx)))
        .on_scroll_wheel(cx.listener(|b, e, w, cx| b.sidebar_swipe(e, w, cx)))
        .text_color(Theme::text())
        .text_size(px(TEXT))
        .child(header)
        .child(div().flex_1().min_h_0().relative().overflow_hidden().child(list(b, cx).left(px(offset))).children(other_list.into_iter().flatten()))
        .child(space_dots(b, cx))
}

/// Keyed by the space, so the two lists of a slide keep apart their scroll and row animations.
fn list(b: &Browser, cx: &mut Context<Browser>) -> Stateful<Div> {
    let mut list = div().id(("space-list", b.model.space_ix));
    // GPUI scrolls a vertical list by a purely horizontal delta, which would scroll it during swipes.
    list.style().restrict_scroll_to_axis = Some(true);
    list.absolute()
        .size_full()
        .overflow_y_scroll()
        .on_scroll_wheel(cx.listener(|b, _, _, _| b.motion.forget_pointer()))
        .on_mouse_move(cx.listener(|b, e: &MouseMoveEvent, _, _| {
            b.motion.pointer = Some(e.position);
            b.motion.rested = b.motion.rested.filter(|_| b.motion.near_rest(e.position));
        }))
        .on_hover(cx.listener(|b, hovered: &bool, _, _| {
            if !hovered {
                b.motion.pointer = None;
                b.motion.rested = None;
            }
        }))
        .px(px(TILE_GAP))
        .pb_2()
        .child(tiles(b, cx))
        .child(pinned(b, cx))
        .child(div().mx_2().my_1p5().h(px(1.)).bg(Theme::hairline()))
        .child(ephemeral(b, cx))
}

fn title(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let (space_id, space_name) = b.model.current_space().map_or((0, String::new()), |s| (s.space.id, s.space.name.clone()));
    let glyph = b.model.current_space().map(|s| match (s.private, space_tint(s)) {
        (true, _) => (if b.model.burner_shown() { Icon::Flame } else { ui::icon::space_icon(&s.space.icon) }, Theme::warning()),
        (false, tint) => (ui::icon::space_icon(&s.space.icon), tint.unwrap_or(Theme::space_label())),
    });
    let menu = move |b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>| {
        if b.rename_input(RenameTarget::Space(space_id)).is_none() {
            b.space_menu(b.model.space_ix, w, cx);
        }
    };
    div()
        .id("space-switcher")
        .min_w_0()
        .h(px(28.))
        .pr_1()
        .flex()
        .items_center()
        .gap_1()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .on_click(cx.listener(move |b, _, w, cx| menu(b, w, cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| menu(b, w, cx)))
        .children(glyph.map(|(glyph, color)| {
            div()
                .size(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .bg(Theme::selected())
                .child(icon(glyph, 16., color))
        }))
        .text_size(px(14.))
        .font_weight(FontWeight::BOLD)
        .child(name(b, RenameTarget::Space(space_id), &space_name, cx))
}

fn tab_drop(
    kind: TabKind,
    folder: Option<Id>,
    group: Option<Id>,
    before: Option<TabKey>,
) -> impl Fn(&mut Browser, &DraggedTab, &mut Window, &mut Context<Browser>) {
    move |b, dragged, _, cx| {
        let at = browser_core::sidebar::DropAt { kind, folder, group, before };
        if let Some(old) = browser_core::sidebar::drop_tab(&mut b.model, &b.core, dragged.key, at) {
            b.space_shown(&old);
            cx.notify();
        }
    }
}

/// The folder or group name, or its inline name field while it is being renamed; a click
/// anywhere else in the window saves the name.
fn name(b: &Browser, target: RenameTarget, name: &str, cx: &mut Context<Browser>) -> AnyElement {
    match b.rename_input(target) {
        Some(input) => div()
            .flex_1()
            .min_w_0()
            .px_1()
            .rounded(px(Theme::radius_row()))
            .bg(Theme::raised())
            .on_mouse_down_out(cx.listener(|b, _, window, cx| b.end_rename(true, window, cx)))
            .child(input)
            .into_any_element(),
        None => div().min_w_0().line_clamp(1).text_ellipsis().child(SharedString::from(bounded_title(name))).into_any_element(),
    }
}

/// A chevron that turns between closed and open each time `open` changes.
fn chevron(b: &Browser, id: Id, open: bool) -> AnyElement {
    let turn = move |t: f32| percentage(if open { 0.25 * t } else { 0.25 * (1. - t) });
    let glyph = icon(Icon::ChevronRight, ICON_SMALL, Theme::muted()).with_transformation(Transformation::rotate(turn(1.)));
    animate(glyph, (if open { "open" } else { "shut" }, id as u64), FADE, b.pref(Pref::ReduceMotion), move |g, t| {
        g.with_transformation(Transformation::rotate(turn(t)))
    })
}

fn tiles(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let tiles: Vec<&Tab> = b.model.tabs.iter().filter(|t| t.kind == TabKind::Tile).collect();
    let reduce = b.pref(Pref::ReduceMotion);
    let (columns, side) = tile_grid(b.sidebar_width, tiles.len());
    div()
        .id("tiles")
        .grid()
        .grid_cols(columns)
        .gap(px(TILE_GAP))
        .when(!tiles.is_empty(), |d| d.pb_1())
        .min_h(px(if tiles.is_empty() && cx.has_active_drag() { 46. } else { 6. }))
        .rounded(px(Theme::radius_tile()))
        .drag_over::<DraggedTab>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(tab_drop(TabKind::Tile, None, None, None)))
        .children(tiles.into_iter().map(|t| {
            let key = b.model.key(t);
            let active = b.model.active == Some(key);
            let title: SharedString = label(t).into();
            let tile = div()
                .id(("tile", t.id as u64))
                .w_full()
                .h(px(side))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_tile()))
                .bg(if active { Theme::selected() } else { Theme::raised() })
                .border_1()
                .border_color(match (active, b.in_split(key), b.splits.contains(key)) {
                    (true, _, _) => Theme::sidebar_edge(),
                    (false, true, _) => Theme::accent(),
                    (false, false, true) => Theme::faint(),
                    (false, false, false) => Theme::hairline(),
                })
                .when(!active, |d| d.hover(|s| s.bg(Theme::hover())))
                .cursor_pointer()
                .child(match (b.loading(key), b.still()) {
                    (true, false) => spinner(18., Theme::space_label()).into_any_element(),
                    _ => tab_icon(b, &t.url, 22., b.sleeping(key)),
                })
                .on_click(cx.listener(move |b, _, window, cx| b.switch_to(key, window, cx)))
                .on_hover(cx.listener(move |b, hovered: &bool, window, cx| b.hover_tab(key, *hovered, window, cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|b, _, _, cx| b.hide_hover_card(cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| b.tab_menu(key, w, cx)))
                .on_drag(DraggedTab { key, title }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<DraggedTab>(|s, _, _, _| s.border_color(Theme::accent()))
                .on_drop(cx.listener(tab_drop(TabKind::Tile, None, None, Some(key))));
            animate(tile, ("tile-in", t.id as u64), FADE, reduce, |el, p| el.opacity(p))
        }))
}

fn tab_row(b: &Browser, t: &Tab, indent: f32, cx: &mut Context<Browser>) -> AnyElement {
    let key = b.model.key(t);
    let active = b.model.active == Some(key);
    let title: SharedString = label(t).into();
    let closable = t.kind == TabKind::Ephemeral;
    let loading = b.loading(key);
    let asleep = b.sleeping(key);
    let drift = b.pinned_drift(key).and_then(|u| u.host_str().map(str::to_owned));
    let drifted = drift.is_some();
    let renaming = b.rename_input(RenameTarget::Tab(t.id)).is_some();
    let entering = b.motion.entering.iter().any(|(id, _)| *id == t.id);
    let engine = b.tab_web_engine(key);
    let burner = b.model.burner_shown();
    let edge = |color| div().absolute().left_0().top(px(9.)).bottom(px(9.)).w(px(3.)).rounded_full().bg(color);
    let row = div()
        .id(("tab", t.id as u64))
        .group("tab")
        .relative()
        .h(px(TAB_ROW))
        .flex_none()
        .overflow_hidden()
        .pl(px(10. + indent))
        .pr_0p5()
        .flex()
        .items_center()
        .gap_2p5()
        .rounded(px(Theme::radius_row()))
        .when(active, |d| {
            let highlight = div().absolute().inset_0().rounded(px(Theme::radius_row())).bg(Theme::selected()).border_1().border_color(Theme::surface_edge());
            d.child(fade_in(highlight, ("tab-active", t.id as u64), b.pref(Pref::ReduceMotion)))
        })
        // The other tabs of a split carry the edge too: in the accent for the split on screen.
        .when(!active && b.splits.contains(key), |d| d.child(edge(if b.in_split(key) { Theme::accent() } else { Theme::faint() })))
        .when(!active, |d| d.hover(|s| s.bg(Theme::hover())))
        .when(closable && b.motion.rested.is_some() && b.motion.hovered.is_none(), |d| {
            let this = cx.entity().downgrade();
            d.child(
                canvas(
                    |_, _, _| {},
                    move |row, _, window, cx| {
                        let at = window.mouse_position();
                        window.defer(cx, move |_, cx| {
                            this.update(cx, |b, cx| {
                                if b.motion.settle(key, row, at, Instant::now()) {
                                    cx.notify();
                                }
                            })
                            .ok();
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .cursor_pointer()
        .child(div().flex_none().size(px(16.)).flex().items_center().justify_center().child(match (loading, b.still()) {
            (true, false) => spinner(14., Theme::space_label()).into_any_element(),
            (true, true) => div().opacity(0.5).child(tab_icon(b, &t.url, 16., asleep)).into_any_element(),
            _ => tab_icon(b, &t.url, 16., asleep),
        }))
        .when(burner, |d| {
            d.child(
                div()
                    .id(("burner", t.id as u64))
                    .flex_none()
                    .child(icon(Icon::Flame, 14., Theme::warning()))
                    .tooltip(tooltip("Burner tab: nothing is kept after it closes", None)),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(if asleep && !active { Theme::asleep() } else { Theme::text() })
                .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                .child(name(b, RenameTarget::Tab(t.id), &title, cx)),
        )
        .when(engine != b.core.default_web_engine.get() && !renaming, |d| {
            d.child(div().flex_none().text_size(px(TEXT_SMALL)).text_color(Theme::faint()).child(crate::tabs::web_engine_name(engine)))
        })
        .when(b.heavy(key) && !renaming, |d| {
            d.child(
                div()
                    .id(("heavy", t.id as u64))
                    .flex_none()
                    .size(px(6.))
                    .rounded_full()
                    .bg(Theme::warning())
                    .tooltip(tooltip("Awake in the background and using memory. Sleep Tab frees it.", None)),
            )
        })
        .when_some(drift.filter(|_| !renaming), |d, host| {
            d.child(icon_button(("return", t.id as u64), Icon::Undo, true).tooltip(tooltip(format!("Return to {}", bounded_title(&host)), None)).on_click(
                cx.listener(move |b, _, window, cx| {
                    cx.stop_propagation();
                    b.return_to_pinned(key, window, cx);
                }),
            ))
        })
        .children(state_glyph(t, asleep, active && closable || drifted || renaming || closable && b.motion.grown(key) > 0.).map(|glyph| {
            let shown = div().flex_none().w(px(CLOSE_WIDTH)).flex().justify_center().child(glyph);
            let shown = if closable { shown.group_hover("tab", |s| s.opacity(0.)) } else { shown };
            animate(shown, ("state", t.id as u64), SLIDE, b.pref(Pref::ReduceMotion), |el, p| el.opacity(p))
        }))
        .when(closable, |d| {
            let hovered = b.motion.hovered.filter(|(k, ..)| *k == key).map(|(_, at, from)| (at, from));
            let leaving = b.motion.unhovered.filter(|(k, at, _)| *k == key && at.elapsed() < FADE).map(|(_, _, reached)| reached);
            // The row's gap goes with the button, so a hidden button leaves the title the whole row.
            let shown = |el: Stateful<Div>, p: f32| el.w(px(CLOSE_WIDTH * p)).ml(px(-CLOSE_GAP * (1. - p))).opacity(p);
            let button = shown(icon_button(("close", t.id as u64), Icon::Close, true).overflow_hidden(), if hovered.is_some() || active { 1. } else { 0. })
                .tooltip(crate::shortcuts::tip("Close Tab", &crate::CloseTab))
                .on_click(cx.listener(move |b, _, window, cx| {
                    cx.stop_propagation();
                    b.close_tab(key, window, cx);
                }));
            let reduce = b.pref(Pref::ReduceMotion);
            d.child(match (hovered, leaving) {
                _ if active => button.into_any_element(),
                (Some((at, from)), _) if at.elapsed() < FADE => {
                    animate(button, ("close-in", t.id as u64), FADE, reduce, move |el, p| shown(el, from + (1. - from) * p))
                }
                // A row under the pointer shows its button even when no hover event has said so yet.
                (None, Some(reached)) => animate(
                    button.group_hover("tab", |s| s.w(px(CLOSE_WIDTH)).ml(px(0.)).opacity(1.)),
                    ("close-out", t.id as u64),
                    FADE,
                    reduce,
                    move |el, p| shown(el, reached * (1. - p)),
                ),
                _ => button.group_hover("tab", |s| s.w(px(CLOSE_WIDTH)).ml(px(0.)).opacity(1.)).into_any_element(),
            })
        })
        .on_click(cx.listener(move |b, event: &ClickEvent, window, cx| {
            b.switch_to(key, window, cx);
            if event.click_count() == 2 {
                b.begin_rename(RenameTarget::Tab(key.id), window, cx);
            }
        }))
        .on_hover(cx.listener(move |b, hovered: &bool, window, cx| b.hover_tab(key, *hovered, window, cx)))
        .on_mouse_down(MouseButton::Left, cx.listener(|b, _, _, cx| b.hide_hover_card(cx)))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| b.tab_menu(key, w, cx)))
        .on_drag(DraggedTab { key, title }, |d, _, _, cx| cx.new(|_| d.clone()))
        .map(|d| match b.split_drop.filter(|(k, _)| *k == key) {
            None => d,
            // A line where the tab will land, before this row.
            Some((_, None)) => d.child(div().absolute().top_0().left(px(6.)).right(px(6.)).h(px(2.)).rounded_full().bg(Theme::accent())),
            // Outlined and named, so a drop that splits reads apart from one that moves the tab.
            Some((_, Some(does))) => d
                .bg(Theme::accent_soft())
                .child(div().absolute().inset_0().rounded(px(Theme::radius_row())).border_1().border_color(Theme::accent()))
                .child(div().flex_none().pr_1p5().text_size(px(TEXT_SMALL)).text_color(Theme::accent()).child(does)),
        })
        .on_drag_move(cx.listener(move |b, e: &DragMoveEvent<DraggedTab>, _, cx| {
            let inside = e.bounds.contains(&e.event.position);
            let share = (e.event.position.y - e.bounds.origin.y) / e.bounds.size.height;
            let dragged = e.drag(cx).key;
            let does = b.splits.drop_label(key, dragged).filter(|_| browser_core::split::drops_into(share));
            let target = if inside && dragged != key { Some((key, does)) } else { b.split_drop.filter(|(k, _)| *k != key) };
            if b.split_drop != target {
                b.split_drop = target;
                cx.notify();
            }
        }))
        .on_drop(cx.listener({
            let reorder = tab_drop(t.kind, t.folder_id, t.group_id, Some(key));
            move |b, dragged: &DraggedTab, window, cx| {
                if b.split_drop.take().is_some_and(|(k, does)| k == key && does.is_some()) {
                    b.drop_on_pane(key, dragged.key, window, cx);
                } else {
                    reorder(b, dragged, window, cx);
                }
            }
        }));
    animate(row, ("tab-in", t.id as u64), POP, b.pref(Pref::ReduceMotion), move |el, p| {
        let el = el.opacity(p);
        if entering { el.h(px(TAB_ROW * p)) } else { el }
    })
}

fn state_glyph(t: &Tab, asleep: bool, busy: bool) -> Option<impl IntoElement> {
    let glyph = match (asleep, t.kind) {
        _ if busy => return None,
        (true, _) => Icon::Moon,
        (false, TabKind::Pinned) => Icon::Pin,
        _ => return None,
    };
    Some(icon(glyph, 13., Theme::faint()))
}

/// A closed tab's row collapsing out of the list.
fn leaving_row(b: &Browser, t: &Tab) -> AnyElement {
    let row = div()
        .h(px(TAB_ROW))
        .flex_none()
        .overflow_hidden()
        .pl(px(10.))
        .flex()
        .items_center()
        .gap_2p5()
        .text_color(Theme::muted())
        .child(site_icon(b, &t.url, 16.))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(label(t))));
    animate(row, ("tab-out", t.id as u64), EXIT, false, |el, p| el.h(px(TAB_ROW * (1. - p))).opacity(1. - p))
}

/// The unpinned tabs of a group (or of none), with closing tabs kept in their old place.
fn rows(b: &Browser, group: Option<Id>, show: impl Fn(&Tab) -> bool, cx: &mut Context<Browser>) -> Vec<AnyElement> {
    let space = b.model.current_space().map(|s| s.space.id);
    let mut tabs: Vec<(&Tab, bool)> = b
        .model
        .tabs
        .iter()
        .filter(|t| t.kind == TabKind::Ephemeral && t.group_id == group && show(t))
        .map(|t| (t, false))
        .chain(
            b.motion
                .leaving
                .iter()
                .filter(|(private, _, t)| *private == b.model.private() && Some(t.space_id) == space && t.group_id == group)
                .map(|(.., t)| (t, true)),
        )
        .collect();
    tabs.sort_by_key(|(t, _)| t.position);
    tab_rows(b, &tabs, 0., cx)
}

/// One row per tab, a split's panes side by side in one row at the place of the first of them.
fn tab_rows(b: &Browser, tabs: &[(&Tab, bool)], indent: f32, cx: &mut Context<Browser>) -> Vec<AnyElement> {
    let keys: Vec<TabKey> = tabs.iter().filter(|(_, leaving)| !leaving).map(|(t, _)| b.model.key(t)).collect();
    tabs.iter()
        .filter_map(|&(t, leaving)| {
            if leaving {
                return Some(leaving_row(b, t));
            }
            Some(match b.splits.entry(b.model.key(t), &keys)?.as_slice() {
                [_] => tab_row(b, t, indent, cx),
                panes => div()
                    .flex()
                    .gap(px(ROW_GAP))
                    .pl(px(indent))
                    .children(panes.iter().filter_map(|k| b.model.tab(*k)).map(|t| div().flex_1().min_w_0().flex().flex_col().child(tab_row(b, t, 0., cx))))
                    .into_any_element(),
            })
        })
        .collect()
}

fn pinned(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let mut col = div()
        .id("pinned")
        .flex()
        .flex_col()
        .gap(px(ROW_GAP))
        .mt_1()
        .min_h(px(12.))
        .rounded(px(Theme::radius_row()))
        .drag_over::<DraggedTab>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(tab_drop(TabKind::Pinned, None, None, None)))
        .when(!b.model.tabs.iter().any(|t| t.kind == TabKind::Pinned) && b.model.folders.is_empty(), |d| {
            d.child(
                div()
                    .h(px(TAB_ROW))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .rounded(px(Theme::radius_row()))
                    .border_1()
                    .border_dashed()
                    .border_color(Theme::hairline())
                    .text_size(px(TEXT_SMALL))
                    .text_color(Theme::faint())
                    .child("Drag tabs here to pin them"),
            )
        });
    let loose: Vec<(&Tab, bool)> = b.model.tabs.iter().filter(|t| t.kind == TabKind::Pinned && t.folder_id.is_none()).map(|t| (t, false)).collect();
    col = col.children(tab_rows(b, &loose, 0., cx));
    let folders: Vec<AnyElement> = b.model.folders.iter().filter(|f| f.parent_id.is_none()).map(|f| folder(b, f, 0, cx)).collect();
    col.drag_over::<DraggedFolder>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(|b, dragged: &DraggedFolder, _, cx| b.move_folder(dragged.id, None, cx)))
        .children(folders)
}

/// A folder row with its tabs and subfolders, each level indented one step further.
fn folder(b: &Browser, f: &Folder, depth: usize, cx: &mut Context<Browser>) -> AnyElement {
    let id = f.id;
    let indent = 16. * depth as f32;
    let open = b.model.expanded.contains(&id);
    let glyph = chosen_icon(&f.icon, ICON, Theme::space_label()).unwrap_or_else(|| icon(Icon::Folder, ICON, Theme::space_label()).into_any_element());
    let header = div()
        .id(("folder", id as u64))
        .h(px(TAB_ROW))
        .pl(px(10. + indent))
        .pr_2p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_2p5()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(Theme::text())
        .hover(|s| s.bg(Theme::hover()))
        .drag_over::<DraggedTab>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(tab_drop(TabKind::Pinned, Some(id), None, None)))
        .drag_over::<DraggedFolder>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(move |b, dragged: &DraggedFolder, _, cx| b.move_folder(dragged.id, Some(id), cx)))
        .on_drag(DraggedFolder { id, name: bounded_title(&f.name).into() }, |d, _, _, cx| cx.new(|_| d.clone()))
        .on_click(cx.listener(move |b, event: &ClickEvent, window, cx| {
            b.toggle_folder(id, cx);
            if event.click_count() == 2 {
                b.begin_rename(RenameTarget::Folder(id), window, cx);
            }
        }))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| b.folder_menu(id, w, cx)))
        .child(div().flex_none().size(px(16.)).flex().items_center().justify_center().child(glyph))
        .child(name(b, RenameTarget::Folder(id), &f.name, cx))
        .child(chevron(b, id, open));
    let col = div().flex().flex_col().gap(px(ROW_GAP)).child(header).children(icon_picker(b, RenameTarget::Folder(id), &f.icon, cx));
    if !open {
        // A closed folder still shows its active tab, so the selection never disappears.
        let tree = folder_tree(&b.model.folders, id);
        let active = b.model.tabs.iter().filter(|t| t.folder_id.is_some_and(|f| tree.contains(&f)) && b.model.active == Some(b.model.key(t)));
        return col.children(active.map(|t| tab_row(b, t, indent + 16., cx)).collect::<Vec<_>>()).into_any_element();
    }
    let tabs: Vec<(&Tab, bool)> = b.model.tabs.iter().filter(|t| t.kind == TabKind::Pinned && t.folder_id == Some(id)).map(|t| (t, false)).collect();
    let tabs = tab_rows(b, &tabs, indent + 16., cx);
    let subfolders: Vec<AnyElement> = b.model.folders.iter().filter(|c| c.parent_id == Some(id)).map(|c| folder(b, c, depth + 1, cx)).collect();
    let col = col.children(tabs).children(subfolders);
    if depth == 0 { col.my_1().p(px(INNER_PAD)).rounded(px(Theme::radius_tile())).bg(Theme::group()) } else { col }.into_any_element()
}

fn ephemeral(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let cleanable = browser_core::cleanup::cleanable(&b.model.tabs) && !b.model.burner_shown();
    div()
        .id("ephemeral")
        .flex()
        .flex_col()
        .gap(px(ROW_GAP))
        .min_h(px(120.))
        .pb(px(TAB_ROW * 2.))
        .rounded(px(Theme::radius_row()))
        .drag_over::<DraggedTab>(|s, _, _, _| s.bg(Theme::accent_soft()))
        .on_drop(cx.listener(tab_drop(TabKind::Ephemeral, None, None, None)))
        .child(
            div()
                .id("new-tab")
                .h(px(TAB_ROW))
                .pl(px(10.))
                .pr_0p5()
                .flex()
                .flex_none()
                .items_center()
                .gap_2p5()
                .rounded(px(Theme::radius_row()))
                .text_color(Theme::muted())
                .cursor_pointer()
                .hover(|s| s.bg(Theme::hover()).text_color(Theme::text()))
                .active(|s| s.bg(Theme::pressed()))
                .child(
                    div().size(px(16.)).flex().flex_none().items_center().justify_center().rounded(px(Theme::radius_small())).bg(Theme::accent()).child(icon(
                        Icon::Plus,
                        11.,
                        Theme::on_fill(Theme::accent()),
                    )),
                )
                .child(div().flex_1().child("New Tab"))
                .child(icon_button("new-group", Icon::Group, true).tooltip(crate::shortcuts::tip("New Tab Group", &crate::NewGroup)).on_click(cx.listener(
                    |b, _, w, cx| {
                        cx.stop_propagation();
                        b.new_group(b.model.active, w, cx);
                    },
                )))
                .child(icon_button("new-folder", Icon::FolderPlus, true).tooltip(tooltip("New Folder", None)).on_click(cx.listener(|b, _, w, cx| {
                    cx.stop_propagation();
                    b.new_folder(None, w, cx);
                })))
                .when(cleanable, |d| {
                    d.child(icon_button("clean-up", Icon::BroomSparkles, true).tooltip(tooltip(browser_core::cleanup::DESCRIPTION, None)).on_click(
                        cx.listener(|b, _, _, cx| {
                            cx.stop_propagation();
                            b.clean_up_tabs(cx);
                        }),
                    ))
                })
                .on_click(cx.listener(|b, _, window, cx| b.new_tab_page(window, cx))),
        )
        .children(b.model.groups.iter().enumerate().map(|(ix, g)| group_card(b, ix, g, cx)).collect::<Vec<_>>())
        .children(rows(b, None, |_| true, cx))
}

/// A group's or folder's chosen icon, drawn `size` and `color` when it is a symbol; `None` for none.
fn chosen_icon(stored: &str, size: f32, color: Rgba) -> Option<AnyElement> {
    Some(match browser_core::icon::parse(stored)? {
        browser_core::icon::Icon::Symbol(name) => icon(Icon::named(name)?, size, color).into_any_element(),
        browser_core::icon::Icon::Emoji(emoji) => {
            div().text_size(px(size)).line_height(px(size + 2.)).child(SharedString::from(emoji.to_owned())).into_any_element()
        }
    })
}

/// The icon picker under a group's or folder's row while "Change Icon…" is open.
fn icon_picker(b: &Browser, target: RenameTarget, current: &str, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let input = b.icon_input(target)?;
    let focus = gpui::Focusable::focus_handle(input.read(cx), cx);
    let symbols = browser_core::icon::SYMBOLS.iter().filter_map(|name| {
        let button =
            icon_button(*name, Icon::named(name)?, true).when(*name == current, |d| d.bg(Theme::selected())).on_click(cx.listener(move |b, _, window, cx| {
                b.set_icon(target, name, cx);
                b.end_rename(false, window, cx);
            }));
        Some(button)
    });
    let row = div()
        .flex()
        .items_center()
        .gap_1()
        .child(field(true).flex_1().min_w_0().child(input))
        .when(cfg!(target_os = "macos"), |d| {
            d.child(icon_button("emoji-palette", Icon::Keyboard, true).tooltip(tooltip("Emoji & Symbols", None)).on_click(move |_, window, _| {
                window.focus(&focus);
                window.show_character_palette();
            }))
        })
        .when(!current.is_empty(), |d| {
            d.child(button("remove-icon", "Remove", false).on_click(cx.listener(move |b, _, window, cx| {
                b.set_icon(target, "", cx);
                b.end_rename(false, window, cx);
            })))
        });
    let picker = div()
        .flex()
        .flex_col()
        .gap_1()
        .p_1()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .on_mouse_down_out(cx.listener(|b, _, window, cx| b.end_rename(true, window, cx)))
        .child(div().flex().flex_wrap().children(symbols))
        .child(row);
    Some(picker.into_any_element())
}

fn avatar(color: Rgba, size: f32, mark: impl IntoElement) -> impl IntoElement {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color)
        .text_size(px(if size > 20. { TEXT_SMALL } else { TEXT_TINY }))
        .font_weight(FontWeight::BOLD)
        .text_color(Theme::on_fill(color))
        .child(mark)
}

fn group_avatar(g: &Group, size: f32) -> impl IntoElement {
    let color = group_color(g);
    let initial = || SharedString::from(g.name.trim().chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default()).into_any_element();
    avatar(color, size, chosen_icon(&g.icon, ICON_SMALL, Theme::on_fill(color)).unwrap_or_else(initial))
}

/// Height of a group's inner card holding `rows` tabs, border included.
fn inner_height(rows: usize) -> f32 {
    if rows == 0 { 0. } else { rows as f32 * (TAB_ROW + ROW_GAP) - ROW_GAP + 2. * INNER_PAD + 2. }
}

fn group_card(b: &Browser, ix: usize, g: &Group, cx: &mut Context<Browser>) -> AnyElement {
    let id = g.id;
    let name_text: SharedString = bounded_title(&g.name).into();
    let header = div()
        .id(("group", id as u64))
        .h(px(TAB_ROW - 2.))
        .pl_1p5()
        .pr_0p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .font_weight(FontWeight::MEDIUM)
        .hover(|s| s.bg(Theme::hover()))
        .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| b.group_menu(id, w, cx)))
        .on_click(cx.listener(move |b, event: &ClickEvent, window, cx| {
            b.toggle_group(id, cx);
            if event.click_count() == 2 {
                b.begin_rename(RenameTarget::Group(id), window, cx);
            }
        }))
        .on_drag(DraggedGroup { id, name: name_text }, |d, _, _, cx| cx.new(|_| d.clone()))
        .child(group_avatar(g, 18.))
        .child(name(b, RenameTarget::Group(id), &g.name, cx))
        .child(chevron(b, id, !g.collapsed))
        .child(div().flex_1())
        .child(icon_button(("close-group", id as u64), Icon::Close, true).tooltip(tooltip("Close Group and Its Tabs", None)).on_click(cx.listener(
            move |b, _, window, cx| {
                cx.stop_propagation();
                b.close_group(id, window, cx);
            },
        )));
    let toggling = b.motion.toggled_group.is_some_and(|(gid, at)| gid == id && at.elapsed() < FADE);
    let is_active = |t: &Tab| b.model.active == Some(b.model.key(t));
    let keys: Vec<TabKey> = b.model.tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.group_id == Some(id)).map(|t| b.model.key(t)).collect();
    // A split shows as one row.
    let all = keys.iter().filter(|k| b.splits.entry(**k, &keys).is_some()).count();
    let active = b.model.tabs.iter().filter(|t| t.group_id == Some(id) && is_active(t)).count();
    let tabs = rows(b, Some(id), |t| !g.collapsed || toggling || is_active(t), cx);
    let (from, to) = if g.collapsed { (inner_height(all), inner_height(active)) } else { (inner_height(active), inner_height(all)) };
    let inner = (!tabs.is_empty()).then(|| {
        let card = div()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .p(px(INNER_PAD))
            .overflow_hidden()
            .rounded(px(Theme::radius_row() + INNER_PAD))
            .bg(Theme::raised())
            .border_1()
            .border_color(Theme::hairline())
            .children(tabs);
        let key = (if g.collapsed { "group-shut" } else { "group-open" }, id as u64);
        animate(card, key, FADE, b.pref(Pref::ReduceMotion), move |el, t| if toggling { el.h(px(from + (to - from) * t)) } else { el })
    });
    div()
        .id(("group-card", id as u64))
        .flex()
        .flex_col()
        .gap(px(ROW_GAP))
        .my_0p5()
        .p(px(INNER_PAD))
        .rounded(px(Theme::radius_tile()))
        .bg(Theme::group())
        .border_1()
        .border_color(Theme::hairline())
        .drag_over::<DraggedTab>(|s, _, _, _| s.border_color(Theme::accent()))
        .on_drop(cx.listener(tab_drop(TabKind::Ephemeral, None, Some(id), None)))
        .drag_over::<DraggedGroup>(|s, _, _, _| s.border_color(Theme::accent()))
        .on_drop(cx.listener(move |b, dragged: &DraggedGroup, _, cx| b.move_group(dragged.id, ix, cx)))
        .child(header)
        .children(icon_picker(b, RenameTarget::Group(id), &g.icon, cx))
        .children(inner)
        .into_any_element()
}

/// Library on the left and New Space and Settings on the right get the same width, so the space icons sit in the bar's centre.
const BAR_SIDE: f32 = 58.;

fn space_dots(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let dots = b.model.spaces.iter().enumerate().map(|(ix, s)| {
        let current = ix == b.model.space_ix;
        let color = match (current, s.private, space_tint(s)) {
            (true, true, _) => Theme::warning(),
            (true, false, tint) => tint.unwrap_or(Theme::text()),
            (false, _, Some(tint)) => Rgba { a: 0.45, ..tint },
            (false, _, None) => Theme::faint(),
        };
        let name = bounded_title(&s.space.name);
        div()
            .id(("space", ix))
            .w(px(28.))
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::radius_row()))
            .cursor_pointer()
            .when(current, |d| d.bg(Theme::hover()))
            .hover(|st| st.bg(Theme::hover()))
            .child(icon(if s.private && Some(s.space.id) == b.model.burner { Icon::Flame } else { ui::icon::space_icon(&s.space.icon) }, 16., color))
            .map(|d| match crate::menubar::SPACES.get(ix) {
                Some(action) => d.tooltip(crate::shortcuts::tip(name, *action)),
                None => d.tooltip(tooltip(name, None)),
            })
            .on_click(cx.listener(move |b, _, window, cx| b.switch_space(ix, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |b, _, w, cx| b.space_menu(ix, w, cx)))
    });
    // Centred while they fit; past that the row scrolls instead of squeezing the dots.
    let fits = b.model.spaces.len() as f32 * 30. + 2. * (BAR_SIDE + 2.) + 16. <= b.sidebar_width;
    div()
        .h(px(36.))
        .flex_none()
        .px_2()
        .flex()
        .items_center()
        .gap_0p5()
        .child(
            div().w(px(BAR_SIDE)).flex_none().flex().child(
                icon_button("library", Icon::Library, true)
                    .tooltip(crate::shortcuts::tip("Library", &crate::ToggleLibrary))
                    .when(b.library.is_some(), |d| d.bg(Theme::selected()))
                    .on_click(cx.listener(|b, _, window, cx| b.toggle_library(window, cx))),
            ),
        )
        .child(div().id("space-dots").flex_1().min_w_0().flex().when(fits, |d| d.justify_center()).gap_0p5().overflow_x_scroll().children(dots))
        .child(
            div()
                .w(px(BAR_SIDE))
                .flex_none()
                .flex()
                .justify_end()
                .gap_0p5()
                .child(
                    icon_button("new-space", Icon::Plus, true)
                        .tooltip(crate::shortcuts::tip("New Space", &crate::NewSpace))
                        .on_click(cx.listener(|b, _, window, cx| b.new_space(window, cx))),
                )
                .child(
                    icon_button("settings", Icon::Gear, true)
                        .tooltip(crate::shortcuts::tip("Settings", &crate::Settings))
                        .on_click(cx.listener(|b, _, window, cx| b.open_settings(window, cx))),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::{Motion, SIDEBAR_WIDTH, TAB_ROW, inner_height, mix, tile_grid};
    use crate::browser::TabKey;
    use gpui::{Bounds, Rgba, point, px, size};
    use std::time::{Duration, Instant};

    #[test]
    fn rows_moved_under_a_resting_pointer_show_their_button_there_only() {
        let (a, b) = (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 });
        let long_ago = Instant::now() - Duration::from_secs(1);
        let mut motion = Motion { hovered: Some((a, long_ago, 0.)), pointer: Some(point(px(50.), px(100.))), ..Default::default() };
        assert_eq!(motion.grown(a), 1.);
        motion.forget_pointer();
        assert!(motion.hovered.is_none());
        assert_eq!(motion.unhovered.map(|(k, _, reached)| (k, reached)), Some((a, 1.)));
        assert_eq!(motion.grown(b), 0.);
        assert!(motion.near_rest(point(px(52.), px(101.))));
        assert!(!motion.near_rest(point(px(50.), px(100. + TAB_ROW))));

        let row = |top: f32| Bounds::new(point(px(0.), px(top)), size(px(240.), px(TAB_ROW)));
        let now = Instant::now();
        assert!(!motion.settle(a, row(100. + TAB_ROW), point(px(52.), px(101.)), now), "a row away from the pointer stays plain");
        assert!(motion.settle(b, row(90.), point(px(52.), px(101.)), now), "the row that slid under the pointer is hovered");
        assert_eq!(motion.grown(b), 1., "its button shows at once");
        assert!(!motion.settle(a, row(90.), point(px(52.), px(101.)), now), "only one row takes the hover");
    }

    #[test]
    fn a_missing_tint_fades_the_wash_not_the_hue() {
        let red = Rgba { r: 1., g: 0., b: 0., a: 1. };
        let blue = Rgba { r: 0., g: 0., b: 1., a: 1. };
        assert_eq!(mix(None, None, 0.5), None);
        assert_eq!(mix(Some(red), None, 0.25), Some(Rgba { a: 0.75, ..red }));
        assert_eq!(mix(None, Some(red), 0.25), Some(Rgba { a: 0.25, ..red }));
        assert_eq!(mix(Some(red), Some(blue), 0.5), Some(Rgba { r: 0.5, g: 0., b: 0.5, a: 1. }));
    }

    #[test]
    fn tiles_are_four_squares_a_row_at_the_default_width() {
        assert_eq!(tile_grid(SIDEBAR_WIDTH, 9), (4, 53.));
        assert_eq!(tile_grid(420., 9).0, 7, "a wider sidebar fits more in a row");
        assert_eq!(tile_grid(SIDEBAR_WIDTH, 2), (2, 53.), "two tiles share the row and keep their height");
    }

    #[test]
    fn group_inner_card_fits_its_rows() {
        assert_eq!(inner_height(0), 0.);
        assert_eq!(inner_height(1), TAB_ROW + 8.);
        assert_eq!(inner_height(3), 3. * TAB_ROW + 2. + 8.);
    }
}
