use std::ops::Range;
use std::time::{Duration, Instant};

use url::Url;

use crate::command::{MAX_HOST, host_tail};
use crate::tabs::TabKey;
use crate::text::bounded_title;

/// The invisible strip at the window's left edge that opens the floating sidebar.
pub const EDGE_WIDTH: f32 = 12.;
pub const OPEN_DELAY: Duration = Duration::from_millis(100);
pub const CLOSE_DELAY: Duration = Duration::from_millis(350);
/// Faster than this, in points per millisecond, the pointer is passing through, not resting.
const REST_SPEED: f32 = 0.5;
/// Room around the floating sidebar that still counts as on it.
pub const GRACE: f32 = 16.;
pub const CARD_DELAY: Duration = Duration::from_millis(600);

/// The floating sidebar's hover rules, fed with pointer moves over the window and with the
/// pointer entering or leaving the sidebar's card.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Hover {
    pub open: bool,
    /// Since when the pointer has rested in the edge strip.
    rest: Option<Instant>,
    /// Since when the pointer has been off the open card.
    left: Option<Instant>,
    last: Option<(f32, f32, Instant)>,
}

impl Hover {
    /// A pointer move at `(x, y)` in window points, which may lie outside the window; the card
    /// spans `rows` (from below the toolbar to the window's bottom) and `reach` is its right edge
    /// plus its grace margin. Returns whether a pending open or close changed.
    pub fn moved(&mut self, x: f32, y: f32, pressed: bool, reach: f32, rows: Range<f32>, now: Instant) -> bool {
        let (top, height) = (rows.start, rows.end);
        let before = (self.rest, self.left);
        let fast = self.last.is_some_and(|(lx, ly, at)| (x - lx).hypot(y - ly) > REST_SPEED * (now - at).as_secs_f32().max(0.001) * 1000.);
        self.last = Some((x, y, now));
        if self.open {
            self.card((0. ..reach).contains(&x) && (top - GRACE..height).contains(&y), now);
        } else {
            let edge = (0. ..EDGE_WIDTH).contains(&x) && (top..height).contains(&y) && !pressed;
            self.rest = if edge && !fast { self.rest.or(Some(now)) } else { edge.then_some(now) };
        }
        before != (self.rest, self.left)
    }

    /// The pointer entered or left the card.
    pub fn card(&mut self, inside: bool, now: Instant) {
        self.left = if inside { None } else { self.left.or(Some(now)) };
    }

    pub fn opened(&mut self) {
        *self = Hover { open: true, last: self.last, ..Hover::default() };
    }

    pub fn closed(&mut self) {
        *self = Hover { last: self.last, ..Hover::default() };
    }

    /// Whether to open (`true`) or close, and how much longer to wait first.
    pub fn next(&self, now: Instant) -> Option<(bool, Duration)> {
        let wait = |since: Instant, delay: Duration| delay.saturating_sub(now - since);
        match (self.open, self.rest, self.left) {
            (false, Some(since), _) => Some((true, wait(since, OPEN_DELAY))),
            (true, _, Some(since)) => Some((false, wait(since, CLOSE_DELAY))),
            _ => None,
        }
    }
}

/// Which row the pointer rests on and since when; the card shows once it has rested `CARD_DELAY`.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Rest {
    on: Option<(TabKey, Instant)>,
}

impl Rest {
    pub fn enter(&mut self, key: TabKey, now: Instant) {
        if self.on.is_none_or(|(k, _)| k != key) {
            self.on = Some((key, now));
        }
    }

    /// Leaving a row the pointer is no longer on, after it entered the next one, changes nothing.
    pub fn leave(&mut self, key: TabKey) -> bool {
        let left = self.on.is_some_and(|(k, _)| k == key);
        if left {
            self.on = None;
        }
        left
    }

    /// The row to show, or how much longer to wait for it.
    pub fn due(&self, now: Instant) -> Option<(TabKey, Duration)> {
        self.on.map(|(key, since)| (key, CARD_DELAY.saturating_sub(now - since)))
    }
}

pub fn card_lines(url: &str, sleeping: bool, muted: bool) -> (String, String) {
    let place = Url::parse(url).ok().and_then(|u| {
        let host = host_tail(u.host_str()?.trim_start_matches("www."), MAX_HOST);
        Some(format!("{host}{}", u.path().trim_end_matches('/')))
    });
    let state: Vec<&str> = [(sleeping, "Sleeping"), (muted, "Muted")].into_iter().filter(|(on, _)| *on).map(|(_, s)| s).collect();
    (bounded_title(&place.unwrap_or_else(|| url.to_owned())), state.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REACH: f32 = 300.;
    const TOP: f32 = 44.;
    const Y: f32 = 400.;
    const H: f32 = 800.;
    const A: TabKey = TabKey { private: false, id: 1 };
    const B: TabKey = TabKey { private: false, id: 2 };

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn opens_after_resting_in_the_edge_strip() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        assert!(!hover.moved(200., Y, false, REACH, TOP..H, t));
        assert_eq!(hover.next(t), None);
        assert!(hover.moved(EDGE_WIDTH - 1., Y, false, REACH, TOP..H, at(t, 400)));
        assert_eq!(hover.next(at(t, 430)), Some((true, OPEN_DELAY - Duration::from_millis(30))));
        assert!(!hover.moved(EDGE_WIDTH - 2., Y + 1., false, REACH, TOP..H, at(t, 450)), "a slow drift keeps resting");
        assert_eq!(hover.next(at(t, 500)), Some((true, Duration::ZERO)));
    }

    #[test]
    fn ignores_fast_moves_buttons_and_the_toolbar() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        hover.moved(4., Y, false, REACH, TOP..H, t);
        hover.moved(4., Y + 60., false, REACH, TOP..H, at(t, 20));
        assert_eq!(hover.next(at(t, 100)), Some((true, Duration::from_millis(20))), "a fast move restarts the wait");
        hover.moved(40., Y + 60., false, REACH, TOP..H, at(t, 30));
        assert_eq!(hover.next(at(t, 500)), None, "passing through never opens");
        hover.moved(4., Y, true, REACH, TOP..H, at(t, 600));
        assert_eq!(hover.next(at(t, 800)), None, "not while a button is down");
        hover.moved(4., TOP - 1., false, REACH, TOP..H, at(t, 900));
        assert_eq!(hover.next(at(t, 1100)), None, "not beside the toolbar");
        hover.moved(-4., Y, false, REACH, TOP..H, at(t, 1200));
        hover.moved(4., H + 10., false, REACH, TOP..H, at(t, 1300));
        assert_eq!(hover.next(at(t, 1500)), None, "not from outside the window");
    }

    #[test]
    fn stays_open_on_the_card_and_its_margin_then_closes_after_leaving() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        hover.opened();
        assert_eq!(hover.next(t), None);
        hover.moved(REACH - 1., Y, false, REACH, TOP..H, at(t, 10));
        assert_eq!(hover.next(at(t, 1000)), None);
        hover.moved(REACH + 50., Y, false, REACH, TOP..H, at(t, 1000));
        assert_eq!(hover.next(at(t, 1100)), Some((false, CLOSE_DELAY - Duration::from_millis(100))));
        hover.moved(REACH - 1., H + 10., false, REACH, TOP..H, at(t, 1150));
        assert!(hover.next(at(t, 1150)).is_some(), "below the window is off the card");
        hover.card(true, at(t, 1200));
        assert_eq!(hover.next(at(t, 2000)), None, "coming back cancels the close");
        hover.card(false, at(t, 2000));
        assert_eq!(hover.next(at(t, 2000 + CLOSE_DELAY.as_millis() as u64)), Some((false, Duration::ZERO)));
        hover.closed();
        assert_eq!(hover.next(at(t, 3000)), None);
    }

    #[test]
    fn the_card_waits_for_the_pointer_to_rest() {
        let (mut rest, t) = (Rest::default(), Instant::now());
        assert_eq!(rest.due(t), None);
        rest.enter(A, t);
        assert_eq!(rest.due(t + Duration::from_millis(200)), Some((A, Duration::from_millis(400))));
        rest.enter(A, t + Duration::from_millis(300));
        assert_eq!(rest.due(t + CARD_DELAY), Some((A, Duration::ZERO)), "moving within the row keeps the wait");
    }

    #[test]
    fn a_new_row_restarts_the_wait_and_a_late_leave_is_ignored() {
        let (mut rest, t) = (Rest::default(), Instant::now());
        rest.enter(A, t);
        rest.enter(B, t + Duration::from_millis(500));
        assert!(!rest.leave(A));
        assert_eq!(rest.due(t + Duration::from_millis(600)), Some((B, Duration::from_millis(500))));
        assert!(rest.leave(B));
        assert_eq!(rest.due(t + Duration::from_secs(5)), None);
    }

    #[test]
    fn card_lines_show_host_path_and_state() {
        assert_eq!(card_lines("https://www.example.com/docs/", true, true), ("example.com/docs".to_owned(), "Sleeping · Muted".to_owned()));
        assert_eq!(card_lines("https://example.com/", false, false), ("example.com".to_owned(), String::new()));
        assert_eq!(card_lines("about:blank", false, true), ("about:blank".to_owned(), "Muted".to_owned()));
    }
}
