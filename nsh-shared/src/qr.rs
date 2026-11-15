use std::fmt::Write;

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

pub fn markup(code: &[Vec<bool>], quiet: usize) -> String {
    let side = code.len() + 2 * quiet;
    let mut path = String::new();
    for (y, row) in code.iter().enumerate() {
        for (x, _) in row.iter().enumerate().filter(|(_, dark)| **dark) {
            let _ = write!(path, "M{} {}h1v1h-1z", x + quiet, y + quiet);
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" shape-rendering=\"crispEdges\"><rect width=\"{side}\" height=\"{side}\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#000\"/></svg>"
    )
}

pub fn qr_code(code: &[Vec<bool>], quiet: usize, size: f32) -> Svg {
    svg(ImageSrc::svg_markup(&markup(code, quiet))).w(px(size)).h(px(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_modules_become_unit_squares_inside_the_quiet_zone() {
        let svg = markup(&[vec![true, false], vec![false, true]], 3);
        assert!(svg.contains("viewBox=\"0 0 8 8\""));
        assert!(svg.contains("M3 3h1v1h-1zM4 4h1v1h-1z"));
        assert!(!svg.contains("M4 3"));
    }
}
