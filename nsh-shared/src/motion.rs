use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;
use nsh_core::{LayoutCurve, LayoutTransition};

pub fn animates(reduced: bool, lifecycle: Lifecycle) -> bool {
    !reduced && lifecycle == Lifecycle::Active
}

pub fn while_active(period: Duration, running: impl Fn() -> bool + 'static, tick: impl Fn() + Clone + 'static) {
    let timer: Rc<RefCell<Option<Spawned<()>>>> = Rc::default();
    effect(move || {
        if let Some(old) = timer.borrow_mut().take() {
            old.cancel();
        }
        if animates(reduce_motion(), lifecycle()) && running() {
            let tick = tick.clone();
            *timer.borrow_mut() = Some(untracked(|| set_interval(period, tick)));
        }
    });
}

pub const POP: Duration = Duration::from_millis(160);
pub const FADE: Duration = Duration::from_millis(130);
pub const EXIT: Duration = Duration::from_millis(110);
pub const SLIDE: Duration = Duration::from_millis(280);
/// How far a pop-up rises as it appears.
pub const RISE: f32 = 5.;
pub const SPIN_TURN: Duration = Duration::from_millis(900);
pub const RELOAD_SPIN: Duration = Duration::from_millis(600);
const SPIN_FRAME: Duration = Duration::from_millis(33);

/// A pop-up's entrance: it fades in and rises into place over `POP`.
pub fn pop_in() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(ms(POP)).easing(Easing::EaseOut)).values(MotionChannel::Opacity, [0., 1.]).values(MotionChannel::TranslateY, [RISE, 0.])
}

/// A scrim's entrance: it fades in over `FADE`.
pub fn fade_in() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(ms(FADE)).easing(Easing::EaseOut)).values(MotionChannel::Opacity, [0., 1.])
}

pub fn fading<E: Element>(element: E) -> E {
    element.bind_motion(&motion().entering(fade_in()))
}

/// `element` popping in when it appears; it lands in place at once under Reduce Motion.
pub fn popping<E: Element>(element: E) -> E {
    element.bind_motion(&motion().entering(pop_in()))
}

/// A layout change of the next commit (rows growing, a slot sliding) over `duration`.
pub fn layout(duration: Duration) -> LayoutTransition {
    LayoutTransition::new(LayoutCurve::EaseOut, ms(duration))
}

pub fn ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1000.
}

pub fn spin_angle(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / SPIN_TURN.as_secs_f32()).fract() * std::f32::consts::TAU
}

pub fn spinner(size: f32, glyph: impl IntoElement) -> View {
    let start = std::time::Instant::now();
    let angle = signal(0f32);
    while_active(SPIN_FRAME, || true, move || angle.set(spin_angle(start.elapsed())));
    view().w(px(size)).h(px(size)).items_center().justify_center().rotate(move || Radians(angle.get())).child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_runs_only_in_the_foreground_without_reduce_motion() {
        assert!(animates(false, Lifecycle::Active));
        assert!(!animates(true, Lifecycle::Active));
        assert!(!animates(false, Lifecycle::Inactive));
        assert!(!animates(false, Lifecycle::Background));
    }

    #[test]
    fn the_durations_match_the_gpui_motion() {
        for (name, duration) in [("POP", POP), ("FADE", FADE), ("EXIT", EXIT), ("SLIDE", SLIDE)] {
            let line = format!("pub const {name}: Duration = Duration::from_millis({});", duration.as_millis());
            assert!(include_str!("../../crates/ui/src/motion.rs").contains(&line), "{line}");
        }
        assert_eq!((ms(POP), layout(SLIDE).duration_ms), (160., 280.));
    }

    #[test]
    fn the_spinner_turns_once_per_turn_time() {
        assert_eq!(spin_angle(Duration::ZERO), 0.);
        assert!((spin_angle(SPIN_TURN / 4) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert!(spin_angle(SPIN_TURN * 3) < 1e-3);
    }
}
