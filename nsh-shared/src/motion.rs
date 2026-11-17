use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;
use nsh_core::{LayoutAnimation, LayoutCurve, LayoutTransition};

static REDUCED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct Revision;

pub fn set_reduced(on: bool) {
    if REDUCED.swap(on, Ordering::Relaxed) != on
        && let Some(revision) = try_global::<Revision>()
    {
        Owner::detached().run(|| drop(spawn(async move { revision.set(Revision) })));
    }
}

pub fn reduced() -> bool {
    if let Some(revision) = try_global::<Revision>() {
        revision.with(|_| ());
    }
    reduce_motion() || REDUCED.load(Ordering::Relaxed)
}

pub fn animates(reduced: bool, lifecycle: Lifecycle) -> bool {
    !reduced && lifecycle == Lifecycle::Active
}

pub fn while_active(period: Duration, running: impl Fn() -> bool + 'static, tick: impl Fn() + Clone + 'static) {
    let timer: Rc<RefCell<Option<Spawned<()>>>> = Rc::default();
    effect(move || {
        if let Some(old) = timer.borrow_mut().take() {
            old.cancel();
        }
        if animates(reduced(), lifecycle()) && running() {
            let tick = tick.clone();
            *timer.borrow_mut() = Some(untracked(|| set_interval(period, tick)));
        }
    });
}

pub const POP: Duration = Duration::from_millis(160);
pub const FADE: Duration = Duration::from_millis(130);
pub const EXIT: Duration = Duration::from_millis(110);
pub const SLIDE: Duration = Duration::from_millis(280);
pub const RISE: f32 = 5.;
pub const SCRIM: f32 = 0.45;
pub const SPIN_TURN: Duration = Duration::from_millis(900);
pub const RELOAD_SPIN: Duration = Duration::from_millis(600);

pub fn fading<E: Element>(element: E) -> E {
    element.bind_motion(&fade_in())
}

pub fn popping<E: Element>(element: E) -> E {
    element.bind_motion(&pop_in())
}

pub fn layout(duration: Duration) -> LayoutTransition {
    LayoutTransition::new(LayoutCurve::EaseOut, ms(duration))
}

pub fn animate_next(moving: Duration) {
    if !reduced() {
        animate_layout(LayoutAnimation { create: Some(layout(POP)), update: Some(layout(moving)), delete: Some(layout(EXIT)) });
    }
}

pub fn ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1000.
}

pub fn spin() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(ms(SPIN_TURN)).easing(Easing::Linear).infinite()).values(MotionChannel::Rotate, [0., 360.])
}

pub fn spinner(size: f32, glyph: impl IntoElement) -> View {
    let turn = motion();
    effect(move || {
        if animates(reduce_motion(), lifecycle()) {
            drop(untracked(|| turn.play(spin())));
        } else {
            turn.stop();
        }
    });
    view().w(px(size)).h(px(size)).items_center().justify_center().bind_motion(&turn).child(glyph)
}

fn eased(duration: Duration) -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(ms(duration)).easing(Easing::EaseOut))
}

fn entering(spec: MotionSpec) -> Motion {
    if reduced() { motion() } else { motion().entering(spec) }
}

pub fn pop_in() -> Motion {
    entering(eased(POP).values(MotionChannel::Opacity, [0., 1.]).values(MotionChannel::TranslateY, [RISE, 0.]))
}

pub fn pop_out(shown: Motion, from: f32, then: impl FnOnce() + 'static) {
    if reduced() {
        then();
        return;
    }
    drop(shown.play(eased(EXIT).values(MotionChannel::Opacity, [from, 0.]).values(MotionChannel::TranslateY, [0., RISE])));
    drop(set_timeout(EXIT, then));
}

pub fn fade_in() -> Motion {
    entering(eased(FADE).values(MotionChannel::Opacity, [0., 1.]))
}

pub fn slide_up(distance: f32) -> Motion {
    entering(eased(SLIDE).values(MotionChannel::TranslateY, [distance, 0.]))
}

pub fn fade_to(opacity: f32) -> Motion {
    entering(eased(SLIDE).values(MotionChannel::Opacity, [0., opacity]))
}

pub fn transition_ms(duration: Duration) -> f32 {
    if reduced() { 0. } else { ms(duration) }
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
        let spin = spin();
        assert_eq!((spin.timing.duration_ms, spin.timing.iterations, spin.timing.easing), (900., None, Easing::Linear));
        let [(MotionChannel::Rotate, frames)] = spin.tracks.as_slice() else { panic!("{spin:?}") };
        assert_eq!(nsh_core::animation::keyframe_ends(frames, &spin.timing), Some((0., 360.)));
    }
}
