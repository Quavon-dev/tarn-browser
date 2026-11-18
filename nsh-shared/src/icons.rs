use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

pub const APP_WINDOW: &str = include_str!("../../crates/ui/icons/app-window.svg");
pub const ARROW_UP_RIGHT: &str = include_str!("../../crates/ui/icons/arrow-up-right.svg");
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
pub const LIBRARY: &str = include_str!("../../crates/ui/icons/library.svg");
pub const PIN: &str = include_str!("../../crates/ui/icons/pin.svg");
pub const VOLUME_2: &str = include_str!("../../crates/ui/icons/volume-2.svg");
pub const MOON: &str = include_str!("../../crates/ui/icons/moon.svg");
pub const SHARE: &str = include_str!("../../crates/ui/icons/share.svg");
pub const QR_CODE: &str = include_str!("../../crates/ui/icons/qr-code.svg");
pub const LINK: &str = include_str!("../../crates/ui/icons/link.svg");
pub const FILE_DOWN: &str = include_str!("../../crates/ui/icons/file-down.svg");
pub const PRINTER: &str = include_str!("../../crates/ui/icons/printer.svg");
pub const MAP_PIN: &str = include_str!("../../crates/ui/icons/map-pin.svg");
pub const MIC: &str = include_str!("../../crates/ui/icons/mic.svg");
pub const BELL: &str = include_str!("../../crates/ui/icons/bell.svg");
pub const TRIANGLE_ALERT: &str = include_str!("../../crates/ui/icons/triangle-alert.svg");
pub const KEY_ROUND: &str = include_str!("../../crates/ui/icons/key-round.svg");
pub const EXTERNAL_LINK: &str = include_str!("../../crates/ui/icons/external-link.svg");
pub const ZOOM_IN: &str = include_str!("../../crates/ui/icons/zoom-in.svg");
pub const FOLDER_OPEN: &str = include_str!("../../crates/ui/icons/folder-open.svg");
pub const FILE_TEXT: &str = include_str!("../../crates/ui/icons/file-text.svg");
pub const FILE_ARCHIVE: &str = include_str!("../../crates/ui/icons/file-archive.svg");
pub const FILE_IMAGE: &str = include_str!("../../crates/ui/icons/file-image.svg");
pub const FILE_VIDEO: &str = include_str!("../../crates/ui/icons/file-video.svg");
pub const LAYOUT_GRID: &str = include_str!("../../crates/ui/icons/layout-grid.svg");
pub const USER_PLUS: &str = include_str!("../../crates/ui/icons/user-plus.svg");
pub const ARCHIVE: &str = include_str!("../../crates/ui/icons/archive.svg");
pub const COPY: &str = include_str!("../../crates/ui/icons/copy.svg");
pub const ZAP: &str = include_str!("../../crates/ui/icons/zap.svg");
pub const COLUMNS_2: &str = include_str!("../../crates/ui/icons/columns-2.svg");
pub const ARROW_LEFT_RIGHT: &str = include_str!("../../crates/ui/icons/arrow-left-right.svg");
pub const PICTURE_IN_PICTURE_2: &str = include_str!("../../crates/ui/icons/picture-in-picture-2.svg");
pub const LOCK: &str = include_str!("../../crates/ui/icons/lock.svg");
pub const CHEVRONS_UP_DOWN: &str = include_str!("../../crates/ui/icons/chevrons-up-down.svg");
pub const INFO: &str = include_str!("../../crates/ui/icons/info.svg");
pub const CIRCLE_CHECK: &str = include_str!("../../crates/ui/icons/circle-check.svg");
pub const BROOM_SPARKLES: &str = include_str!("../../crates/ui/icons/broom-sparkles.svg");
pub const SPLIT_ADD: &str = include_str!("../../crates/ui/icons/split-add.svg");
pub const COMMAND: &str = include_str!("../../crates/ui/icons/command.svg");
pub const ELLIPSIS: &str = include_str!("../../crates/ui/icons/ellipsis.svg");
pub const GROUP: &str = include_str!("../../crates/ui/icons/group.svg");
pub const PENCIL: &str = include_str!("../../crates/ui/icons/pencil.svg");
pub const UNDO_2: &str = include_str!("../../crates/ui/icons/undo-2.svg");
pub const PAUSE: &str = include_str!("../../crates/ui/icons/pause.svg");
pub const STAR_FILL: &str = include_str!("../../crates/ui/icons/star-fill.svg");
pub const MARK: &str = include_str!("../../crates/ui/brand/mark.svg");

pub const NAMED: &[(&str, &str)] = &[
    ("app-window", APP_WINDOW),
    ("arrow-up-right", ARROW_UP_RIGHT),
    ("arrow-down", ARROW_DOWN),
    ("back", BACK),
    ("book-open", BOOK_OPEN),
    ("briefcase", BRIEFCASE),
    ("camera", CAMERA),
    ("chevron-down", CHEVRON_DOWN),
    ("file", FILE_PLAIN),
    ("chevron-up", CHEVRON_UP),
    ("chevron-left", CHEVRON_LEFT),
    ("chevron-right", CHEVRON_RIGHT),
    ("circle-arrow-down", CIRCLE_ARROW_DOWN),
    ("close", CLOSE),
    ("code", CODE),
    ("coffee", COFFEE),
    ("dumbbell", DUMBBELL),
    ("flask-conical", FLASK_CONICAL),
    ("folder", FOLDER),
    ("folder-plus", FOLDER_PLUS),
    ("forward", FORWARD),
    ("gamepad-2", GAMEPAD_2),
    ("gear", GEAR),
    ("globe", GLOBE),
    ("graduation-cap", GRADUATION_CAP),
    ("heart", HEART),
    ("history", HISTORY),
    ("house", HOUSE),
    ("music", MUSIC),
    ("palette", PALETTE),
    ("person", PERSON),
    ("plane", PLANE),
    ("plus", PLUS),
    ("reload", RELOAD),
    ("search", SEARCH),
    ("shield", SHIELD),
    ("shield-check", SHIELD_CHECK),
    ("shopping-cart", SHOPPING_CART),
    ("sidebar", SIDEBAR),
    ("sparkles", SPARKLES),
    ("star", STAR),
    ("wallet", WALLET),
    ("bookmark", BOOKMARK),
    ("check", CHECK),
    ("flame", FLAME),
    ("gauge", GAUGE),
    ("hand", HAND),
    ("keyboard", KEYBOARD),
    ("play", PLAY),
    ("smartphone", SMARTPHONE),
    ("trash", TRASH),
    ("volume-x", VOLUME_X),
    ("library", LIBRARY),
    ("pin", PIN),
    ("volume-2", VOLUME_2),
    ("moon", MOON),
    ("share", SHARE),
    ("qr-code", QR_CODE),
    ("link", LINK),
    ("file-down", FILE_DOWN),
    ("printer", PRINTER),
    ("map-pin", MAP_PIN),
    ("mic", MIC),
    ("bell", BELL),
    ("triangle-alert", TRIANGLE_ALERT),
    ("key-round", KEY_ROUND),
    ("external-link", EXTERNAL_LINK),
    ("zoom-in", ZOOM_IN),
    ("folder-open", FOLDER_OPEN),
    ("file-text", FILE_TEXT),
    ("file-archive", FILE_ARCHIVE),
    ("file-image", FILE_IMAGE),
    ("file-video", FILE_VIDEO),
    ("layout-grid", LAYOUT_GRID),
    ("user-plus", USER_PLUS),
    ("archive", ARCHIVE),
    ("copy", COPY),
    ("zap", ZAP),
    ("columns-2", COLUMNS_2),
    ("arrow-left-right", ARROW_LEFT_RIGHT),
    ("picture-in-picture-2", PICTURE_IN_PICTURE_2),
    ("lock", LOCK),
    ("chevrons-up-down", CHEVRONS_UP_DOWN),
    ("info", INFO),
    ("circle-check", CIRCLE_CHECK),
    ("broom-sparkles", BROOM_SPARKLES),
    ("split-add", SPLIT_ADD),
    ("command", COMMAND),
    ("ellipsis", ELLIPSIS),
    ("group", GROUP),
    ("pencil", PENCIL),
    ("undo-2", UNDO_2),
    ("pause", PAUSE),
    ("star-fill", STAR_FILL),
];

pub fn named(name: &str) -> Option<&'static str> {
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, i)| *i)
}

pub fn glyph(name: &str) -> &'static str {
    named(name).unwrap_or(COMMAND)
}

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
        assert_eq!(SPACE_ICONS[0].0, browser_core::space_card::DEFAULT_SPACE_ICON, "a new space saves the first icon the card shows");
        assert_eq!(space_icon("code"), CODE);
        let missing: Vec<&str> =
            browser_core::command::GLYPHS.iter().map(|(_, g)| *g).chain(browser_core::icon::SYMBOLS).filter(|g| named(g).is_none()).collect();
        assert!(missing.is_empty(), "{missing:?}");
        for kind in ["camera", "microphone", "location", "notifications", "credentials", "external", "alert"] {
            assert!(named(browser_core::glyphs::glyph(kind)).is_some(), "{kind}");
        }
        assert_eq!(glyph("nope"), COMMAND);
    }
}
