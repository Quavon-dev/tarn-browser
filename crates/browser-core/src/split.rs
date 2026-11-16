//! Split view: two to four tabs of a space shown at once, and every split of a window.

pub const MAX_PANES: usize = 4;
/// The narrowest a column and the lowest a row get, as a share of the split's size.
pub const MIN_RATIO: f32 = 0.2;

/// x, y, width, height as shares of the split's size.
pub type Share = (f32, f32, f32, f32);
/// x, y, width, height in window points.
pub type Rect = (f32, f32, f32, f32);

/// The line between the two columns, or between the two rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Column,
    Row,
}

fn clamp(share: f32) -> f32 {
    share.clamp(MIN_RATIO, 1. - MIN_RATIO)
}

fn transposed((x, y, w, h): Share) -> Share {
    (y, x, h, w)
}

/// The main and cross shares, and the main and cross dividers, before a stacked layout is transposed.
fn axes(ratio: f32, row: f32, stacked: bool) -> (f32, f32, Divider, Divider) {
    if stacked { (clamp(row), clamp(ratio), Divider::Row, Divider::Column) } else { (clamp(ratio), clamp(row), Divider::Column, Divider::Row) }
}

/// Where each of `count` panes goes: two side by side, three with the right column stacked, four
/// in a grid. `ratio` is the first column's share of the width and `row` the first row's share of
/// the height. `stacked` transposes that: two panes one above the other, three with one on top.
pub fn layout(count: usize, ratio: f32, row: f32, stacked: bool) -> Vec<Share> {
    let (a, b, _, _) = axes(ratio, row, stacked);
    let (c, d) = (1. - a, 1. - b);
    let cells = match count {
        0 => Vec::new(),
        1 => vec![(0., 0., 1., 1.)],
        2 => vec![(0., 0., a, 1.), (a, 0., c, 1.)],
        3 => vec![(0., 0., a, 1.), (a, 0., c, b), (a, b, c, d)],
        _ => vec![(0., 0., a, b), (a, 0., c, b), (0., b, a, d), (a, b, c, d)],
    };
    if stacked { cells.into_iter().map(transposed).collect() } else { cells }
}

/// The dividers of that layout as lines: a column divider has no width, a row divider no height.
/// With three panes the second one runs only between the two panes that share a column or row.
pub fn dividers(count: usize, ratio: f32, row: f32, stacked: bool) -> Vec<(Divider, Share)> {
    let (a, b, main, cross) = axes(ratio, row, stacked);
    let lines = match count {
        0 | 1 => Vec::new(),
        2 => vec![(main, (a, 0., 0., 1.))],
        3 => vec![(main, (a, 0., 0., 1.)), (cross, (a, b, 1. - a, 0.))],
        _ => vec![(main, (a, 0., 0., 1.)), (cross, (0., b, 1., 0.))],
    };
    if stacked { lines.into_iter().map(|(divider, line)| (divider, transposed(line))).collect() } else { lines }
}

/// The first column's or row's share for a divider dragged to `pos` in a split `extent` wide or high.
pub fn ratio_at(pos: f32, extent: f32) -> f32 {
    clamp(pos / extent.max(1.))
}

/// The pane whose page is under the pointer.
pub fn pane_at(rects: &[Rect], x: f32, y: f32) -> Option<usize> {
    rects.iter().position(|(rx, ry, w, h)| x >= *rx && x < rx + w && y >= *ry && y < ry + h)
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

/// Two to four tabs shown at once, in layout order.
#[derive(Debug, Clone, PartialEq)]
pub struct Split<K> {
    panes: Vec<K>,
    ratio: f32,
    row: f32,
    stacked: bool,
}

impl<K: PartialEq + Copy> Split<K> {
    pub fn panes(&self) -> &[K] {
        &self.panes
    }

    /// The other panes of the split that holds `key`.
    pub fn others(&self, key: K) -> Vec<K> {
        if self.panes.contains(&key) { self.panes.iter().copied().filter(|k| *k != key).collect() } else { Vec::new() }
    }

    /// The pane after `key`, or the one before it, wrapping around.
    pub fn next(&self, key: K, forward: bool) -> Option<K> {
        let (ix, len) = (self.panes.iter().position(|k| *k == key)?, self.panes.len());
        Some(self.panes[(ix + if forward { 1 } else { len - 1 }) % len])
    }

    pub fn layout(&self) -> Vec<Share> {
        layout(self.panes.len(), self.ratio, self.row, self.stacked)
    }

    pub fn dividers(&self) -> Vec<(Divider, Share)> {
        dividers(self.panes.len(), self.ratio, self.row, self.stacked)
    }
}

/// The first entry of a tab's Split View menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Open,
    Add,
}

impl Entry {
    pub fn label(self) -> &'static str {
        match self {
            Entry::Open => "Open in Split View",
            Entry::Add => "Add to Split View",
        }
    }
}

/// A tab's Split View menu: its first entry and whether that can run, and whether the tab is in a
/// split, which Remove, Rotate and Close need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Menu {
    pub entry: Entry,
    pub enabled: bool,
    pub member: bool,
}

/// Every split of a window. A tab is in one split at most, and the split on screen is the active
/// tab's; the others wait until one of their tabs is selected.
#[derive(Debug, Clone, PartialEq)]
pub struct Splits<K>(Vec<Split<K>>);

impl<K> Default for Splits<K> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: PartialEq + Copy> Splits<K> {
    pub fn of(&self, key: K) -> Option<&Split<K>> {
        self.0.iter().find(|s| s.panes.contains(&key))
    }

    fn of_mut(&mut self, key: K) -> Option<&mut Split<K>> {
        self.0.iter_mut().find(|s| s.panes.contains(&key))
    }

    pub fn contains(&self, key: K) -> bool {
        self.of(key).is_some()
    }

    /// The other panes of `key`'s split, once those `keep` refuses, as a gone tab or one shown in
    /// another window, have left it.
    pub fn partners(&mut self, key: K, mut keep: impl FnMut(K) -> bool) -> Vec<K> {
        for pane in self.of(key).map(|s| s.others(key)).unwrap_or_default() {
            if !keep(pane) {
                self.leave(pane);
            }
        }
        self.of(key).map(|s| s.others(key)).unwrap_or_default()
    }

    /// The tab `active` splits with: the most recently used of `tabs`, with their last use, that
    /// may join it.
    pub fn partner(&self, active: K, tabs: impl IntoIterator<Item = (K, i64)>) -> Option<K> {
        tabs.into_iter().filter(|(k, _)| self.can_join(active, *k)).max_by_key(|(_, used)| *used).map(|(k, _)| k)
    }

    /// Whether `key`, a tab in no split, may join `anchor`: in its split while that has room, else
    /// the two start one. A full split is never replaced.
    pub fn can_join(&self, anchor: K, key: K) -> bool {
        anchor != key && !self.contains(key) && self.of(anchor).is_none_or(|s| with_pane(&s.panes, key).is_some())
    }

    pub fn join(&mut self, anchor: K, key: K) -> bool {
        if !self.can_join(anchor, key) {
            return false;
        }
        match self.of_mut(anchor) {
            Some(split) => split.panes.push(key),
            None => self.0.push(Split { panes: vec![anchor, key], ratio: 0.5, row: 0.5, stacked: false }),
        }
        true
    }

    /// What dropping `dragged` on the middle of `target`'s row or on its pane does, if anything.
    pub fn drop_label(&self, target: K, dragged: K) -> Option<&'static str> {
        if dragged != target && self.of(target).is_some_and(|s| s.panes.contains(&dragged)) {
            return Some("Swap");
        }
        self.can_join(target, dragged).then(|| if self.contains(target) { "Add to Split" } else { "Split" })
    }

    /// The pane that takes the focus when `key` and the rest of `closing` close: the next one that
    /// stays, else the one before, also when it is the last one and the split ends.
    pub fn heir(&self, key: K, closing: &[K]) -> Option<K> {
        let panes = self.of(key)?.panes();
        let ix = panes.iter().position(|k| *k == key)?;
        let stays = |k: &&K| **k != key && !closing.contains(k);
        panes[ix..].iter().find(stays).or_else(|| panes[..ix].iter().rev().find(stays)).copied()
    }

    /// Takes `key` out of its split, which ends when one pane is left. The pane that takes the
    /// focus in a split that stays; when the split ends, `key` keeps it.
    pub fn leave(&mut self, key: K) -> Option<K> {
        let heir = self.heir(key, &[]).filter(|_| self.of(key).is_some_and(|s| s.panes.len() > 2));
        self.retain(|k| k != key);
        heir
    }

    /// Drops the panes `keep` refuses, as when their tabs are gone.
    pub fn retain(&mut self, keep: impl Fn(K) -> bool) {
        for split in &mut self.0 {
            split.panes.retain(|k| keep(*k));
        }
        self.0.retain(|s| s.panes.len() >= 2);
    }

    /// Ends `key`'s split; its tabs stay open.
    pub fn close(&mut self, key: K) -> bool {
        let before = self.0.len();
        self.0.retain(|s| !s.panes.contains(&key));
        self.0.len() != before
    }

    /// Exchanges two panes of one split.
    pub fn swap(&mut self, a: K, b: K) -> bool {
        let Some(split) = self.of_mut(a) else { return false };
        let at = |key: K| split.panes.iter().position(|k| *k == key);
        match (at(a), at(b)) {
            (Some(i), Some(j)) if i != j => split.panes.swap(i, j),
            _ => return false,
        }
        true
    }

    /// Turns `key`'s split between side by side and stacked.
    pub fn rotate(&mut self, key: K) -> bool {
        self.of_mut(key).map(|s| s.stacked = !s.stacked).is_some()
    }

    /// Moves a divider of `key`'s split to `share`; false when it stays where it is.
    pub fn resize(&mut self, key: K, divider: Divider, share: f32) -> bool {
        let Some(split) = self.of_mut(key) else { return false };
        let (slot, share) = (if divider == Divider::Column { &mut split.ratio } else { &mut split.row }, clamp(share));
        let moved = *slot != share;
        *slot = share;
        moved
    }

    /// Gives the columns and the rows of `key`'s split the same size again.
    pub fn equalise(&mut self, key: K) -> bool {
        let column = self.resize(key, Divider::Column, 0.5);
        self.resize(key, Divider::Row, 0.5) || column
    }

    /// The Split View menu of `key` while `active` is the active tab.
    pub fn menu(&self, active: Option<K>, key: K) -> Menu {
        let member = self.contains(key);
        Menu {
            entry: if active.is_some_and(|a| a != key && self.contains(a)) { Entry::Add } else { Entry::Open },
            enabled: active.is_some_and(|a| if a == key { !member } else { self.can_join(a, key) }),
            member,
        }
    }

    /// What a tab list in `keys` order shows at `key`'s place: its split's panes that are in
    /// `keys`, in layout order, at the place of the first of them, and nothing at the others'
    /// places. A tab in no split shows alone.
    pub fn entry(&self, key: K, keys: &[K]) -> Option<Vec<K>> {
        let Some(split) = self.of(key) else { return Some(vec![key]) };
        let panes: Vec<K> = split.panes.iter().copied().filter(|p| keys.contains(p)).collect();
        (keys.iter().find(|k| panes.contains(k)) == Some(&key)).then_some(panes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::TabKey;

    fn splits(sets: &[&[u8]]) -> Splits<u8> {
        Splits(sets.iter().map(|panes| Split { panes: panes.to_vec(), ratio: 0.5, row: 0.5, stacked: false }).collect())
    }

    fn panes(splits: &Splits<u8>) -> Vec<&[u8]> {
        splits.0.iter().map(Split::panes).collect()
    }

    #[test]
    fn a_split_lists_as_one_entry_at_its_first_pane() {
        let s = splits(&[&[4, 2]]);
        let keys = [1, 2, 3, 4];
        let entries: Vec<Vec<u8>> = keys.iter().filter_map(|k| s.entry(*k, &keys)).collect();
        assert_eq!(entries, vec![vec![1], vec![4, 2], vec![3]]);
        assert_eq!(s.entry(2, &[2, 9]), Some(vec![2]));
        assert_eq!(s.entry(4, &[1]), None);
    }

    #[test]
    fn panes_fill_the_split_without_overlap() {
        for count in 1..=MAX_PANES {
            let rects = layout(count, 0.4, 0.5, false);
            assert_eq!(rects.len(), count);
            let area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
            assert!((area - 1.).abs() < 1e-5, "{count} panes cover the split");
        }
        assert_eq!(layout(2, 0.5, 0.5, false), [(0., 0., 0.5, 1.), (0.5, 0., 0.5, 1.)]);
        assert_eq!(layout(3, 0.5, 0.5, false)[2], (0.5, 0.5, 0.5, 0.5));
        assert_eq!(layout(4, 0.5, 0.5, false)[2], (0., 0.5, 0.5, 0.5));
        assert_eq!(layout(2, 0.05, 0.5, false)[0].2, MIN_RATIO, "the divider stops short of the edge");
    }

    #[test]
    fn rows_resize_and_a_stacked_split_is_the_layout_transposed() {
        for (count, stacked) in (1..=MAX_PANES).flat_map(|count| [(count, false), (count, true)]) {
            let rects = layout(count, 0.3, 0.7, stacked);
            let area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
            assert!((area - 1.).abs() < 1e-5, "{count} panes cover the split, stacked: {stacked}");
        }
        assert_eq!(layout(3, 0.5, 0.25, false), [(0., 0., 0.5, 1.), (0.5, 0., 0.5, 0.25), (0.5, 0.25, 0.5, 0.75)]);
        assert_eq!(layout(4, 0.5, 0.25, false)[3], (0.5, 0.25, 0.5, 0.75));
        assert_eq!(layout(2, 0.5, 0.25, true), [(0., 0., 1., 0.25), (0., 0.25, 1., 0.75)], "two stacked panes share the height by `row`");
        assert_eq!(layout(3, 0.25, 0.5, true), [(0., 0., 1., 0.5), (0., 0.5, 0.25, 0.5), (0.25, 0.5, 0.75, 0.5)], "one on top, two below");
        assert_eq!(layout(4, 0.25, 0.5, true)[1], (0., 0.5, 0.25, 0.5));
        assert_eq!(layout(3, 0.5, 0.95, false)[1].3, 1. - MIN_RATIO, "the row divider stops short of the edge");
    }

    #[test]
    fn dividers_lie_between_the_panes() {
        assert!(dividers(1, 0.5, 0.5, false).is_empty());
        assert_eq!(dividers(2, 0.25, 0.5, false), [(Divider::Column, (0.25, 0., 0., 1.))]);
        assert_eq!(dividers(2, 0.5, 0.25, true), [(Divider::Row, (0., 0.25, 1., 0.))]);
        assert_eq!(dividers(3, 0.25, 0.5, false), [(Divider::Column, (0.25, 0., 0., 1.)), (Divider::Row, (0.25, 0.5, 0.75, 0.))]);
        assert_eq!(dividers(3, 0.25, 0.5, true), [(Divider::Row, (0., 0.5, 1., 0.)), (Divider::Column, (0.25, 0.5, 0., 0.5))]);
        for stacked in [false, true] {
            let lines = dividers(4, 0.25, 0.5, stacked);
            assert!(lines.contains(&(Divider::Column, (0.25, 0., 0., 1.))) && lines.contains(&(Divider::Row, (0., 0.5, 1., 0.))));
        }
    }

    #[test]
    fn divider_and_drops_follow_the_pointer() {
        assert_eq!(ratio_at(300., 1000.), 0.3);
        assert_eq!(ratio_at(10., 1000.), MIN_RATIO);
        assert_eq!(ratio_at(0., 0.), MIN_RATIO);
        assert!(drops_into(0.5) && !drops_into(0.1) && !drops_into(0.9));
    }

    #[test]
    fn clicks_pick_the_pane_under_the_pointer() {
        let rects = [(10., 50., 400., 600.), (418., 50., 400., 600.)];
        assert_eq!(pane_at(&rects, 10., 50.), Some(0));
        assert_eq!(pane_at(&rects, 409.9, 649.), Some(0));
        assert_eq!(pane_at(&rects, 414., 300.), None, "the gap belongs to neither pane");
        assert_eq!(pane_at(&rects, 418., 300.), Some(1));
        assert_eq!(pane_at(&rects, 500., 20.), None, "the toolbar is outside both panes");
    }

    #[test]
    fn a_split_takes_four_distinct_tabs_at_most() {
        assert_eq!(with_pane(&[1, 2], 3), Some(vec![1, 2, 3]));
        assert_eq!(with_pane(&[1, 2], 2), None);
        assert_eq!(with_pane(&[1, 2, 3, 4], 5), None);
    }

    #[test]
    fn others_are_the_rest_of_the_split() {
        let (a, b, c, d) =
            (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 }, TabKey { private: true, id: 1 }, TabKey { private: false, id: 3 });
        let split = |panes: &[TabKey]| Split { panes: panes.to_vec(), ratio: 0.5, row: 0.5, stacked: false };
        assert_eq!(split(&[a, b, d]).others(a), [b, d]);
        assert_eq!(split(&[a, b]).others(b), [a]);
        assert!(split(&[a, b]).others(c).is_empty(), "a private tab with the same id is another tab");
    }

    #[test]
    fn partners_drop_the_panes_that_may_not_stay() {
        let mut s = splits(&[&[1, 2, 3], &[4, 5]]);
        assert_eq!(s.partners(1, |k| k != 3), [2]);
        assert_eq!(panes(&s), [&[1, 2][..], &[4, 5]]);
        assert!(s.partners(4, |k| k != 5).is_empty(), "a split left with one pane ends");
        assert_eq!(panes(&s), [[1, 2]]);
        assert!(s.partners(9, |_| true).is_empty());
    }

    #[test]
    fn the_partner_is_the_most_recently_used_tab_that_may_join() {
        let s = splits(&[&[1, 2]]);
        assert_eq!(s.partner(3, [(1, 9), (3, 8), (4, 5), (5, 7)]), Some(5), "split panes and the tab itself are skipped");
        assert_eq!(s.partner(1, [(3, 1), (4, 2)]), Some(4), "a pane's split takes another tab");
        assert_eq!(s.partner(3, [(1, 9), (3, 8)]), None);
    }

    #[test]
    fn a_tab_joins_one_split_and_a_full_split_is_never_replaced() {
        let mut s = Splits::default();
        assert!(!s.can_join(1, 1) && !s.join(1, 1));
        assert!(s.join(1, 2) && s.join(2, 3) && s.join(1, 4));
        assert_eq!(panes(&s), [[1, 2, 3, 4]]);
        assert!(!s.can_join(1, 5) && !s.join(1, 5), "a fifth tab is refused");
        assert!(!s.join(5, 1), "a pane does not leave its split for another tab");
        assert!(s.join(5, 6), "another pair is a split of its own");
        assert_eq!(panes(&s), [&[1, 2, 3, 4][..], &[5, 6]]);
        assert_eq!(s.of(6).map(Split::panes), Some(&[5, 6][..]));
        assert!(s.contains(3) && !s.contains(7) && s.of(7).is_none());
    }

    #[test]
    fn a_closed_pane_hands_the_focus_to_its_neighbour() {
        let mut s = splits(&[&[1, 2, 3, 4]]);
        assert_eq!(s.heir(2, &[]), Some(3));
        assert_eq!(s.heir(4, &[]), Some(3), "the last pane hands over to the one before");
        assert_eq!(s.heir(2, &[3]), Some(4));
        assert_eq!(s.heir(2, &[3, 4]), Some(1), "the last pane left takes the focus as the split ends");
        assert_eq!(s.heir(2, &[1, 3, 4]), None);
        assert_eq!(s.heir(9, &[]), None);
        assert_eq!(s.leave(2), Some(3));
        assert_eq!(s.leave(4), Some(3));
        assert_eq!(panes(&s), [[1, 3]]);
        assert_eq!(s.leave(1), None);
        assert!(panes(&s).is_empty(), "a split left with one pane ends");
        assert_eq!(s.leave(1), None);
    }

    #[test]
    fn gone_tabs_leave_and_a_closed_split_keeps_the_others() {
        let mut s = splits(&[&[1, 2, 3], &[4, 5], &[6, 7]]);
        s.retain(|k| k != 3 && k != 5);
        assert_eq!(panes(&s), [[1, 2], [6, 7]]);
        assert!(s.close(7) && !s.close(7) && !s.close(9));
        assert_eq!(panes(&s), [[1, 2]]);
    }

    #[test]
    fn panes_swap_and_the_focus_cycles() {
        let mut s = splits(&[&[1, 2, 3], &[4, 5]]);
        assert!(s.swap(1, 3));
        assert_eq!(panes(&s)[0], [3, 2, 1]);
        assert!(!s.swap(1, 1) && !s.swap(1, 4) && !s.swap(9, 1), "only two panes of one split swap");
        let split = s.of(1).unwrap();
        assert_eq!((split.next(1, true), split.next(3, false)), (Some(3), Some(1)), "the ends wrap around");
        assert_eq!((split.next(3, true), split.next(2, false), split.next(9, true)), (Some(2), Some(3), None));
    }

    #[test]
    fn dividers_move_within_the_limits_and_reset() {
        let mut s = splits(&[&[1, 2, 3]]);
        assert!(s.resize(1, Divider::Column, 0.3) && !s.resize(2, Divider::Column, 0.3), "an unmoved divider reports no change");
        assert!(s.resize(1, Divider::Row, 0.99) && !s.resize(1, Divider::Row, 0.9) && !s.resize(9, Divider::Row, 0.5));
        assert_eq!(s.of(1).unwrap().layout(), layout(3, 0.3, 1. - MIN_RATIO, false));
        assert!(s.rotate(3) && !s.rotate(9));
        assert_eq!(s.of(1).unwrap().layout(), layout(3, 0.3, 1. - MIN_RATIO, true));
        assert_eq!(s.of(1).unwrap().dividers(), dividers(3, 0.3, 1. - MIN_RATIO, true));
        assert!(s.equalise(2) && !s.equalise(2));
        assert_eq!(s.of(1).unwrap().layout(), layout(3, 0.5, 0.5, true));
    }

    #[test]
    fn the_menu_and_the_drop_follow_membership() {
        let s = splits(&[&[1, 2, 3], &[4, 5, 6, 7]]);
        let menu = |entry, enabled, member| Menu { entry, enabled, member };
        assert_eq!(s.menu(Some(8), 8), menu(Entry::Open, true, false), "the active tab gets a partner");
        assert_eq!(s.menu(Some(8), 9), menu(Entry::Open, true, false));
        assert_eq!(s.menu(Some(1), 9), menu(Entry::Add, true, false));
        assert_eq!(s.menu(Some(4), 9), menu(Entry::Add, false, false), "a full split takes no fifth tab");
        assert_eq!(s.menu(Some(1), 1), menu(Entry::Open, false, true));
        assert_eq!(s.menu(Some(1), 2), menu(Entry::Add, false, true));
        assert_eq!(s.menu(Some(8), 2), menu(Entry::Open, false, true), "a pane of another split stays there");
        assert_eq!(s.menu(None, 9), menu(Entry::Open, false, false));
        assert_eq!((Entry::Open.label(), Entry::Add.label()), ("Open in Split View", "Add to Split View"));

        assert_eq!(s.drop_label(8, 9), Some("Split"));
        assert_eq!(s.drop_label(1, 9), Some("Add to Split"));
        assert_eq!(s.drop_label(1, 3), Some("Swap"));
        assert_eq!(s.drop_label(4, 9), None, "a full split takes no fifth tab");
        assert_eq!(s.drop_label(8, 2), None, "a pane of another split stays there");
        assert_eq!(s.drop_label(1, 1), None);
    }
}
