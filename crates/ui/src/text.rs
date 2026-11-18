pub use browser_core::text::{bounded_origin, bounded_title, is_bidi};
use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, ShapedLine, SharedString, Size, Style,
    Styled, Window,
};

/// Fills the element with one line of `text`, cut with an ellipsis where it does not fit.
pub trait OneLine: Styled + ParentElement + Sized {
    fn one_line(self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        let text = if text.contains(['\n', '\r']) { text.replace(['\n', '\r'], " ").into() } else { text };
        self.min_w_0().overflow_hidden().child(Line(text))
    }
}

impl<E: Styled + ParentElement> OneLine for E {}

/// GPUI 0.2.2 paints a text from its last measurement, which in grids and flex columns is often
/// a probe at another width, so `text_ellipsis()` cuts mid-letter there; this line is cut at the
/// width it is painted at.
struct Line(SharedString);

impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Line {
    type RequestLayoutState = ();
    type PrepaintState = Option<ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, _: &mut App) -> (LayoutId, ()) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let width = window.text_system().shape_line(self.0.clone(), font_size, &[style.to_run(self.0.len())], None).width;
        let layout =
            window.request_measured_layout(Style::default(), move |known, _, _, _| Size { width: known.width.unwrap_or(width.ceil()), height: line_height });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ShapedLine> {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut runs = vec![style.to_run(self.0.len())];
        let whole = window.text_system().shape_line(self.0.clone(), font_size, &runs, None);
        // The wrapper sums unkerned advances, so it would cut a line whose shaped width fits.
        if whole.width <= bounds.size.width {
            return Some(whole);
        }
        let text = cx.text_system().line_wrapper(style.font(), font_size).truncate_line(self.0.clone(), bounds.size.width, "…", &mut runs);
        Some(window.text_system().shape_line(text, font_size, &runs, None))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        line: &mut Option<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(line) = line {
            line.paint(bounds.origin, window.line_height(), window, cx).ok();
        }
    }
}

/// A URL shown in full up to `MAX_URL_CHARS`; a longer one keeps its start and its end, where an
/// app link's risky arguments often sit, with an ellipsis between.
pub fn bounded_url(raw: &str) -> String {
    const MAX_URL_CHARS: usize = 200;
    let clean: Vec<char> = raw.chars().filter(|c| !c.is_control() && !is_bidi(*c)).collect();
    if clean.len() <= MAX_URL_CHARS {
        return clean.into_iter().collect();
    }
    let half = (MAX_URL_CHARS - 1) / 2;
    clean[..half].iter().chain(std::iter::once(&'…')).chain(&clean[clean.len() - half..]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_urls_keep_their_start_and_end() {
        let url = format!("ms-officecmd:{}&tail=run", "a".repeat(400));
        let shown = bounded_url(&url);
        assert!(shown.starts_with("ms-officecmd:") && shown.ends_with("&tail=run") && shown.contains('…'));
        assert!(shown.chars().count() <= 200);
        assert_eq!(bounded_url("zoommtg://x\u{202E}y"), "zoommtg://xy");
    }

    #[test]
    fn strips_controls_bidi_and_bounds_length() {
        assert_eq!(bounded_title("  Hello\n\t world\u{202E}gpj.exe "), "Hello worldgpj.exe");
        assert_eq!(bounded_title(""), "");
        let long = bounded_title(&"a".repeat(500));
        assert_eq!(long.chars().count(), browser_core::text::MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
    }
}
