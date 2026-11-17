use store::Store;

pub const SETTING: &str = "layout.tablet";
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

pub fn tablet(width: f32, height: f32) -> bool {
    width.min(height) >= TABLET_SIDE
}

pub fn offered(width: f32, height: f32) -> bool {
    tablet(width, height) && width >= MAC_WIDTH
}

pub fn effective(setting: Layout, width: f32, height: f32) -> Layout {
    if setting == Layout::Mac && offered(width, height) { Layout::Mac } else { Layout::Touch }
}

#[cfg(test)]
mod tests {
    use super::*;

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
