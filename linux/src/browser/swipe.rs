pub use browser_core::sidebar::{Step, Swipe};

use super::Browser;

impl Browser {
    pub fn swiped(&mut self, dx: f32, dy: f32, at: Instant) -> Option<Step> {
        let step = self.swipe.feed(dx, dy, at)?;
        self.pref(browser_core::prefs::GESTURES).then_some(step)
    }

    pub fn neighbour_space(&self, step: Step) -> Option<usize> {
        browser_core::sidebar::neighbour_space(&self.model, step, |_| true)
    }

    pub fn swipe_to(&mut self, step: Step) {
        if let Some(ix) = self.neighbour_space(step) {
            self.switch_space_by_user(ix);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

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
            if let Some(step) = b.swiped(dx, 0., t + Duration::from_secs(secs)) {
                b.swipe_to(step);
            }
        };
        swipe(&mut b, -80., 0);
        assert_eq!(b.model.space_ix, 0, "no space before the first");
        b.save_space(&b.new_space_draft());
        b.switch_space_by_user(0);
        swipe(&mut b, 80., 1);
        assert_eq!(b.model.space_ix, 1);
        swipe(&mut b, 80., 2);
        assert_eq!(b.model.space_ix, 1, "no space after the last");
        swipe(&mut b, -80., 3);
        assert_eq!(b.model.space_ix, 0);
        b.set_pref(browser_core::prefs::GESTURES, false);
        swipe(&mut b, 80., 4);
        assert_eq!(b.model.space_ix, 0, "Touchpad gestures is off");
        b.set_pref(browser_core::prefs::GESTURES, true);
        swipe(&mut b, 80., 5);
        assert_eq!(b.model.space_ix, 1);
    }
}
