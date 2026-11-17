use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use browser_core::effects::{self, Effect, Particle};
use nsh_core::element::{BoxShadow, Color};
use nsh_core::platform::Lifecycle;
use nsh_core::prelude::*;

use crate::theme::{Theme, themed};

pub const STILL: f32 = 0.4;

pub fn animates(effect: Effect, reduce_motion: bool, lifecycle: Lifecycle) -> bool {
    effect.fps() > 0. && !reduce_motion && lifecycle == Lifecycle::Active
}

pub fn dots(effect: Effect, width: f32, height: f32, t: f32, color: Rgba) -> Vec<(f32, f32, f32, Rgba)> {
    let frame: Vec<Particle> = effects::frame(effect, width, height, t);
    frame
        .into_iter()
        .filter(|p| p.alpha > 0.01)
        .flat_map(|p| {
            let (r, g, b) = effects::tint((color.r, color.g, color.b), p.heat);
            [(effects::HALO, 0.22), (1., 1.)].map(|(scale, alpha)| (p.x, p.y, p.radius * scale, Rgba { r, g, b, a: p.alpha * alpha }))
        })
        .collect()
}

fn glow(t: f32, color: Rgba) -> Vec<BoxShadow> {
    let alpha = (0.55 * effects::pulse(t) * color.a * 255.).round() as u32;
    let byte = |v: f32| (v * 255.).round() as u32;
    vec![BoxShadow {
        offset_x: Points(0.),
        offset_y: Points(0.),
        blur: Points(effects::GLOW_BLUR),
        spread: Points(effects::GLOW_SPREAD),
        color: Color(byte(color.r) << 24 | byte(color.g) << 16 | byte(color.b) << 8 | alpha),
        inset: false,
    }]
}

pub fn search_fx(row: f32) -> AnyElement {
    themed(move || {
        let effect = Theme::search_effect();
        if effect == Effect::None {
            return None;
        }
        let t = signal(STILL);
        let size = signal((0f32, 0f32));
        nsh_core::prelude::effect(move || {
            if animates(effect, crate::motion::reduced(), lifecycle()) {
                let start = Instant::now();
                let task = set_interval(Duration::from_secs_f32(1. / effect.fps()), move || t.set(effects::clock(start.elapsed())));
                cleanup(move || task.cancel());
            } else {
                t.set(STILL);
            }
        });
        let (side, rise) = effects::reach(effect, row);
        let color = Theme::search_color();
        let card = move || view().absolute().top(px(rise)).left(px(side)).right(px(side)).bottom(px(side)).rounded(px(Theme::radius_card()));
        let underlay = card().bg(Rgba { a: 1., ..Theme::page() });
        let layer = view()
            .absolute()
            .top(px(-rise))
            .left(px(-side))
            .right(px(-side))
            .bottom(px(-side))
            .overflow_hidden()
            .pointer_events(PointerEvents::None)
            .accessibility_elements_hidden(true);
        Some(match effect {
            Effect::Glow => layer.child(dynamic(move || card().shadow(glow(t.get(), color)))).child(underlay),
            _ => {
                let painter = move || {
                    let (w, h) = size.get();
                    let width = w - 2. * side;
                    let height = if effect == Effect::Sparkle { h - rise - side } else { row };
                    dots(effect, width, height, t.get(), color)
                        .into_iter()
                        .fold(Painter::new(), |p, (x, y, r, fill)| p.fill_style(fill).begin_path().arc(side + x, rise + y, r, 0., TAU).fill())
                };
                layer
                    .on_layout(move |b: &Bounds<Points>| {
                        let next = (b.size.width.0, b.size.height.0);
                        if size.get_untracked() != next {
                            size.set(next);
                        }
                    })
                    .child(canvas(painter).size_full())
                    .child(underlay)
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_animate_only_in_the_active_window_without_reduce_motion() {
        assert!(animates(Effect::Flames, false, Lifecycle::Active));
        assert!(animates(Effect::Glow, false, Lifecycle::Active));
        assert!(!animates(Effect::Flames, true, Lifecycle::Active), "Reduce Motion keeps a still frame");
        assert!(!animates(Effect::Flames, false, Lifecycle::Inactive), "a window in the background stops");
        assert!(!animates(Effect::Sparkle, false, Lifecycle::Background));
        assert!(!animates(Effect::None, false, Lifecycle::Active));
    }

    #[test]
    fn every_particle_is_a_halo_and_a_dot_in_the_effect_colour() {
        let blue = rgba(0x1f75b0ff);
        let dots = dots(Effect::Flames, 400., 54., STILL, blue);
        assert!(!dots.is_empty() && dots.len().is_multiple_of(2));
        for pair in dots.chunks(2) {
            assert!((pair[0].2 - pair[1].2 * effects::HALO).abs() < 1e-3 && pair[0].3.a < pair[1].3.a);
        }
        assert!(super::dots(Effect::Glow, 400., 54., STILL, blue).is_empty());
        assert_eq!(super::dots(Effect::Sparkle, 400., 54., STILL, blue), super::dots(Effect::Sparkle, 400., 54., STILL, blue));
        let ring = glow(0., blue)[0].color.0;
        assert_eq!(ring >> 8, 0x1f75b0);
        assert!((ring & 0xff) > 0 && (ring & 0xff) <= 0x8d);
    }
}
