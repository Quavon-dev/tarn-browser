//! The developer Measure tool: rulers and a dragged box drawn by the shell over a still snapshot of
//! the page, so nothing runs on the page. Shells offer it only where they offer Web Inspector
//! (`devkit::SETTING` or a Developer profile).

/// What every client says while Measure is open.
pub const HINT: &str = "Drag to measure. Click or Esc closes.";
/// Ruler ticks every `MINOR` CSS pixels, labelled every `MAJOR`.
pub const MINOR: f64 = 10.;
pub const MAJOR: f64 = 100.;
/// The rulers' thickness, in the shell's points.
pub const RULER: f64 = 28.;

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    /// Distance from the ruler's start, in the shell's points.
    pub at: f64,
    /// The CSS pixel it marks, on major ticks.
    pub label: Option<String>,
}

/// The ticks of a ruler `length` points long over a page shown at `zoom`, so the numbers are the
/// page's CSS pixels.
pub fn ticks(length: f64, zoom: f64) -> Vec<Tick> {
    let zoom = if zoom.is_finite() && zoom > 0. { zoom } else { 1. };
    if !length.is_finite() || length <= 0. {
        return Vec::new();
    }
    let count = (length / zoom / MINOR).floor().min(10_000.) as u32;
    (0..=count)
        .map(|n| {
            let css = f64::from(n) * MINOR;
            Tick { at: css * zoom, label: (n % (MAJOR / MINOR) as u32 == 0).then(|| format!("{css:.0}")) }
        })
        .collect()
}

/// The dragged box in CSS pixels: `[x, y, width, height]` from a drag in the shell's points over a
/// page of `page` points shown at `zoom`.
pub fn css_box(from: (f64, f64), to: (f64, f64), page: (f64, f64), zoom: f64) -> [f64; 4] {
    let zoom = if zoom.is_finite() && zoom > 0. { zoom } else { 1. };
    crate::capture::drag_rect(from, to, page).map(|v| v / zoom)
}

/// Whether a press released at `to` was a click rather than a drag, in CSS pixels; a click closes Measure.
pub fn is_click(from: (f64, f64), to: (f64, f64)) -> bool {
    (to.0 - from.0).abs() < crate::capture::MIN_SIDE && (to.1 - from.1).abs() < crate::capture::MIN_SIDE
}

/// Where the pointer rests, in whole CSS pixels.
pub fn point_label((x, y): (f64, f64)) -> String {
    format!("{:.0}, {:.0}", x.max(0.), y.max(0.))
}

/// The box's label: its size and position in whole CSS pixels.
pub fn label(css: [f64; 4]) -> String {
    let [x, y, w, h] = css.map(f64::round);
    format!("{w:.0} \u{d7} {h:.0} px  at {x:.0}, {y:.0}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rulers_count_css_pixels_at_any_zoom() {
        let plain = ticks(250., 1.);
        assert_eq!(plain.len(), 26);
        assert_eq!(plain[0], Tick { at: 0., label: Some("0".into()) });
        assert_eq!(plain[10], Tick { at: 100., label: Some("100".into()) });
        assert_eq!(plain[5].label, None);
        let zoomed = ticks(250., 2.);
        assert_eq!(zoomed.len(), 13);
        assert_eq!(zoomed[10], Tick { at: 200., label: Some("100".into()) });
        assert!(ticks(0., 1.).is_empty() && ticks(f64::NAN, 1.).is_empty());
        assert_eq!(ticks(100., 0.).len(), 11, "a broken zoom counts as 1");
        assert!(ticks(1e12, 1.).len() <= 10_001);
    }

    #[test]
    fn the_box_is_in_css_pixels_and_cut_to_the_page() {
        assert_eq!(css_box((10., 20.), (110., 70.), (800., 600.), 1.), [10., 20., 100., 50.]);
        assert_eq!(css_box((110., 70.), (10., 20.), (800., 600.), 2.), [5., 10., 50., 25.]);
        assert_eq!(css_box((700., 500.), (900., 900.), (800., 600.), 1.), [700., 500., 100., 100.]);
        assert_eq!(label([10.4, 20., 99.6, 50.]), "100 \u{d7} 50 px  at 10, 20");
    }

    #[test]
    fn a_click_closes_but_a_thin_box_is_measured() {
        assert!(is_click((10., 10.), (11., 12.)));
        assert!(!is_click((10., 10.), (210., 11.)), "a one-pixel line is still a measure");
        assert!(!is_click((10., 10.), (10., 300.)));
        assert_eq!(point_label((12.4, 7.6)), "12, 8");
        assert_eq!(point_label((-3., 0.)), "0, 0");
    }
}
