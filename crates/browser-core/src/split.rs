//! Split view: up to four tabs of a space shown at once.

pub const MAX_PANES: usize = 4;
/// The narrowest a column gets, as a share of the split's width.
pub const MIN_RATIO: f32 = 0.2;

/// x, y, width, height as shares of the split's size.
pub type Share = (f32, f32, f32, f32);

/// Where each of `count` panes goes: two side by side, three with the right column stacked, four
/// in a grid. `ratio` is the left column's share of the width.
pub fn layout(count: usize, ratio: f32) -> Vec<Share> {
    let left = ratio.clamp(MIN_RATIO, 1. - MIN_RATIO);
    let right = 1. - left;
    match count {
        0 => Vec::new(),
        1 => vec![(0., 0., 1., 1.)],
        2 => vec![(0., 0., left, 1.), (left, 0., right, 1.)],
        3 => vec![(0., 0., left, 1.), (left, 0., right, 0.5), (left, 0.5, right, 0.5)],
        _ => vec![(0., 0., left, 0.5), (left, 0., right, 0.5), (0., 0.5, left, 0.5), (left, 0.5, right, 0.5)],
    }
}

/// The left column's share for a divider dragged to `x` in a split `width` wide.
pub fn ratio_at(x: f32, width: f32) -> f32 {
    (x / width.max(1.)).clamp(MIN_RATIO, 1. - MIN_RATIO)
}

/// The panes with `key` added at the end, unless it is there already or the split is full.
pub fn with_pane<K: PartialEq + Copy>(panes: &[K], key: K) -> Option<Vec<K>> {
    (!panes.contains(&key) && panes.len() < MAX_PANES).then(|| panes.iter().copied().chain([key]).collect())
}

/// Whether a tab dragged onto another tab's row at `share` of its height (0 at the top) joins it in
/// a split; near the edges the drop moves the tab instead.
pub fn drops_into(share: f32) -> bool {
    (0.25..0.75).contains(&share)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_fill_the_split_without_overlap() {
        for count in 1..=MAX_PANES {
            let rects = layout(count, 0.4);
            assert_eq!(rects.len(), count);
            let area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
            assert!((area - 1.).abs() < 1e-5, "{count} panes cover the split");
        }
        assert_eq!(layout(2, 0.5), [(0., 0., 0.5, 1.), (0.5, 0., 0.5, 1.)]);
        assert_eq!(layout(3, 0.5)[2], (0.5, 0.5, 0.5, 0.5));
        assert_eq!(layout(4, 0.5)[2], (0., 0.5, 0.5, 0.5));
        assert_eq!(layout(2, 0.05)[0].2, MIN_RATIO, "the divider stops short of the edge");
    }

    #[test]
    fn divider_and_drops_follow_the_pointer() {
        assert_eq!(ratio_at(300., 1000.), 0.3);
        assert_eq!(ratio_at(10., 1000.), MIN_RATIO);
        assert_eq!(ratio_at(0., 0.), MIN_RATIO);
        assert!(drops_into(0.5) && !drops_into(0.1) && !drops_into(0.9));
    }

    #[test]
    fn a_split_takes_four_distinct_tabs_at_most() {
        assert_eq!(with_pane(&[1, 2], 3), Some(vec![1, 2, 3]));
        assert_eq!(with_pane(&[1, 2], 2), None);
        assert_eq!(with_pane(&[1, 2, 3, 4], 5), None);
    }
}
