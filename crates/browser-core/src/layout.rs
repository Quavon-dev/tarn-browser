use store::Store;

pub const SETTING: &str = "layout.tablet";
pub const SIDEBAR_SETTING: &str = "sidebar_open";
pub const TABLET_SIDE: f32 = 600.;
pub const MAC_WIDTH: f32 = 700.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Touch,
    Mac,
}

impl Layout {
    pub const ALL: [Layout; 2] = [Layout::Touch, Layout::Mac];

    pub fn key(self) -> &'static str {
        match self {
            Layout::Touch => "touch",
            Layout::Mac => "mac",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Touch => "Touch",
            Layout::Mac => "Mac",
        }
    }

    pub fn parse(value: &str) -> Option<Layout> {
        Layout::ALL.into_iter().find(|l| l.key() == value)
    }

    pub fn load(store: &Store) -> Layout {
        store.setting(SETTING).ok().flatten().and_then(|v| Layout::parse(&v)).unwrap_or_default()
    }

    pub fn save(self, store: &Store) -> Result<(), store::Error> {
        store.set_setting(SETTING, self.key())
    }
}

pub const TABS_SETTING: &str = "layout.tabs";

/// Where a window lists its tabs: in the sidebar, or in a strip across the top with the sidebar
/// hidden. Each window keeps its own; the last one picked is where new windows start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TabPlacement {
    #[default]
    Side,
    Top,
}

impl TabPlacement {
    pub fn key(self) -> &'static str {
        match self {
            TabPlacement::Side => "side",
            TabPlacement::Top => "top",
        }
    }

    pub fn load(store: &Store) -> TabPlacement {
        match store.setting(TABS_SETTING).ok().flatten().as_deref() {
            Some("top") => TabPlacement::Top,
            _ => TabPlacement::Side,
        }
    }

    /// The other placement, kept for the windows opened from now on.
    pub fn toggled(self, store: &Store) -> TabPlacement {
        let next = if self == TabPlacement::Top { TabPlacement::Side } else { TabPlacement::Top };
        store.set_setting(TABS_SETTING, next.key()).ok();
        next
    }

    /// What a window shows: phones and the touch layout keep their bottom bar, so only a layout
    /// with a sidebar (desktop, a tablet's Mac layout) puts tabs on top.
    pub fn shown(self, sidebar_layout: bool) -> TabPlacement {
        if sidebar_layout { self } else { TabPlacement::Side }
    }
}

pub fn sidebar_open(store: &Store) -> bool {
    store.setting(SIDEBAR_SETTING).ok().flatten().as_deref() != Some("off")
}

pub fn save_sidebar(store: &Store, open: bool) -> Result<(), store::Error> {
    store.set_setting(SIDEBAR_SETTING, if open { "on" } else { "off" })
}

pub fn tablet(width: f32, height: f32) -> bool {
    width.min(height) >= TABLET_SIDE
}

pub fn offered(width: f32, height: f32) -> bool {
    tablet(width, height) && width >= MAC_WIDTH
}

pub fn effective(setting: Layout, width: f32, height: f32) -> Layout {
    if setting == Layout::Mac && offered(width, height) { Layout::Mac } else { Layout::Touch }
}

/// A page's width while the sidebar slides, held at one size so the page is laid out once, not on
/// every frame: the widest it gets when the window clips what reaches past its edge, else the
/// narrowest. `shown` is how far the sidebar is shown, `full` its whole share of the width.
/// Layout snaps `width` to device pixels, so the result is rounded to whole points to stay put.
pub fn sliding_width(width: f32, shown: f32, full: f32, clipped: bool) -> f32 {
    (if clipped { width + full * shown } else { width - full * (1. - shown) }).round()
}

/// The width each web view keeps while the sidebar slides, by an id for the view. A width is taken
/// once per slide and view, since layout snaps bounds to pixels, which would nudge it by a point.
#[derive(Default)]
pub struct HeldWidths(std::collections::HashMap<usize, (f32, f32)>);

impl HeldWidths {
    /// The width view `id` keeps, laid out `width` wide. `slide` is how far the sidebar is shown and
    /// its share of the view's width, or `None` once it rests. A view its window does not clip
    /// never reaches past its place.
    pub fn width(&mut self, id: usize, width: f32, slide: Option<(f32, f32)>, clipped: bool) -> f32 {
        let Some((shown, full)) = slide else {
            self.0.remove(&id);
            return width;
        };
        let kept = match self.0.get(&id) {
            Some(&(of, kept)) if of == full => kept,
            _ => {
                let kept = sliding_width(width, shown, full, clipped);
                self.0.insert(id, (full, kept));
                kept
            }
        };
        if clipped { kept } else { kept.min(width) }
    }

    /// A new slide takes every width afresh.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Where a slide from `from` to `to` stands at eased progress `t`.
pub fn slide_at(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sliding_page_keeps_one_size_and_never_reaches_left_of_its_place() {
        let (closed, full): (f32, f32) = (1262., 263.);
        for shown in [0., 0.25, 0.5, 0.75, 1.] {
            let laid_out = ((closed - full * shown) * 2.).round() / 2.;
            let wide = sliding_width(laid_out, shown, full, true);
            let narrow = sliding_width(laid_out, shown, full, false);
            assert_eq!(wide, closed, "clipped: the closed-sidebar width throughout, snapped to device pixels or not");
            assert_eq!(narrow, closed - full, "unclipped: the open-sidebar width throughout");
            assert!(narrow <= laid_out + 0.5, "an unclipped page never reaches past its place");
        }
    }

    #[test]
    fn a_held_width_lasts_one_slide_per_view_and_share() {
        let mut held = HeldWidths::default();
        assert_eq!(held.width(1, 1000., Some((0.5, 260.)), true), 1130.);
        assert_eq!(held.width(1, 1060.5, Some((0.25, 260.)), true), 1130., "kept while the slide runs, snapping or not");
        assert_eq!(held.width(2, 500., Some((0.5, 130.)), true), 565., "each view holds its own");
        assert_eq!(held.width(1, 600., Some((0.5, 130.)), true), 665., "a view now in a split pane takes a new share");
        assert_eq!(held.width(1, 1262., None, true), 1262., "at rest the layout width");
        assert_eq!(held.width(1, 1000., Some((0.5, 260.)), true), 1130., "and the next slide takes it afresh");
        assert_eq!(held.width(3, 1000., Some((0.5, 260.)), false), 870.);
        assert_eq!(held.width(3, 800., Some((0.6, 260.)), false), 800., "never past its place when not clipped");
        held.clear();
        assert_eq!(held.width(2, 600., Some((0.5, 130.)), true), 665.);
    }

    #[test]
    fn a_slide_turned_back_midway_starts_where_it_stood() {
        let opening = slide_at(0., 1., 0.4);
        assert_eq!(slide_at(opening, 0., 0.), opening, "no jump when it turns");
        assert_eq!(slide_at(opening, 0., 1.), 0.);
        assert_eq!(slide_at(1., 0., 0.25), 0.75);
    }

    #[test]
    fn touch_is_the_default_and_the_choice_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(Layout::load(&store), Layout::Touch);
        Layout::Mac.save(&store).unwrap();
        assert_eq!(Layout::load(&store), Layout::Mac);
        store.set_setting(SETTING, "bogus").unwrap();
        assert_eq!(Layout::load(&store), Layout::Touch);
    }

    #[test]
    fn the_sidebar_starts_open_and_remembers_being_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(sidebar_open(&store));
        save_sidebar(&store, false).unwrap();
        assert!(!sidebar_open(&store));
        save_sidebar(&store, true).unwrap();
        assert!(sidebar_open(&store));
    }

    #[test]
    fn tabs_start_at_the_side_and_new_windows_take_the_last_pick() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(TabPlacement::load(&store), TabPlacement::Side);
        assert_eq!(TabPlacement::Side.toggled(&store), TabPlacement::Top);
        assert_eq!(TabPlacement::load(&store), TabPlacement::Top);
        assert_eq!(TabPlacement::Top.toggled(&store), TabPlacement::Side);
        assert_eq!(TabPlacement::load(&store), TabPlacement::Side);
        assert_eq!(TabPlacement::Top.shown(true), TabPlacement::Top);
        assert_eq!(TabPlacement::Top.shown(false), TabPlacement::Side, "a phone keeps its bottom bar");
    }

    #[test]
    fn keys_round_trip() {
        for layout in Layout::ALL {
            assert_eq!(Layout::parse(layout.key()), Some(layout));
        }
        assert_eq!(Layout::parse(""), None);
    }

    #[test]
    fn mac_shows_only_on_a_tablet_window_wide_enough_for_the_sidebar() {
        assert_eq!(effective(Layout::Mac, 1032., 1376.), Layout::Mac);
        assert_eq!(effective(Layout::Mac, 1376., 1032.), Layout::Mac);
        assert_eq!(effective(Layout::Mac, 744., 1133.), Layout::Mac);
        assert_eq!(effective(Layout::Mac, 932., 430.), Layout::Touch);
        assert_eq!(effective(Layout::Mac, 430., 932.), Layout::Touch);
        assert_eq!(effective(Layout::Mac, 640., 1376.), Layout::Touch);
        assert_eq!(effective(Layout::Touch, 1032., 1376.), Layout::Touch);
    }

    #[test]
    fn the_choice_is_offered_only_where_mac_can_show() {
        assert!(offered(1032., 1376.));
        assert!(!offered(640., 1376.));
        assert!(!offered(932., 430.));
    }

    #[test]
    fn phones_are_not_tablets_in_either_orientation() {
        assert!(!tablet(430., 932.));
        assert!(!tablet(932., 430.));
        assert!(tablet(744., 1133.));
        assert!(tablet(800., 1280.));
    }
}
