//! The profile's Shield card (issue #3): the brand mark's glacier lake, its ridges and time of day
//! drawn from a random value stored with the profile, and the card as a PNG for "Save as image".
//! Everything is drawn on this Mac.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use crate::browser::Browser;
pub use browser_core::shield_card::{Card, H, RIDGES, W, back_svg, front_svg, landscape};
use resvg::tiny_skia::Pixmap;

/// Room around the card in a tilted frame, so its near edge is not cut off.
pub const MARGIN: u32 = 14;
/// How far the near ridges float above the card under the pointer, in texture pixels.
const LIFT: f32 = 22.;

/// Bumped whenever a counter or design changes in any window, so an open card reads again.
static CHANGES: AtomicU64 = AtomicU64::new(0);

pub fn touch() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

/// The current profile's card, read from its store; the private profile shows no dates.
pub fn for_profile(b: &Browser) -> Option<Card> {
    let space = b.model.current_space()?;
    let profile = b.current_profile()?;
    browser_core::shield_card::read(b.store(space.private), profile.id, space.private, ui::text::bounded_title(&profile.name), browser_core::now())
}

/// `text` loads the system fonts once, which takes a moment; the ridges have no text.
fn rasterize(svg: &str, width: u32, text: bool) -> Option<Pixmap> {
    static FONTS: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    let options = if text { resvg::usvg::Options { fontdb: FONTS.clone(), ..Default::default() } } else { resvg::usvg::Options::default() };
    let tree = resvg::usvg::Tree::from_str(svg, &options).ok()?;
    let scale = width as f32 / tree.size().width();
    let mut pixmap = Pixmap::new(width, (tree.size().height() * scale).round() as u32)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(pixmap)
}
/// The card's front as a PNG at four times its size, for "Save as image".
pub fn card_png(card: &Card) -> Option<Vec<u8>> {
    rasterize(&front_svg(card, true), W * 4, true)?.encode_png().ok()
}

/// What the tilting card is drawn from: both faces, and the near ridges that float above the front.
pub struct Faces {
    front: Pixmap,
    ridges: Pixmap,
    back: Pixmap,
}

pub fn faces(card: &Card) -> Option<Faces> {
    let (top, height) = RIDGES;
    let ridges =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" viewBox="0 {top} {W} {height}">{}</svg>"#, landscape(card.seed).2);
    Some(Faces {
        front: rasterize(&front_svg(card, false), W * 2, true)?,
        ridges: rasterize(&ridges, W * 2, false)?,
        back: rasterize(&back_svg(card), W * 2, true)?,
    })
}

/// The card under `pose`, as the BGRA image GPUI draws, `MARGIN` points larger on every side.
pub fn frame(faces: &Faces, pose: &ui::tilt::Pose) -> image::RgbaImage {
    fn texture(p: &Pixmap) -> ui::tilt::Texture<'_> {
        ui::tilt::Texture { rgba: p.data(), width: p.width(), height: p.height() }
    }
    let raised = (&texture(&faces.ridges), 0., RIDGES.0 as f32 * 2., LIFT);
    let bytes = ui::tilt::frame(&texture(&faces.front), &texture(&faces.back), raised, MARGIN * 2, pose);
    image::RgbaImage::from_raw((W + 2 * MARGIN) * 2, (H + 2 * MARGIN) * 2, bytes).expect("the frame is the card and its margin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::shield_card::stamps;
    use shield::ShieldLevel;

    fn card(name: &str) -> Card {
        Card {
            name: name.to_owned(),
            level: ShieldLevel::Strict,
            seed: 42,
            since: Some("2026-01-02".into()),
            days: Some(316),
            counters: [("Tracking parameters removed", 12), ("Pages upgraded to HTTPS", 3), ("Local network visits blocked", 0)],
            stamps: stamps(Some(2025), 2026, 150),
            week: Vec::new(),
            sites: Vec::new(),
        }
    }

    #[test]
    fn designs_follow_the_seed() {
        assert_eq!(landscape(7).1, landscape(7).1);
        assert_ne!(landscape(7).1, landscape(8).1);
        assert_ne!(front_svg(&card("Personal"), true), front_svg(&Card { level: ShieldLevel::Lockdown, ..card("Personal") }, true));
        let skies: std::collections::HashSet<_> = (0..40).map(|seed| landscape(seed).0.sky).collect();
        assert_eq!(skies.len(), 4, "every time of day comes up");
        for seed in [0, 1, 2, 3, 99, i64::MAX] {
            let faces = faces(&Card { seed, ..card("Personal") }).unwrap();
            assert_eq!((faces.ridges.width(), faces.ridges.height()), (W * 2, RIDGES.1 * 2));
            assert!(faces.ridges.pixels().iter().any(|p| p.alpha() > 0));
        }
    }

    #[test]
    fn the_card_escapes_its_text_and_renders() {
        let svg = front_svg(&card("<script>&\"x\u{7}"), true);
        assert!(svg.contains("&lt;script&gt;&amp;&quot;x<") && !svg.contains('\u{7}'));
        assert!(resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).is_ok());
        let png = card_png(&card("Personal")).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!((image.width(), image.height()), (W * 4, H * 4));
    }

    #[test]
    fn the_card_fits_a_private_profile_and_many_stamps() {
        let private = Card { since: None, days: None, stamps: Vec::new(), ..card("Private") };
        assert!(!front_svg(&private, true).contains("Protected for") && back_svg(&private).contains("No stamps yet"));
        let old = Card { stamps: stamps(Some(2012), 2026, 20_000), counters: [("", 1_234_567), ("", 0), ("", 0)], ..card("A name far too long for the card") };
        let (front, back) = (front_svg(&old, true), back_svg(&old));
        assert!(front.contains(r#"font-size="16""#) && front.contains("A name far too long fo…"));
        assert_eq!(back.matches("<g transform=\"rotate").count(), 10);
        assert!(back.contains("10K ✓") && !back.contains(">2012<") && back.contains("Every milestone reached"));
        assert!(faces(&old).is_some());
    }

    #[test]
    fn the_frame_shows_the_front_at_rest_and_the_back_once_flipped() {
        let faces = faces(&card("Personal")).unwrap();
        let (ow, oh) = ((W + 2 * MARGIN) * 2, (H + 2 * MARGIN) * 2);
        let pose = |tilt, glow, flip| ui::tilt::Pose { tilt, glow, flip, press: 0. };
        let rest = frame(&faces, &pose((0., 0.), 0., 0.));
        assert_eq!((rest.width(), rest.height()), (ow, oh));
        assert_eq!(rest.get_pixel(2, 2).0[3], 0, "the margin stays clear");
        let at = |image: &image::RgbaImage, x: u32, y: u32| image.get_pixel(MARGIN * 2 + x, MARGIN * 2 + y).0;
        let texel = |pixmap: &Pixmap, x: u32, y: u32| pixmap.pixel(x, y).unwrap().demultiply();
        let (seen, drawn) = (at(&rest, 500, 300), texel(&faces.front, 500, 300));
        assert!(seen[2].abs_diff(drawn.red()) <= 2 && seen[0].abs_diff(drawn.blue()) <= 2 && seen[3] == 255, "BGRA, unchanged at rest");
        let flipped = frame(&faces, &pose((0., 0.), 0., 1.));
        let (seen, drawn) = (at(&flipped, 500, 300), texel(&faces.back, 500, 300));
        assert!(seen[2].abs_diff(drawn.red()) <= 2 && seen[1].abs_diff(drawn.green()) <= 2, "the back reads the right way round");
        assert_ne!(frame(&faces, &pose((1., -1.), 1., 0.)), rest);
        let lifted = frame(&faces, &pose((0., 0.), 1., 0.));
        assert_ne!(lifted, rest, "the ridges rise under the pointer");
        assert!((190..230).all(|y| lifted.get_pixel(MARGIN * 2 - 3, MARGIN * 2 + y).0[3] == 0), "the ridges stay on the card");
        assert!(frame(&faces, &pose((0., 0.), 0., 0.5)).pixels().all(|p| p.0[3] == 0), "edge-on shows nothing");
    }
}
