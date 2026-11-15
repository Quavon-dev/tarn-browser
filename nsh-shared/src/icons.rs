use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

pub const APP_WINDOW: &str = include_str!("../../crates/ui/icons/app-window.svg");
pub const ARROW_DOWN: &str = include_str!("../../crates/ui/icons/arrow-down.svg");
pub const BACK: &str = include_str!("../../crates/ui/icons/back.svg");
pub const BOOK_OPEN: &str = include_str!("../../crates/ui/icons/book-open.svg");
pub const BRIEFCASE: &str = include_str!("../../crates/ui/icons/briefcase.svg");
pub const CAMERA: &str = include_str!("../../crates/ui/icons/camera.svg");
pub const CHEVRON_DOWN: &str = include_str!("../../crates/ui/icons/chevron-down.svg");
pub const FILE_PLAIN: &str = include_str!("../../crates/ui/icons/file.svg");
pub const CHEVRON_UP: &str = include_str!("../../crates/ui/icons/chevron-up.svg");
pub const CHEVRON_LEFT: &str = include_str!("../../crates/ui/icons/chevron-left.svg");
pub const CHEVRON_RIGHT: &str = include_str!("../../crates/ui/icons/chevron-right.svg");
pub const CIRCLE_ARROW_DOWN: &str = include_str!("../../crates/ui/icons/circle-arrow-down.svg");
pub const CLOSE: &str = include_str!("../../crates/ui/icons/close.svg");
pub const CODE: &str = include_str!("../../crates/ui/icons/code.svg");
pub const COFFEE: &str = include_str!("../../crates/ui/icons/coffee.svg");
pub const DUMBBELL: &str = include_str!("../../crates/ui/icons/dumbbell.svg");
pub const FLASK_CONICAL: &str = include_str!("../../crates/ui/icons/flask-conical.svg");
pub const FOLDER: &str = include_str!("../../crates/ui/icons/folder.svg");
pub const FOLDER_PLUS: &str = include_str!("../../crates/ui/icons/folder-plus.svg");
pub const FORWARD: &str = include_str!("../../crates/ui/icons/forward.svg");
pub const GAMEPAD_2: &str = include_str!("../../crates/ui/icons/gamepad-2.svg");
pub const GEAR: &str = include_str!("../../crates/ui/icons/gear.svg");
pub const GLOBE: &str = include_str!("../../crates/ui/icons/globe.svg");
pub const GRADUATION_CAP: &str = include_str!("../../crates/ui/icons/graduation-cap.svg");
pub const HEART: &str = include_str!("../../crates/ui/icons/heart.svg");
pub const HISTORY: &str = include_str!("../../crates/ui/icons/history.svg");
pub const HOUSE: &str = include_str!("../../crates/ui/icons/house.svg");
pub const MUSIC: &str = include_str!("../../crates/ui/icons/music.svg");
pub const PALETTE: &str = include_str!("../../crates/ui/icons/palette.svg");
pub const PERSON: &str = include_str!("../../crates/ui/icons/person.svg");
pub const PLANE: &str = include_str!("../../crates/ui/icons/plane.svg");
pub const PLUS: &str = include_str!("../../crates/ui/icons/plus.svg");
pub const RELOAD: &str = include_str!("../../crates/ui/icons/reload.svg");
pub const SEARCH: &str = include_str!("../../crates/ui/icons/search.svg");
pub const SHIELD: &str = include_str!("../../crates/ui/icons/shield.svg");
pub const SHIELD_CHECK: &str = include_str!("../../crates/ui/icons/shield-check.svg");
pub const SHOPPING_CART: &str = include_str!("../../crates/ui/icons/shopping-cart.svg");
pub const SIDEBAR: &str = include_str!("../../crates/ui/icons/sidebar.svg");
pub const SPARKLES: &str = include_str!("../../crates/ui/icons/sparkles.svg");
pub const STAR: &str = include_str!("../../crates/ui/icons/star.svg");
pub const WALLET: &str = include_str!("../../crates/ui/icons/wallet.svg");
pub const BOOKMARK: &str = include_str!("../../crates/ui/icons/bookmark.svg");
pub const CHECK: &str = include_str!("../../crates/ui/icons/check.svg");
pub const FLAME: &str = include_str!("../../crates/ui/icons/flame.svg");
pub const GAUGE: &str = include_str!("../../crates/ui/icons/gauge.svg");
pub const HAND: &str = include_str!("../../crates/ui/icons/hand.svg");
pub const KEYBOARD: &str = include_str!("../../crates/ui/icons/keyboard.svg");
pub const PLAY: &str = include_str!("../../crates/ui/icons/play.svg");
pub const SMARTPHONE: &str = include_str!("../../crates/ui/icons/smartphone.svg");
pub const TRASH: &str = include_str!("../../crates/ui/icons/trash.svg");
pub const VOLUME_X: &str = include_str!("../../crates/ui/icons/volume-x.svg");
pub const MARK: &str = include_str!("../../crates/ui/brand/mark.svg");

pub const SPACE_ICONS: &[(&str, &str)] = &[
    ("house", HOUSE),
    ("briefcase", BRIEFCASE),
    ("code", CODE),
    ("book-open", BOOK_OPEN),
    ("graduation-cap", GRADUATION_CAP),
    ("flask-conical", FLASK_CONICAL),
    ("palette", PALETTE),
    ("music", MUSIC),
    ("gamepad-2", GAMEPAD_2),
    ("shopping-cart", SHOPPING_CART),
    ("plane", PLANE),
    ("heart", HEART),
    ("star", STAR),
    ("coffee", COFFEE),
    ("camera", CAMERA),
    ("globe", GLOBE),
    ("wallet", WALLET),
    ("dumbbell", DUMBBELL),
    ("sparkles", SPARKLES),
    ("user", PERSON),
];

pub fn space_icon(name: &str) -> &'static str {
    SPACE_ICONS.iter().find(|(n, _)| *n == name).map_or(HOUSE, |(_, i)| i)
}

pub fn tinted(source: &str) -> String {
    source.replace("\"#000\"", "\"currentColor\"")
}

pub fn icon(source: &str, size: f32) -> Svg {
    svg(ImageSrc::svg_markup(&tinted(source))).w(px(size)).h(px(size))
}

pub fn picture(markup: &str, width: f32, height: f32) -> Svg {
    svg(ImageSrc::svg_markup(markup)).w(px(width)).h(px(height))
}

pub fn mark(size: f32) -> Svg {
    svg(ImageSrc::svg_markup(MARK)).w(px(size)).h(px(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_take_the_text_colour_and_spaces_default_to_the_house() {
        assert!(tinted(FOLDER).contains("fill=\"currentColor\" stroke=\"currentColor\""));
        assert!(!tinted(BACK).contains("#000"));
        assert_eq!(space_icon("nope"), HOUSE);
        assert_eq!(space_icon("code"), CODE);
    }
}
