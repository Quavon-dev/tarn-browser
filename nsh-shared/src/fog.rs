use std::ops::Range;
use std::time::{Duration, Instant};

use nsh_core::canvas::CanvasTextBaseline;
use nsh_core::prelude::*;

use browser_core::theme::Fog;

use crate::theme::{Theme, themed};

pub const CELL: f32 = 6.02;
pub const LINE: f32 = 11.;
#[cfg(target_os = "ios")]
const FONT: &str = "10px Menlo";
#[cfg(not(target_os = "ios"))]
const FONT: &str = "10px monospace";

pub type Row = (String, Vec<(Range<usize>, u8)>);

pub fn grid(width: f32, height: f32) -> (usize, usize) {
    ((width.max(0.) / CELL) as usize, (height.max(0.) / LINE) as usize)
}

pub fn layers((text, shades): &Row) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
    for (range, shade) in shades {
        let at = match out.iter().position(|(s, _)| s == shade) {
            Some(at) => at,
            None => {
                out.push((*shade, vec![b' '; text.len()]));
                out.len() - 1
            }
        };
        out[at].1[range.clone()].copy_from_slice(&text.as_bytes()[range.clone()]);
    }
    out.into_iter().map(|(shade, bytes)| (shade, String::from_utf8_lossy(&bytes).into_owned())).collect()
}

pub fn fog(frame: impl Fn(usize, usize, Duration) -> Vec<Row> + 'static, period: Duration) -> View {
    let frame = std::rc::Rc::new(frame);
    view().absolute().inset_0().pointer_events(PointerEvents::None).child(themed(move || {
        let frame = frame.clone();
        match Theme::fog_mode() {
            Fog::Off => None,
            mode => Some(layer(move |c, r, t| frame(c, r, t), period, mode == Fog::On)),
        }
    }))
}

fn layer(frame: impl Fn(usize, usize, Duration) -> Vec<Row> + 'static, period: Duration, moving: bool) -> View {
    let size = signal((0f32, 0f32));
    let elapsed = signal(Duration::ZERO);
    let start = Instant::now();
    crate::motion::while_active(period, move || moving, move || elapsed.set(start.elapsed()));
    let painter = move || {
        let (width, height) = size.get();
        let (cols, rows) = grid(width, height);
        let x = (width - cols as f32 * CELL) / 2.;
        let y = (height - rows as f32 * LINE) / 2.;
        let mut painter = Painter::new().font(FONT).text_baseline(CanvasTextBaseline::Top);
        for (r, row) in frame(cols, rows, elapsed.get()).iter().enumerate() {
            for (shade, text) in layers(row) {
                painter = painter.fill_style(Theme::fog(shade)).fill_text(&text, x, y + r as f32 * LINE);
            }
        }
        painter
    };
    view()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .pointer_events(PointerEvents::None)
        .accessibility_elements_hidden(true)
        .on_layout(move |b: &Bounds<Points>| {
            let next = (b.size.width.0, b.size.height.0);
            if size.get_untracked() != next {
                size.set(next);
            }
        })
        .child(canvas(painter).size_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_split_a_row_by_shade_and_keep_columns() {
        let row: Row = ("ab cd".into(), vec![(0..2, 1), (3..4, 2), (4..5, 1)]);
        assert_eq!(layers(&row), vec![(1, "ab  d".to_owned()), (2, "   c ".to_owned())]);
    }

    #[test]
    fn grid_counts_whole_cells() {
        assert_eq!(grid(CELL * 10.5, LINE * 3.2), (10, 3));
        assert_eq!(grid(-5., 0.), (0, 0));
    }
}
