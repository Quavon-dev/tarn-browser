//! The ASCII cloud bank behind the start card. Shells draw each row in a monospace font and
//! colour its ranges with their theme's `fog` shades.

use std::ops::Range;

const RAMP: &[u8] = b" .,:;i+*?%S";
/// Shades above zero; 0 is blank space.
pub const SHADES: u8 = 4;
/// Seconds per frame while the start page animates.
pub const FRAME: f32 = 0.12;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: String,
    /// Byte ranges of `text` and their shade, `1..=SHADES`; spaces are left out.
    pub shades: Vec<(Range<usize>, u8)>,
}

fn hash(x: i32, y: i32) -> f32 {
    let h = (x.wrapping_mul(374_761_393)).wrapping_add(y.wrapping_mul(668_265_263)) as u32;
    let h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

fn noise(x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (xf, yf) = (x - xi, y - yi);
    let (u, v) = (xf * xf * (3. - 2. * xf), yf * yf * (3. - 2. * yf));
    let (xi, yi) = (xi as i32, yi as i32);
    let (a, b, c, d) = (hash(xi, yi), hash(xi + 1, yi), hash(xi, yi + 1), hash(xi + 1, yi + 1));
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

fn fbm(x: f32, y: f32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0., 0.5, 1., 0.);
    for _ in 0..5 {
        sum += amp * noise(x * freq, y * freq);
        norm += amp;
        freq *= 2.;
        amp *= 0.5;
    }
    sum / norm
}

/// Brightness `0..=1` of the cell at column `c`, row `r` of a `cols` × `rows` grid at `t`
/// seconds: two cloud banks, thickest at the sides, thinned around the centre so the start card
/// stays readable.
fn intensity(c: usize, r: usize, cols: usize, rows: usize, t: f32) -> f32 {
    let (x, y) = (c as f32 / cols.max(1) as f32, r as f32 / rows.max(1) as f32);
    let low = (-((y - 0.68) / 0.17).powi(2)).exp() * (0.45 + 0.9 * (x - 0.5).abs());
    let high = (-((y - 0.2) / 0.09).powi(2)).exp() * 0.35;
    let n = fbm(c as f32 * 0.028 + t * 0.04, r as f32 * 0.075 - t * 0.01);
    let centre = (((x - 0.5) / 0.3).powi(2) + ((y - 0.42) / 0.22).powi(2)).sqrt();
    ((n - 0.39).max(0.) * 3.6 * (low + high) * (0.25 + 0.75 * centre.min(1.))).min(1.)
}

fn shade(v: f32) -> u8 {
    match v {
        v if v < 0.1 => 0,
        v if v < 0.28 => 1,
        v if v < 0.45 => 2,
        v if v < 0.7 => 4,
        _ => 3,
    }
}

/// The frame shown `elapsed` after the fog appeared.
pub fn step(elapsed: std::time::Duration) -> u32 {
    (elapsed.as_secs_f32() / FRAME) as u32
}

/// Frame `step` of a `cols` × `rows` grid.
pub fn frame(cols: usize, rows: usize, step: u32) -> Vec<Row> {
    let t = step as f32 * FRAME;
    (0..rows)
        .map(|r| {
            let mut row = Row { text: String::with_capacity(cols), shades: Vec::new() };
            for c in 0..cols {
                let v = intensity(c, r, cols, rows, t);
                let s = shade(v);
                let start = row.text.len();
                row.text.push(if s == 0 { ' ' } else { RAMP[((v * RAMP.len() as f32) as usize).clamp(1, RAMP.len() - 1)] as char });
                match row.shades.last_mut() {
                    _ if s == 0 => {}
                    Some((range, last)) if *last == s && range.end == start => range.end = start + 1,
                    _ => row.shades.push((start..start + 1, s)),
                }
            }
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_fills_the_grid_and_marks_only_drawn_cells() {
        let rows = frame(120, 40, 25);
        assert_eq!(rows.len(), 40);
        assert!(rows.iter().all(|r| r.text.len() == 120 && r.text.is_ascii()));
        let mut drawn = 0;
        for row in &rows {
            let mut last = 0;
            for (range, s) in &row.shades {
                assert!(range.start >= last && (1..=SHADES).contains(s), "ranges are ordered and shaded");
                assert!(row.text[range.clone()].bytes().all(|b| b != b' '));
                drawn += range.len();
                last = range.end;
            }
            assert_eq!(row.text.bytes().filter(|b| *b != b' ').count(), row.shades.iter().map(|(r, _)| r.len()).sum::<usize>());
        }
        assert!(drawn > 120 * 40 / 10, "clouds cover part of the grid");
        assert!(drawn < 120 * 40 * 3 / 4, "and leave sky");
    }

    #[test]
    fn frames_are_stable_and_move_over_time() {
        assert_eq!(frame(80, 24, 8), frame(80, 24, 8));
        assert_ne!(frame(80, 24, 8), frame(80, 24, 75));
        assert!(frame(0, 0, 0).is_empty());
        assert!(frame(5, 1, 0)[0].text.len() == 5);
        assert_eq!(step(std::time::Duration::from_millis(250)), 2);
    }

    #[test]
    fn the_centre_stays_thinner_than_the_sides() {
        let mean = |cols: Range<usize>| {
            let cells = (0..20).flat_map(|t| {
                let cols = cols.clone();
                (15..28).flat_map(move |r| cols.clone().map(move |c| intensity(c, r, 160, 50, t as f32 * 7.)))
            });
            let (sum, n) = cells.fold((0., 0.), |(s, n), v| (s + v, n + 1.));
            sum / n
        };
        assert!(mean(64..96) * 2. < mean(0..32) + mean(128..160));
    }
}
