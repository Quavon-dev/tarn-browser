/// What a list in the bundle blocks; each is a switch in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Ads,
    Trackers,
    Malware,
    Adult,
    Popups,
    Annoyances,
}

impl Category {
    pub const ALL: [Category; 6] = [Category::Ads, Category::Trackers, Category::Malware, Category::Adult, Category::Popups, Category::Annoyances];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Ads => "ads",
            Category::Trackers => "trackers",
            Category::Malware => "malware",
            Category::Adult => "adult",
            Category::Popups => "popups",
            Category::Annoyances => "annoyances",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// Categories whose sets stop top-level navigations with a warning page.
    pub fn warns(self) -> bool {
        matches!(self, Category::Malware | Category::Adult)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
        }
        assert_eq!(Category::parse("youtube"), None);
    }
}
