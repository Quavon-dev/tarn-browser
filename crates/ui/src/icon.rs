use std::borrow::Cow;

use gpui::{AssetSource, Rgba, SharedString, Styled, Svg, px, svg};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon {
            $($variant),*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$variant),*];

            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$variant => concat!("icons/", $file, ".svg")),*
                }
            }
        }

        fn bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $file, ".svg") => Some(include_bytes!(concat!("../icons/", $file, ".svg"))),)*
                _ => None,
            }
        }
    };
}

icons! {
    Sidebar => "sidebar",
    Back => "back",
    Forward => "forward",
    Reload => "reload",
    Folder => "folder",
    FolderPlus => "folder-plus",
    ChevronRight => "chevron-right",
    ChevronDown => "chevron-down",
    Plus => "plus",
    Close => "close",
    Search => "search",
    Gear => "gear",
    Hand => "hand",
    Person => "person",
    Keyboard => "keyboard",
    Shield => "shield",
    ShieldCheck => "shield-check",
    Code => "code",
    Smartphone => "smartphone",
    Trash => "trash",
    Braces => "braces",
    Pencil => "pencil",
    Palette => "palette",
    AppWindow => "app-window",
    History => "history",
    Bookmark => "bookmark",
    Group => "group",
    ChevronLeft => "chevron-left",
    ChevronUp => "chevron-up",
    Star => "star",
    StarFill => "star-fill",
    Check => "check",
    Command => "command",
    House => "house",
    Briefcase => "briefcase",
    BookOpen => "book-open",
    GraduationCap => "graduation-cap",
    FlaskConical => "flask-conical",
    Music => "music",
    Gamepad => "gamepad-2",
    ShoppingCart => "shopping-cart",
    Plane => "plane",
    Heart => "heart",
    Coffee => "coffee",
    Camera => "camera",
    Globe => "globe",
    Wallet => "wallet",
    Dumbbell => "dumbbell",
    Sparkles => "sparkles",
    Download => "circle-arrow-down",
    Flame => "flame",
    File => "file",
    ArrowDown => "arrow-down",
    ArrowUpRight => "arrow-up-right",
    Caret => "caret",
    VolumeX => "volume-x",
    Undo => "undo-2",
    Gauge => "gauge",
    Pause => "pause",
    Play => "play",
    Ellipsis => "ellipsis",
}

/// Icons a space can show, stored by Lucide name in `spaces.icon`; the first is the default.
pub const SPACE_ICONS: &[(&str, Icon)] = &[
    ("house", Icon::House),
    ("briefcase", Icon::Briefcase),
    ("code", Icon::Code),
    ("book-open", Icon::BookOpen),
    ("graduation-cap", Icon::GraduationCap),
    ("flask-conical", Icon::FlaskConical),
    ("palette", Icon::Palette),
    ("music", Icon::Music),
    ("gamepad-2", Icon::Gamepad),
    ("shopping-cart", Icon::ShoppingCart),
    ("plane", Icon::Plane),
    ("heart", Icon::Heart),
    ("star", Icon::Star),
    ("coffee", Icon::Coffee),
    ("camera", Icon::Camera),
    ("globe", Icon::Globe),
    ("wallet", Icon::Wallet),
    ("dumbbell", Icon::Dumbbell),
    ("sparkles", Icon::Sparkles),
    ("user", Icon::Person),
];

pub fn space_icon(name: &str) -> Icon {
    SPACE_ICONS.iter().find(|(n, _)| *n == name).map_or(Icon::House, |(_, i)| *i)
}

pub const MARK: &str = "brand/mark.svg";

pub fn icon(icon: Icon, size: f32, color: Rgba) -> Svg {
    svg().path(icon.path()).size(px(size)).flex_none().text_color(color)
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let mark = (path == MARK).then_some(include_bytes!("../brand/mark.svg").as_slice());
        Ok(bytes(path).or(mark).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Icon::ALL.iter().map(|i| i.path()).filter(|p| p.starts_with(path)).map(SharedString::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_an_asset() {
        for icon in Icon::ALL {
            let data = Assets.load(icon.path()).unwrap().unwrap_or_else(|| panic!("{icon:?} missing"));
            let text = std::str::from_utf8(&data).unwrap();
            assert!(text.starts_with("<svg") && text.trim_end().ends_with("</svg>"), "{icon:?}");
        }
        assert!(Assets.load(MARK).unwrap().is_some());
        assert!(Assets.load("icons/none.svg").unwrap().is_none());
    }

    #[test]
    fn unknown_space_icons_fall_back_to_house() {
        assert_eq!(space_icon(""), Icon::House);
        assert_eq!(space_icon("../x"), Icon::House);
        assert_eq!(space_icon("gamepad-2"), Icon::Gamepad);
    }
}
