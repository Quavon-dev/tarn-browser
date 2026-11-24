use std::f32::consts::TAU;
use std::time::Duration;

use browser_core::effects::{self, Effect, Particle};
use nsh_core::element::{BoxShadow, Color};
use nsh_core::prelude::*;

use crate::theme::{Theme, themed};

pub use effects::STILL;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub fill: Rgba,
    pub soft: bool,
}

pub fn dots(effect: Effect, width: f32, height: f32, t: f32, color: Rgba) -> Vec<Dot> {
    let frame: Vec<Particle> = effects::frame(effect, width, height, Theme::radius_card(), t);
    let mut dots = frame
        .into_iter()
        .filter(|p| p.alpha > 0.01)
        .flat_map(|p| {
            let (((r, g, b), a), glint) = effects::shade(&p, (color.r, color.g, color.b));
            let glow = Dot { x: p.x, y: p.y, radius: p.radius * effects::HALO, fill: Rgba { r, g, b, a }, soft: true };
            let glint = glint.map(|((r, g, b), a)| Dot { radius: p.radius * effects::GLINT, fill: Rgba { r, g, b, a }, soft: false, ..glow });
            std::iter::once(glow).chain(glint)
        })
        .collect::<Vec<Dot>>();
    dots.sort_by_key(|d| !d.soft);
    dots
}

fn paint(painter: Painter, d: Dot, (dx, dy): (f32, f32)) -> Painter {
    let (x, y) = (dx + d.x, dy + d.y);
    let painter = if d.soft {
        let at = Point::new(Points(x), Points(y));
        let gradient = nsh_core::canvas::CanvasGradient::radial(at, Points(0.), at, Points(d.radius));
        painter.fill_style(
            effects::GLOW_STOPS.into_iter().fold(gradient, |g, (at, share)| g.add_color_stop(Percentage(at), Rgba { a: d.fill.a * share, ..d.fill })),
        )
    } else {
        painter.fill_style(d.fill)
    };
    painter.begin_path().arc(x, y, d.radius, 0., TAU).fill()
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
        crate::motion::while_active(
            Duration::from_secs_f32(1. / effect.fps().max(1.)),
            || true,
            move |held| t.set(effects::clock(held + Duration::from_secs_f32(STILL))),
        );
        nsh_core::prelude::effect(move || {
            if crate::motion::reduced() {
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
        let painter = move || {
            let (w, h) = size.get();
            let width = w - 2. * side;
            let height = if effect.around_card() { h - rise - side } else { row };
            dots(effect, width, height, t.get(), color).into_iter().fold(Painter::new(), |p, d| paint(p, d, (side, rise)))
        };
        let canvas = canvas(painter).size_full();
        let layer = layer.on_layout(move |b: &Bounds<Points>| {
            let next = (b.size.width.0, b.size.height.0);
            if size.get_untracked() != next {
                size.set(next);
            }
        });
        Some(match effect {
            Effect::Glow => layer.child(dynamic(move || card().shadow(glow(t.get(), color)))).child(canvas).child(underlay),
            _ => layer.child(canvas).child(underlay),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_particle_is_a_soft_glow_and_sparks_add_a_glint() {
        let blue = rgba(0x1f75b0ff);
        let aurora = dots(Effect::Aurora, 400., 54., STILL, blue);
        assert!(!aurora.is_empty() && aurora.iter().all(|d| d.soft), "no hard edges in the aurora");
        let sparkle = dots(Effect::Sparkle, 400., 54., STILL, blue);
        let glints: Vec<&Dot> = sparkle.iter().filter(|d| !d.soft).collect();
        assert!(
            !glints.is_empty() && glints.iter().all(|g| sparkle.iter().any(|d| d.soft && d.x == g.x && d.radius > g.radius)),
            "a glint sits inside its glow"
        );
        assert!(sparkle.windows(2).all(|w| w[0].soft || !w[1].soft), "glints paint last");
        assert!(!super::dots(Effect::Glow, 400., 54., 1., blue).is_empty(), "the glow carries its running light");
        assert!(super::dots(Effect::Glow, 400., 54., STILL, blue).is_empty(), "but not in a still frame");
        assert_eq!(super::dots(Effect::Sparkle, 400., 54., STILL, blue), super::dots(Effect::Sparkle, 400., 54., STILL, blue));
        let ring = glow(0., blue)[0].color.0;
        assert_eq!(ring >> 8, 0x1f75b0);
        assert!((ring & 0xff) > 0 && (ring & 0xff) <= 0x8d);
    }
}
