use std::io::Write;
use std::path::{Path, PathBuf};

use engine::view_policy::SnapshotArea;

/// What Take Screenshot captures: the visible page, an area the user drags over it, an element
/// they pick, or the whole document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Visible,
    Area,
    Element,
    FullPage,
}

impl Mode {
    /// Whether the mode runs a script of Tarn's on the page (the picker), which Lockdown does not.
    pub fn needs_script(self) -> bool {
        self == Mode::Element
    }
}

/// Whether a capture also goes on the system clipboard, which may carry it to other devices:
/// never from a private space or burner tab, whose captures are only saved.
pub fn copies(private: bool) -> bool {
    !private
}

/// The toast after a screenshot was saved, and copied when `copies` allowed it.
pub fn saved_note(copied: bool) -> &'static str {
    if copied { "Screenshot saved and copied" } else { "Screenshot saved" }
}

/// The notice for Capture Element on a Lockdown page, where the picker cannot run.
pub const NO_PICKER: &str = "Capture Element needs Tarn's picker, which Lockdown turns off";
/// The notice when the picked element has no part inside the page's visible area.
pub const OFF_SCREEN: &str = "Nothing of that element is on screen";
/// What every client says while the user drags out an area or picks an element.
pub const AREA_HINT: &str = "Drag over the area to capture. Esc cancels.";
pub const ELEMENT_HINT: &str = "Click the element to capture. Esc cancels.";
/// The hints on a touch screen, which cancels with the overlay's Cancel button.
pub const AREA_HINT_TOUCH: &str = "Drag over the area to capture.";
pub const ELEMENT_HINT_TOUCH: &str = "Tap the element to capture.";

/// `area` as the NSH component's `screenshot` and `snapshotRect` arguments, which
/// `page_ops::snapshot_area` reads back.
pub fn area_args(area: SnapshotArea) -> serde_json::Value {
    match area {
        SnapshotArea::FullPage => serde_json::json!({ "fullPage": true }),
        SnapshotArea::Rect { x, y, width, height } => serde_json::json!({ "x": x, "y": y, "width": width, "height": height }),
    }
}

/// A shorter drag is a click and captures nothing, in CSS pixels.
pub const MIN_SIDE: f64 = 4.;

/// The rectangle `[x, y, width, height]` a drag from `from` to `to` spans, cut to a visible page of
/// size `page`; empty sides stay 0.
pub fn drag_rect(from: (f64, f64), to: (f64, f64), page: (f64, f64)) -> [f64; 4] {
    let clamp = |v: f64, max: f64| if v.is_finite() { v.clamp(0., max.max(0.)) } else { 0. };
    let (x0, x1) = (clamp(from.0, page.0), clamp(to.0, page.0));
    let (y0, y1) = (clamp(from.1, page.1), clamp(to.1, page.1));
    [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()]
}

/// The area a drag selects for Capture Area; `None` for a drag shorter than `MIN_SIDE` either way.
pub fn drag_area(from: (f64, f64), to: (f64, f64), page: (f64, f64)) -> Option<SnapshotArea> {
    let [x, y, w, h] = drag_rect(from, to, page);
    (w >= MIN_SIDE && h >= MIN_SIDE).then(|| SnapshotArea::rect(x, y, w, h, 1.)).flatten()
}

/// The picked element's area from `page_scripts::PICK_RECT`'s answer `[x, y, width, height]`, cut
/// to the visible page; `None` for a cancelled pick, an element out of view or a malformed answer.
pub fn element_area(answer: &str, page: (f64, f64)) -> Option<SnapshotArea> {
    let [x, y, w, h]: [f64; 4] = serde_json::from_str(answer).ok()?;
    let [x, y, w, h] = drag_rect((x, y), (x + w, y + h), page);
    (w >= 1. && h >= 1.).then(|| SnapshotArea::rect(x, y, w, h, 1.)).flatten()
}

pub fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = engine::downloads::free_path(dir, name, |_| false);
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?.write_all(bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_capture_never_reaches_the_clipboard() {
        assert!(copies(false));
        assert!(!copies(true));
        assert_eq!(saved_note(copies(true)), "Screenshot saved");
    }

    #[test]
    fn a_drag_is_cut_to_the_page_in_any_direction_and_a_click_is_no_area() {
        let page = (800., 600.);
        assert_eq!(drag_area((10., 20.), (110., 70.), page), Some(SnapshotArea::Rect { x: 10., y: 20., width: 100., height: 50. }));
        assert_eq!(drag_area((110., 70.), (10., 20.), page), drag_area((10., 20.), (110., 70.), page), "upward and leftward drags");
        assert_eq!(drag_area((-50., -50.), (900., 700.), page), Some(SnapshotArea::Rect { x: 0., y: 0., width: 800., height: 600. }));
        assert_eq!(drag_area((10., 10.), (12., 300.), page), None, "too narrow");
        assert_eq!(drag_area((10., 10.), (10., 10.), page), None, "a click");
        assert_eq!(drag_area((f64::NAN, 0.), (100., 100.), page), Some(SnapshotArea::Rect { x: 0., y: 0., width: 100., height: 100. }));
        assert_eq!(drag_rect((5., 5.), (5., 5.), (0., 0.)), [0., 0., 0., 0.]);
    }

    #[test]
    fn a_picked_element_is_cut_to_the_visible_page() {
        let page = (800., 600.);
        assert_eq!(element_area("[10, 20, 300, 40]", page), Some(SnapshotArea::Rect { x: 10., y: 20., width: 300., height: 40. }));
        assert_eq!(element_area("[-20, 500, 2000, 400]", page), Some(SnapshotArea::Rect { x: 0., y: 500., width: 800., height: 100. }));
        assert_eq!(element_area("[10, 900, 100, 100]", page), None, "out of view");
        assert_eq!(element_area("", page), None, "cancelled or Lockdown");
        assert_eq!(element_area("#ad", page), None);
        assert_eq!(element_area("[1, 2, 3]", page), None);
        assert!(Mode::Element.needs_script() && !Mode::Area.needs_script() && !Mode::FullPage.needs_script());
        for area in [SnapshotArea::FullPage, SnapshotArea::Rect { x: 1., y: 2., width: 30., height: 40. }] {
            assert_eq!(crate::page_ops::snapshot_area(&area_args(area), 1.), Some(area));
        }
    }

    #[test]
    fn saves_never_replace_a_file_or_follow_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_new(dir.path(), "shot.png", b"one").unwrap();
        assert_eq!(first, dir.path().join("shot.png"));
        let second = save_new(dir.path(), "shot.png", b"two").unwrap();
        assert_ne!(second, first);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("link.png")).unwrap();
            assert_ne!(save_new(dir.path(), "link.png", b"three").unwrap(), dir.path().join("link.png"));
            assert!(!dir.path().join("elsewhere").exists());
        }
    }
}
