use std::time::{Duration, Instant};

use super::Browser;

const THRESHOLD: f32 = 60.;
const GAP: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

#[derive(Default)]
pub struct Swipe {
    dx: f32,
    dy: f32,
    armed: bool,
    last: Option<Instant>,
}

impl Swipe {
    pub fn feed(&mut self, dx: f32, dy: f32, at: Instant) -> Option<Step> {
        if self.last.is_none_or(|last| at.saturating_duration_since(last) > GAP) {
            *self = Swipe { armed: true, ..Swipe::default() };
        }
        self.last = Some(at);
        if !self.armed {
            return None;
        }
        (self.dx, self.dy) = (self.dx + dx, self.dy + dy);
        if self.dy.abs() > THRESHOLD {
            self.armed = false;
        }
        if !self.armed || self.dx.abs() < THRESHOLD || self.dx.abs() < 2. * self.dy.abs() {
            return None;
        }
        self.armed = false;
        Some(if self.dx > 0. { Step::Next } else { Step::Previous })
    }
}

impl Browser {
    pub fn neighbour_space(&self, step: Step) -> Option<usize> {
        match step {
            Step::Next => Some(self.model.space_ix + 1),
            Step::Previous => self.model.space_ix.checked_sub(1),
        }
        .filter(|ix| *ix < self.model.spaces.len())
    }

    pub fn swipe_to(&mut self, step: Step) {
        if let Some(ix) = self.neighbour_space(step) {
            self.switch_space_by_user(ix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_step_per_gesture_past_the_threshold() {
        let (mut s, t) = (Swipe::default(), Instant::now());
        let ms = |n| t + Duration::from_millis(n);
        assert_eq!(s.feed(40., 0., ms(0)), None);
        assert_eq!(s.feed(40., 5., ms(10)), Some(Step::Next));
        assert_eq!(s.feed(200., 0., ms(20)), None, "the rest of the gesture");
        assert_eq!(s.feed(-70., 0., ms(400)), Some(Step::Previous), "a pause starts the next gesture");
        assert_eq!(s.feed(10., -70., ms(800)), None, "a vertical scroll");
        assert_eq!(s.feed(100., 0., ms(810)), None, "stays vertical until it pauses");
    }

    #[test]
    fn a_swipe_moves_only_towards_a_space_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        let t = Instant::now();
        let swipe = |b: &mut Browser, dx: f32, secs: u64| {
            if let Some(step) = b.swipe.feed(dx, 0., t + Duration::from_secs(secs)) {
                b.swipe_to(step);
            }
        };
        swipe(&mut b, -80., 0);
        assert_eq!(b.model.space_ix, 0, "no space before the first");
        b.new_space();
        b.switch_space_by_user(0);
        swipe(&mut b, 80., 1);
        assert_eq!(b.model.space_ix, 1);
        swipe(&mut b, 80., 2);
        assert_eq!(b.model.space_ix, 1, "no space after the last");
        swipe(&mut b, -80., 3);
        assert_eq!(b.model.space_ix, 0);
    }
}
