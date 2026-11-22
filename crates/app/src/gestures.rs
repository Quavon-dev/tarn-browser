use gpui::{Context, ScrollDelta, ScrollWheelEvent, TouchPhase, Window};

use crate::browser::Browser;
use crate::prefs::Pref;

/// A pause in wheel events that ends a Windows touchpad gesture, and a wheel line in pixels.
#[cfg(windows)]
const WHEEL_PAUSE: std::time::Duration = std::time::Duration::from_millis(180);
#[cfg(windows)]
const WHEEL_LINE: f32 = 40.;

pub use browser_core::sidebar::{Step, Swipe};

/// Feeds a trackpad scroll to `swipe`; a mouse wheel's lines are never a swipe.
fn touch(swipe: &mut Swipe, phase: TouchPhase, delta: ScrollDelta) -> Option<Step> {
    use browser_core::sidebar::Phase;
    let ScrollDelta::Pixels(delta) = delta else { return None };
    let phase = match phase {
        TouchPhase::Started => Phase::Started,
        TouchPhase::Moved => Phase::Moved,
        TouchPhase::Ended => Phase::Ended,
    };
    swipe.touch(phase, f32::from(delta.x), f32::from(delta.y))
}

/// A touchpad scrolls by fractions of a wheel line; a mouse's wheel, tilted too, by whole lines.
#[cfg(windows)]
fn touchpad(lines: gpui::Point<f32>) -> bool {
    lines.x.fract() != 0. || lines.y.fract() != 0.
}

/// Past this share of the sidebar's width, or flung, a released drag switches space.
const COMMIT: f32 = 0.35;
const FLING: f32 = 12.;
const ENGAGE: f32 = 10.;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Drag {
    /// Where the fingers have moved the list, positive to the right.
    Follow(f32),
    Release(Option<Step>),
}

/// Tracks a two-finger swipe that the sidebar's list follows until the fingers lift.
#[derive(Default)]
pub struct Pan {
    dx: f32,
    dy: f32,
    speed: f32,
    armed: bool,
    engaged: bool,
    inverted: bool,
}

impl Pan {
    pub fn new(natural: bool) -> Self {
        Pan { inverted: !natural, ..Pan::default() }
    }

    pub fn feed(&mut self, phase: TouchPhase, delta: ScrollDelta, width: f32) -> Option<Drag> {
        let ScrollDelta::Pixels(delta) = delta else { return None };
        match phase {
            TouchPhase::Started => {
                *self = Pan { armed: true, ..Pan::new(!self.inverted) };
                None
            }
            TouchPhase::Ended => {
                let release = self.engaged.then(|| Drag::Release(release(self.dx, self.speed, width)));
                *self = Pan::new(!self.inverted);
                release
            }
            TouchPhase::Moved if self.armed => {
                let x = if self.inverted { -f32::from(delta.x) } else { f32::from(delta.x) };
                self.dx += x;
                self.dy += f32::from(delta.y);
                self.speed = x;
                if !self.engaged {
                    self.engaged = self.dx.abs() >= ENGAGE && self.dx.abs() >= 2. * self.dy.abs();
                    self.armed = self.engaged || self.dy.abs() < ENGAGE;
                }
                self.engaged.then_some(Drag::Follow(self.dx))
            }
            TouchPhase::Moved => None,
        }
    }
}

fn release(dx: f32, speed: f32, width: f32) -> Option<Step> {
    let commit = dx.abs() > COMMIT * width || (speed * dx > 0. && speed.abs() > FLING);
    commit.then_some(if dx < 0. { Step::Next } else { Step::Previous })
}

/// Where the list sits for a pull of `offset`: it never leaves its place towards a missing space.
pub fn follow(offset: f32, width: f32, neighbour: bool) -> f32 {
    if neighbour { offset.clamp(-width, width) } else { 0. }
}

impl Browser {
    fn swipe_step(&mut self, event: &ScrollWheelEvent) -> Option<Step> {
        if !self.pref(Pref::Gestures) {
            return None;
        }
        if matches!(event.touch_phase, TouchPhase::Started) {
            self.swipe = Swipe::natural(engine::menu::natural_scrolling());
        }
        touch(&mut self.swipe, event.touch_phase, event.delta)
    }

    /// Anywhere over the sidebar: the list follows the fingers to the previous or next space.
    pub fn sidebar_swipe(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(windows)]
        for event in self.wheel_gesture(event, None, window, cx) {
            self.sidebar_gesture(&event, window, cx);
        }
        #[cfg(not(windows))]
        self.sidebar_gesture(event, window, cx);
    }

    fn sidebar_gesture(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pref(Pref::Gestures) {
            return;
        }
        if matches!(event.touch_phase, TouchPhase::Started) {
            self.pan = Pan::new(engine::menu::natural_scrolling());
            self.cancel_drag(cx);
        }
        match self.pan.feed(event.touch_phase, event.delta, self.sidebar_width) {
            Some(Drag::Follow(dx)) => self.follow_drag(dx, cx),
            Some(Drag::Release(step)) => match step.and_then(|step| self.neighbour(step)) {
                Some(ix) => self.switch_space(ix, window, cx),
                None => self.cancel_drag(cx),
            },
            None => {}
        }
    }

    pub(crate) fn neighbour(&self, step: Step) -> Option<usize> {
        browser_core::sidebar::neighbour_space(&self.model, step, |_| true)
    }

    /// Over the address: previous or next tab. Elsewhere on the toolbar: collapse or show the sidebar.
    pub fn toolbar_swipe(&mut self, event: &ScrollWheelEvent, address: bool, window: &mut Window, cx: &mut Context<Self>) {
        if address {
            cx.stop_propagation();
        }
        #[cfg(windows)]
        for event in self.wheel_gesture(event, Some(address), window, cx) {
            self.toolbar_gesture(&event, address, window, cx);
        }
        #[cfg(not(windows))]
        self.toolbar_gesture(event, address, window, cx);
    }

    /// Windows reports a touchpad's two-finger scrolling as wheel lines without touch phases: a
    /// burst of them is one gesture, which starts after a pause and ends once the scrolling stops.
    /// `toolbar` names the toolbar's handler (and whether over the address), `None` the sidebar's.
    #[cfg(windows)]
    fn wheel_gesture(&mut self, event: &ScrollWheelEvent, toolbar: Option<bool>, window: &mut Window, cx: &mut Context<Self>) -> Vec<ScrollWheelEvent> {
        let ScrollDelta::Lines(lines) = event.delta else { return vec![event.clone()] };
        if !self.pref(Pref::Gestures) || !touchpad(lines) {
            return Vec::new();
        }
        let still = ScrollWheelEvent { delta: ScrollDelta::Pixels(gpui::Point::default()), touch_phase: TouchPhase::Ended, ..event.clone() };
        let moved =
            ScrollWheelEvent { delta: ScrollDelta::Pixels(gpui::point(gpui::px(lines.x * WHEEL_LINE), gpui::px(lines.y * WHEEL_LINE))), ..event.clone() };
        // A gesture that moves to the other region ends where it was first.
        let fresh = match self.wheel.take() {
            Some((target, _)) if target == toolbar => false,
            Some((target, _)) => {
                self.end_gesture(target, &still, window, cx);
                true
            }
            None => true,
        };
        let ended = still.clone();
        // Each event replaces the pending end, which dropping cancels.
        let end = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(WHEEL_PAUSE).await;
            this.update_in(cx, |b, window, cx| {
                b.wheel = None;
                b.end_gesture(toolbar, &ended, window, cx);
            })
            .ok();
        });
        self.wheel = Some((toolbar, end));
        let started = ScrollWheelEvent { touch_phase: TouchPhase::Started, ..still };
        if fresh { vec![started, moved] } else { vec![moved] }
    }

    #[cfg(windows)]
    fn end_gesture(&mut self, toolbar: Option<bool>, ended: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        match toolbar {
            Some(address) => self.toolbar_gesture(ended, address, window, cx),
            None => self.sidebar_gesture(ended, window, cx),
        }
    }

    fn toolbar_gesture(&mut self, event: &ScrollWheelEvent, address: bool, window: &mut Window, cx: &mut Context<Self>) {
        match (self.swipe_step(event), address) {
            (Some(step), true) => self.step_tab(step, window, cx),
            (Some(Step::Next), false) if self.sidebar_open => self.toggle_sidebar(cx),
            (Some(Step::Previous), false) if !self.sidebar_open => self.toggle_sidebar(cx),
            _ => {}
        }
    }

    /// A three-finger or mouse swipe outside the web view, which has its own swipe navigation.
    pub fn swipe_history(&mut self, back: bool) {
        if !self.pref(Pref::Gestures) {
            return;
        }
        if back { self.back() } else { self.forward() }
    }

    fn step_tab(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let keys = self.model.visible_tab_order();
        let Some(ix) = keys.iter().position(|k| Some(*k) == self.model.active) else { return };
        let next = match step {
            Step::Previous => ix.checked_sub(1),
            Step::Next => Some(ix + 1),
        };
        if let Some(key) = next.and_then(|ix| keys.get(ix)) {
            self.switch_to(*key, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn only_a_touchpads_fractional_lines_make_a_gesture() {
        assert!(super::touchpad(gpui::point(-0.25, 0.)));
        assert!(super::touchpad(gpui::point(0., 0.4)));
        assert!(!super::touchpad(gpui::point(3., 0.)), "a tilted wheel's notch");
        assert!(!super::touchpad(gpui::point(0., -3.)), "a wheel's notch");
    }

    use super::*;
    use gpui::{point, px};

    fn moved(swipe: &mut Swipe, x: f32, y: f32) -> Option<Step> {
        touch(swipe, TouchPhase::Moved, ScrollDelta::Pixels(point(px(x), px(y))))
    }

    #[test]
    fn one_step_per_gesture_past_the_threshold() {
        let mut swipe = Swipe::natural(true);
        assert_eq!(moved(&mut swipe, -100., 0.), None, "momentum without a started gesture");
        touch(&mut swipe, TouchPhase::Started, ScrollDelta::Pixels(point(px(0.), px(0.))));
        assert_eq!(moved(&mut swipe, -40., 0.), None);
        assert_eq!(moved(&mut swipe, -40., 5.), Some(Step::Next));
        assert_eq!(moved(&mut swipe, -200., 0.), None);
        touch(&mut swipe, TouchPhase::Ended, ScrollDelta::Pixels(point(px(0.), px(0.))));
        touch(&mut swipe, TouchPhase::Started, ScrollDelta::Pixels(point(px(0.), px(0.))));
        assert_eq!(moved(&mut swipe, 70., 0.), Some(Step::Previous));
    }

    #[test]
    fn natural_scrolling_off_flips_the_direction() {
        let step = |dx: f32, natural: bool| {
            let mut swipe = Swipe::natural(natural);
            touch(&mut swipe, TouchPhase::Started, ScrollDelta::Pixels(point(px(0.), px(0.))));
            moved(&mut swipe, dx, 0.)
        };
        assert_eq!((step(70., true), step(-70., true)), (Some(Step::Previous), Some(Step::Next)));
        assert_eq!((step(70., false), step(-70., false)), (Some(Step::Next), Some(Step::Previous)));
    }

    fn pan(pan: &mut Pan, phase: TouchPhase, x: f32, y: f32) -> Option<Drag> {
        pan.feed(phase, ScrollDelta::Pixels(point(px(x), px(y))), 200.)
    }

    #[test]
    fn the_list_follows_a_horizontal_drag_and_commits_past_the_threshold() {
        let mut p = Pan::default();
        assert_eq!(pan(&mut p, TouchPhase::Moved, -30., 0.), None, "momentum without a started gesture");
        pan(&mut p, TouchPhase::Started, 0., 0.);
        assert_eq!(pan(&mut p, TouchPhase::Moved, -6., 0.), None);
        assert_eq!(pan(&mut p, TouchPhase::Moved, -6., 1.), Some(Drag::Follow(-12.)));
        assert_eq!(pan(&mut p, TouchPhase::Moved, -60., 0.), Some(Drag::Follow(-72.)));
        assert_eq!(pan(&mut p, TouchPhase::Moved, -2., 0.), Some(Drag::Follow(-74.)));
        assert_eq!(pan(&mut p, TouchPhase::Ended, 0., 0.), Some(Drag::Release(Some(Step::Next))));
        assert_eq!(pan(&mut p, TouchPhase::Moved, -30., 0.), None);

        pan(&mut p, TouchPhase::Started, 0., 0.);
        pan(&mut p, TouchPhase::Moved, 40., 0.);
        pan(&mut p, TouchPhase::Moved, -2., 0.);
        assert_eq!(pan(&mut p, TouchPhase::Ended, 0., 0.), Some(Drag::Release(None)), "short and slow springs back");
    }

    #[test]
    fn a_fling_commits_and_natural_scrolling_off_flips_the_drag() {
        let mut p = Pan::new(false);
        pan(&mut p, TouchPhase::Started, 0., 0.);
        assert_eq!(pan(&mut p, TouchPhase::Moved, -20., 0.), Some(Drag::Follow(20.)));
        assert_eq!(pan(&mut p, TouchPhase::Ended, 0., 0.), Some(Drag::Release(Some(Step::Previous))));
        assert_eq!(release(-20., 15., 200.), None, "flung back towards the rest point");
        assert_eq!(release(-71., 0., 200.), Some(Step::Next));
    }

    #[test]
    fn vertical_scrolls_never_engage_the_drag() {
        let mut p = Pan::default();
        pan(&mut p, TouchPhase::Started, 0., 0.);
        assert_eq!(pan(&mut p, TouchPhase::Moved, 4., 12.), None);
        assert_eq!(pan(&mut p, TouchPhase::Moved, -150., 0.), None);
        assert_eq!(pan(&mut p, TouchPhase::Ended, 0., 0.), None);
        pan(&mut p, TouchPhase::Started, 0., 0.);
        assert_eq!(pan(&mut p, TouchPhase::Moved, 12., 7.), None);
    }

    #[test]
    fn the_list_moves_only_towards_a_space_that_exists() {
        assert_eq!(follow(80., 200., false), 0.);
        assert_eq!(follow(-80., 200., true), -80.);
        assert_eq!(follow(-500., 200., true), -200.);
    }

    #[test]
    fn vertical_scrolls_and_wheels_never_step() {
        let mut swipe = Swipe::natural(true);
        touch(&mut swipe, TouchPhase::Started, ScrollDelta::Pixels(point(px(0.), px(0.))));
        assert_eq!(moved(&mut swipe, 0., 80.), None);
        assert_eq!(moved(&mut swipe, -200., 0.), None);
        touch(&mut swipe, TouchPhase::Started, ScrollDelta::Pixels(point(px(0.), px(0.))));
        assert_eq!(moved(&mut swipe, -70., 40.), None);
        assert_eq!(touch(&mut swipe, TouchPhase::Moved, ScrollDelta::Lines(point(-10., 0.))), None);
    }
}
