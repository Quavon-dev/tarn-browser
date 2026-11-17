use std::cell::RefCell;
use std::collections::HashMap;

use engine::menu::{Image, Item};
use gpui::AssetSource;
use resvg::tiny_skia::{Pixmap, Transform};

const SIDE: u32 = 32;
const INK: &str = if cfg!(windows) { "#6b7280" } else { "#000000" };

thread_local! {
    static DRAWN: RefCell<HashMap<&'static str, Option<Vec<u8>>>> = RefCell::default();
}

fn png(name: &'static str) -> Option<Vec<u8>> {
    let icon = ui::icon::Icon::named(name)?;
    let svg = ui::icon::Assets.load(icon.path()).ok()??;
    let svg = std::str::from_utf8(&svg).ok()?.replace("currentColor", INK);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let scale = SIDE as f32 / tree.size().width().max(tree.size().height());
    let mut pixmap = Pixmap::new(SIDE, SIDE)?;
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

fn glyph(label: &str) -> Option<Image> {
    let name = browser_core::command::glyph(label);
    if name == "command" {
        return None;
    }
    DRAWN.with(|drawn| drawn.borrow_mut().entry(name).or_insert_with(|| png(name)).clone()).map(Image::Template)
}

pub fn with_glyphs<T>(items: Vec<Item<T>>) -> Vec<Item<T>> {
    items
        .into_iter()
        .map(|item| {
            let glyph = match &item {
                Item::Action { title, image: None, .. } => glyph(title),
                _ => None,
            };
            if glyph.is_some() { item.image(glyph) } else { item }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_entries_get_their_label_glyph_and_names_none() {
        let items =
            with_glyphs(vec![Item::action("Copy Link", 1), Item::Separator, Item::action("Holiday plans", 2), Item::Submenu("Move to Space".into(), vec![])]);
        let images: Vec<bool> =
            items.iter().map(|i| matches!(i, Item::Action { image: Some(Image::Template(png)), .. } if png.starts_with(b"\x89PNG"))).collect();
        assert_eq!(images, [true, false, false, false]);
    }
}
