use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{Animation, AnimationExt, AnyElement, ElementId, IntoElement, Styled, px};

pub const POP: Duration = Duration::from_millis(160);
pub const FADE: Duration = Duration::from_millis(130);
pub const EXIT: Duration = Duration::from_millis(110);
pub const SLIDE: Duration = Duration::from_millis(280);
/// How soon after a pop-up closed a click on its button does not reopen it: the click that took
/// the focus from the pop-up has already closed it.
pub const REOPEN_GUARD: Duration = Duration::from_millis(300);

static REDUCE: AtomicBool = AtomicBool::new(false);

/// Records the Reduce Motion setting for controls drawn without the browser at hand.
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

/// Runs `step` from 0 to 1 on the spring when the element first appears.
pub fn animate<E: IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
    duration: Duration,
    reduce: bool,
    step: impl Fn(E, f32) -> E + 'static,
) -> AnyElement {
    if reduce {
        return element.into_any_element();
    }
    element.with_animation(id, Animation::new(duration).with_easing(spring), step).into_any_element()
}

/// Fades in while settling from a few points below; GPUI cannot scale a div, so the rise stands in.
pub fn pop_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, reduce: bool) -> AnyElement {
    animate(element, id, POP, reduce, |e, t| e.opacity(t).relative().top(px((1. - t) * 5.)))
}

/// Eased progress of a transition that started at `since`; 1 once `duration` has passed.
pub fn progress(since: Instant, duration: Duration) -> f32 {
    spring((since.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.))
}

/// The reverse of `pop_in` at progress `t`: fades out while sinking a few points.
pub fn pop_out<E: Styled>(element: E, t: f32) -> E {
    element.opacity(1. - t).relative().top(px(t * 5.))
}

/// A pop-up card's motion: in, it fades in while rising a few points (`POP`); while `closing`,
/// it fades out while sinking as far over `EXIT`, as long as its window waits before it goes.
pub fn popover_motion<E: Styled + IntoElement + 'static>(element: E, id: &'static str, closing: bool, reduce: bool) -> AnyElement {
    if closing {
        animate(element, (id, 1u64), EXIT, reduce, |e, t| e.opacity(1. - t).top(px(5. * t)))
    } else {
        animate(element, (id, 0u64), POP, reduce, |e, t| e.opacity(t).top(px(5. * (1. - t))))
    }
}

pub fn fade_in<E: Styled + IntoElement + 'static>(element: E, id: impl Into<ElementId>, reduce: bool) -> AnyElement {
    animate(element, id, FADE, reduce, |e, t| e.opacity(t))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{progress, spring};

    #[test]
    fn spring_runs_from_zero_to_one_without_overshoot() {
        assert_eq!(spring(0.), 0.);
        assert_eq!(spring(1.), 1.);
        let samples: Vec<f32> = (0..=20).map(|i| spring(i as f32 / 20.)).collect();
        assert!(samples.windows(2).all(|w| w[0] <= w[1]));
        assert!(spring(0.5) > 0.9, "settles early like an ease-out");
    }

    #[test]
    fn progress_ends_at_one() {
        let since = Instant::now() - Duration::from_secs(1);
        assert_eq!(progress(since, Duration::from_millis(200)), 1.);
        assert!(progress(Instant::now(), Duration::from_secs(60)) < 0.01);
    }
}
