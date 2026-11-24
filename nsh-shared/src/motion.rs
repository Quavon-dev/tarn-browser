use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;
use nsh_core::{LayoutAnimation, LayoutCurve, LayoutTransition};

static REDUCED: AtomicBool = AtomicBool::new(false);
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
static TOUCHED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Default)]
struct Woke;

pub fn touched() {
    let asleep = !browser_core::effects::awake(since_input());
    TOUCHED.store(EPOCH.elapsed().as_millis() as u64, Ordering::Relaxed);
    if asleep && let Some(woke) = try_global::<Woke>() {
        Owner::detached().run(|| drop(spawn(async move { woke.set(Woke) })));
    }
}

fn since_input() -> Duration {
    EPOCH.elapsed().saturating_sub(Duration::from_millis(TOUCHED.load(Ordering::Relaxed)))
}

thread_local! {
    static POINTER: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
}

fn pointer_moved(e: &MouseEvent) {
    let at = (e.page_position.x.0, e.page_position.y.0);
    let moved = POINTER.get().is_some_and(|from| browser_core::effects::moved(from, at));
    if moved || POINTER.get().is_none() {
        POINTER.set(Some(at));
    }
    if moved {
        touched();
    }
}

pub fn watching(view: View) -> View {
    view.on_mouse_move(pointer_moved)
        .on_mouse_up(|_: &MouseEvent| touched())
        .on_wheel(|_: &WheelEvent| touched())
        .on_touch(|_: &nsh_core::TouchEvent| touched())
}

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

pub fn while_active(period: Duration, running: impl Fn() -> bool + 'static, tick: impl Fn(Duration) + Clone + 'static) {
    let timer: Rc<RefCell<Option<Spawned<()>>>> = Rc::default();
    let clock = Rc::new(RefCell::new(browser_core::effects::Clock::default()));
    let rest = signal(0u32);
    touched();
    effect(move || {
        global::<Woke>().get();
        rest.get();
        clock.borrow_mut().tick(Instant::now(), false);
        if let Some(old) = timer.borrow_mut().take() {
            old.cancel();
        }
        if animates(reduced(), lifecycle()) && running() && browser_core::effects::awake(since_input()) {
            let (clock, tick) = (clock.clone(), tick.clone());
            *timer.borrow_mut() = Some(untracked(|| {
                set_interval(period, move || {
                    let held = clock.borrow_mut().tick(Instant::now(), browser_core::effects::awake(since_input()));
                    match held {
                        Some(held) => tick(held),
                        None => rest.update(|n| *n += 1),
                    }
                })
            }));
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
pub const SPRING_DAMPING: f32 = 1.;
pub const SHAKE: Duration = browser_core::palette::SHAKE;

/// One sideways shake of `palette::shake` over `SHAKE`.
pub fn shake() -> MotionSpec {
    const STEPS: u16 = 24;
    MotionSpec::new(KeyframeTiming::new(ms(SHAKE)).easing(Easing::Linear))
        .values(MotionChannel::TranslateX, (0..=STEPS).map(|i| browser_core::palette::shake(f32::from(i) / f32::from(STEPS))))
}

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

/// A space switch: what moves springs over `SLIDE`, and a second switch picks up wherever the
/// first stands; nothing moves under Reduce Motion.
pub fn space_switch() {
    if !reduced() {
        let spring = LayoutTransition::new(LayoutCurve::FittedSpring { damping_ratio: SPRING_DAMPING }, ms(SLIDE));
        animate_layout(LayoutAnimation { create: Some(layout(POP)), update: Some(spring), delete: Some(layout(EXIT)) });
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
    fn touching_resets_the_time_since_input() {
        touched();
        assert!(since_input() < Duration::from_secs(1));
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
