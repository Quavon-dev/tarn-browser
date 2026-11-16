use nsh_core::prelude::*;

use crate::theme::{TEXT_SMALL, Theme};

pub struct Words<'a> {
    pub x: u32,
    pub y: u32,
    pub anchor: &'a str,
    pub size: f32,
    pub weight: u32,
    pub fill: &'a str,
    pub opacity: f32,
    pub text: &'a str,
}

pub fn colour(fill: &str, opacity: f32) -> Rgba {
    let hex = fill.strip_prefix('#').and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0xffffff);
    Rgba { a: opacity, ..rgb(hex) }
}

pub fn left_and_width(x: u32, anchor: &str, width: f32) -> (f32, f32) {
    let x = x as f32;
    match anchor {
        "middle" => {
            let half = x.min(width - x);
            (x - half, half * 2.)
        }
        "end" => (0., x),
        _ => (x, width - x),
    }
}

pub const WEEK_HEIGHT: f32 = 22.;

pub fn bar_height(n: i64, busiest: i64) -> f32 {
    if n <= 0 { 0. } else { (WEEK_HEIGHT * n as f32 / busiest.max(1) as f32).max(3.) }
}

pub fn week_bars(days: &[i64]) -> View {
    let busiest = days.iter().copied().max().unwrap_or(0);
    let bars: Vec<AnyElement> = days
        .iter()
        .map(|n| {
            view()
                .w(px(9.))
                .h(px(WEEK_HEIGHT))
                .justify_end()
                .rounded(px(2.))
                .bg(Theme::separator())
                .overflow_hidden()
                .child(view().w_full().h(px(bar_height(*n, busiest))).bg(Theme::accent()))
                .into_element()
        })
        .collect();
    view().flex_row().items_end().gap_1().h(px(WEEK_HEIGHT)).accessibility_elements_hidden(true).children(bars)
}

pub const CHART_HEIGHT: f32 = 96.;

pub fn scaled_height(n: i64, busiest: i64, height: f32) -> f32 {
    if n <= 0 { 0. } else { (height * n as f32 / busiest.max(1) as f32).max(3.) }
}

pub fn day_chart(days: &[(&str, i64)]) -> View {
    let busiest = days.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let bars: Vec<AnyElement> = days
        .iter()
        .map(|(day, n)| {
            let top = *n > 0 && *n == busiest;
            view()
                .flex_1()
                .items_center()
                .justify_end()
                .gap_1()
                .child(text(thousands_short(*n)).text_size(px(TEXT_SMALL)).text_color(Theme::faint))
                .child(
                    view()
                        .w(px(22.))
                        .h(px(scaled_height(*n, busiest, CHART_HEIGHT)))
                        .rounded(px(4.))
                        .bg(move || if top { Theme::space_label() } else { Theme::accent() }),
                )
                .child(text((*day).to_owned()).text_size(px(TEXT_SMALL)).text_color(Theme::muted))
                .into_element()
        })
        .collect();
    view().flex_row().items_end().gap_1().h(px(CHART_HEIGHT + 40.)).accessibility_elements_hidden(true).children(bars)
}

pub fn share_bar(n: i64, most: i64) -> View {
    let share = if most <= 0 { 0. } else { (n.max(0) as f32 / most as f32).clamp(0., 1.) };
    view().h(px(4.)).rounded_full().bg(Theme::separator).overflow_hidden().child(view().h_full().w(relative(share)).rounded_full().bg(Theme::accent))
}

fn thousands_short(n: i64) -> String {
    if n <= 0 { String::new() } else { browser_core::shield_card::thousands(n as u64) }
}

pub fn picture(markup: &str, width: f32, height: f32, words: &[Words], scale: f32) -> View {
    let labels: Vec<AnyElement> = words
        .iter()
        .map(|w| {
            let (left, span) = left_and_width(w.x, w.anchor, width);
            let line = text(w.text.to_owned())
                .text_size(px(w.size * scale))
                .font_weight(FontWeight(u16::try_from(w.weight).unwrap_or(400)))
                .text_color(colour(w.fill, w.opacity))
                .number_of_lines(1);
            let line = match w.anchor {
                "middle" => line.text_center(),
                "end" => line.text_right(),
                _ => line,
            };
            view().absolute().left(px(left * scale)).top(px((w.y as f32 - w.size * 0.95) * scale)).w(px(span * scale)).child(line).into_element()
        })
        .collect();
    view()
        .relative()
        .w(px(width * scale))
        .h(px(height * scale))
        .overflow_hidden()
        .accessibility_elements_hidden(true)
        .child(crate::icons::picture(markup, width * scale, height * scale))
        .children(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_the_drawing_anchor_and_colour() {
        assert_eq!(left_and_width(20, "start", 360.), (20., 340.));
        assert_eq!(left_and_width(340, "end", 360.), (0., 340.));
        assert_eq!(left_and_width(180, "middle", 360.), (0., 360.));
        assert_eq!(left_and_width(50, "middle", 360.), (0., 100.));
        assert_eq!(colour("#FF0000", 0.5), Rgba { a: 0.5, ..rgb(0xff0000) });
        assert_eq!(colour("white", 1.), rgb(0xffffff));
        assert_eq!((bar_height(0, 5), bar_height(5, 5), bar_height(1, 1000)), (0., WEEK_HEIGHT, 3.));
        assert_eq!((scaled_height(0, 9, 90.), scaled_height(9, 9, 90.), scaled_height(3, 9, 90.), scaled_height(1, 1000, 90.)), (0., 90., 30., 3.));
    }
}
