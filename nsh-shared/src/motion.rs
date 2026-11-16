use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;

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

pub const FADE: Duration = Duration::from_millis(130);
pub const SPIN_TURN: Duration = Duration::from_millis(900);
pub const RELOAD_SPIN: Duration = Duration::from_millis(600);
const SPIN_FRAME: Duration = Duration::from_millis(33);

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
    fn the_fade_matches_the_gpui_motion() {
        let line = format!("pub const FADE: Duration = Duration::from_millis({});", FADE.as_millis());
        assert!(include_str!("../../crates/ui/src/motion.rs").contains(&line), "{line}");
    }

    #[test]
    fn the_spinner_turns_once_per_turn_time() {
        assert_eq!(spin_angle(Duration::ZERO), 0.);
        assert!((spin_angle(SPIN_TURN / 4) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert!(spin_angle(SPIN_TURN * 3) < 1e-3);
    }
}
