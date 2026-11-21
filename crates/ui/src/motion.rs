use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{Animation, AnimationExt, AnyElement, ElementId, IntoElement, Styled, px};

pub const POP: Duration = Duration::from_millis(160);
pub const FADE: Duration = Duration::from_millis(130);
pub const EXIT: Duration = Duration::from_millis(110);
pub const SLIDE: Duration = Duration::from_millis(280);
pub const TAB_IN: Duration = Duration::from_millis(340);
pub const TAB_OUT: Duration = Duration::from_millis(220);
pub const SWEEP: f32 = 14.;
pub const STAGGER: Duration = Duration::from_millis(22);
pub const SETTLE: Duration = Duration::from_millis(300);
/// How far a pop-up moves in its window; on Windows the window is clipped to the card, so it stays small.
pub const POPUP_TRAVEL: f32 = 6.;
/// How soon after a pop-up closed a click on its button does not reopen it: the click that took
/// the focus from the pop-up has already closed it.
pub const REOPEN_GUARD: Duration = Duration::from_millis(300);

static REDUCE: AtomicBool = AtomicBool::new(false);

pub fn set_reduced(reduce: bool) {
    REDUCE.store(reduce, Ordering::Relaxed);
}

pub fn reduced() -> bool {
    REDUCE.load(Ordering::Relaxed)
}

/// A critically damped spring, rescaled so it ends exactly at 1.
pub fn spring(t: f32) -> f32 {
    let curve = |t: f32| 1. - (1. + 9. * t) * (-9. * t).exp();
    (curve(t) / curve(1.)).clamp(0., 1.)
}

/// An underdamped spring: overshoots by about 8%, swings back and settles exactly at 1.
pub fn bounce(t: f32) -> f32 {
    let (zeta, omega) = (0.62_f32, 11.5_f32);
    let damped = omega * (1. - zeta * zeta).sqrt();
    let curve = |t: f32| 1. - (-zeta * omega * t).exp() * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin());
    if t >= 1. { 1. } else { curve(t) / curve(1.) }
}

/// Runs `step` from 0 to 1 on the spring when the element first appears.
pub fn animate<E: IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    duration: Duration,
    reduce: bool,
    step: impl Fn(E, f32) -> E + 'static,
) -> AnyElement {
    animate_linear(element, id, duration, reduce, move |e, t| step(e, spring(t)))
}

/// Like `animate`, but hands `step` the linear time, so it can ease each property on its own curve.
pub fn animate_linear<E: IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    duration: Duration,
    reduce: bool,
    step: impl Fn(E, f32) -> E + 'static,
) -> AnyElement {
    if reduce {
        return element.into_any_element();
    }
    element.with_animation(id, Animation::new(duration), step).into_any_element()
}

/// Where a part that starts at `from` and ends at `to` (shares of the whole run) stands at `t`.
pub fn phase(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0., 1.)
}

/// The `ix`th row of a list rising into place a little after the one above it.
pub fn stagger_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, ix: usize, reduce: bool) -> AnyElement {
    let delay = STAGGER.as_secs_f32() * ix.min(8) as f32;
    let total = POP.as_secs_f32() * 1.4 + delay;
    let start = delay / total;
    animate_linear(element, id, Duration::from_secs_f32(total), reduce, move |e, t| {
        let p = phase(t, start, 1.);
        e.opacity(spring(p)).relative().top(px((1. - bounce(p)) * 6.))
    })
}

/// Fades in while springing up from a few points below; GPUI cannot scale a div, so the rise stands in.
pub fn pop_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, reduce: bool) -> AnyElement {
    rise_in(element, id, POPUP_TRAVEL, reduce)
}

/// An entrance at linear time `t`: its opacity, and the share of its travel still ahead, below 0
/// while it swings past its place.
pub fn arrive(t: f32) -> (f32, f32) {
    (spring(phase(t, 0., 0.45)), 1. - bounce(t))
}

/// Fades in quickly while springing up from `distance` points below, a touch past its place and back.
pub fn rise_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, distance: f32, reduce: bool) -> AnyElement {
    animate_linear(element, id, SETTLE, reduce, move |e, t| {
        let (shown, ahead) = arrive(t);
        e.opacity(shown).relative().top(px(ahead * distance))
    })
}

/// Eased progress of a transition that started at `since`; 1 once `duration` has passed.
pub fn progress(since: Instant, duration: Duration) -> f32 {
    spring((since.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.))
}

/// The reverse of `pop_in` at progress `t`: fades out while sinking a few points.
pub fn pop_out<E: Styled>(element: E, t: f32) -> E {
    element.opacity(1. - t).relative().top(px(t * POPUP_TRAVEL))
}

pub fn popover_motion<E: Styled + IntoElement + 'static>(element: E, id: &'static str, closing: bool, reduce: bool) -> AnyElement {
    if closing {
        animate(element, (id, 1u64), EXIT, reduce, |e, t| e.opacity(1. - t).top(px(POPUP_TRAVEL * t)))
    } else {
        animate_linear(element, (id, 0u64), SETTLE, reduce, |e, t| {
            let (shown, ahead) = arrive(t);
            e.opacity(shown).top(px(POPUP_TRAVEL * ahead.max(0.)))
        })
    }
}

pub fn fade_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, reduce: bool) -> AnyElement {
    animate(element, id, FADE, reduce, |e, t| e.opacity(t))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{bounce, phase, progress, spring};

    #[test]
    fn spring_runs_from_zero_to_one_without_overshoot() {
        assert_eq!(spring(0.), 0.);
        assert_eq!(spring(1.), 1.);
        let samples: Vec<f32> = (0..=20).map(|i| spring(i as f32 / 20.)).collect();
        assert!(samples.windows(2).all(|w| w[0] <= w[1]));
        assert!(spring(0.5) > 0.9, "settles early like an ease-out");
    }

    #[test]
    fn bounce_overshoots_a_little_then_settles_at_one() {
        assert_eq!(bounce(0.), 0.);
        assert_eq!(bounce(1.), 1.);
        let peak = (0..=100).map(|i| bounce(i as f32 / 100.)).fold(0., f32::max);
        assert!((1.03..1.12).contains(&peak), "{peak}");
        assert!((bounce(0.97) - 1.).abs() < 0.01, "settled by the end");
    }

    #[test]
    fn a_pop_up_swings_less_than_its_window_inset() {
        let deepest = (0..=100).map(|i| super::arrive(i as f32 / 100.).1).fold(0., f32::min);
        assert!(super::POPUP_TRAVEL * (1. - deepest) < crate::controls::POPOVER_INSET);
        assert_eq!(super::arrive(1.), (1., 0.));
    }

    #[test]
    fn a_phase_runs_only_inside_its_share() {
        assert_eq!((phase(0.1, 0.2, 0.6), phase(0.4, 0.0, 0.8), phase(0.9, 0.2, 0.6)), (0., 0.5, 1.));
    }

    #[test]
    fn progress_ends_at_one() {
        let since = Instant::now() - Duration::from_secs(1);
        assert_eq!(progress(since, Duration::from_millis(200)), 1.);
        assert!(progress(Instant::now(), Duration::from_secs(60)) < 0.01);
    }
}
