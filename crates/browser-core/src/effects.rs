//! Animated decorations around the search field: flames, aurora, sparkle or a glow with a light
//! running around it.
//! Pure and deterministic: a frame is a function of the field's size and the time, so every
//! shell draws the same thing and a still frame (Reduce Motion) is just `t = 0`.

use std::f32::consts::TAU;

/// Frames per second the shells request while an effect is on screen in the active window.
pub const FPS: f32 = 24.;
const MAX_PARTICLES: usize = 480;
/// Circles stacked in one flame tongue, base to tip.
const TONGUE: usize = 7;
const AURORA_BANDS: usize = 3;
/// The glow's light: its speed around the card in points a second, its tail's length and dots,
/// and how far outside the card's edge it runs, so the card does not hide half of it.
const GLOW_SPEED: f32 = 260.;
const GLOW_TAIL_LENGTH: f32 = 110.;
const GLOW_TAIL: usize = 22;
const GLOW_OUT: f32 = 2.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    None,
    Glow,
    Flames,
    Aurora,
    Sparkle,
}

impl Effect {
    pub const ALL: [Effect; 5] = [Effect::None, Effect::Glow, Effect::Flames, Effect::Aurora, Effect::Sparkle];

    pub fn name(self) -> &'static str {
        match self {
            Effect::None => "none",
            Effect::Glow => "glow",
            Effect::Flames => "flames",
            Effect::Aurora => "aurora",
            Effect::Sparkle => "sparkle",
        }
    }

    pub fn parse(s: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|e| e.name().eq_ignore_ascii_case(s))
    }

    /// How it is named in Settings.
    pub fn label(self) -> &'static str {
        match self {
            Effect::None => "None",
            Effect::Glow => "Glow",
            Effect::Flames => "Flames",
            Effect::Aurora => "Aurora",
            Effect::Sparkle => "Sparkle",
        }
    }

    /// Frames per second while it shows; none for `None`.
    pub fn fps(self) -> f32 {
        match self {
            Effect::None => 0.,
            _ => FPS,
        }
    }

    /// Whether it runs around the whole card rather than growing from the search row.
    pub fn around_card(self) -> bool {
        matches!(self, Effect::Glow | Effect::Sparkle)
    }
}

/// The effect a user's Settings choice (`theme` or an effect name) leaves of the theme's.
pub fn chosen(theme: Effect, setting: Option<&str>) -> Effect {
    setting.and_then(Effect::parse).unwrap_or(theme)
}

/// The colour of a particle of `heat` for an effect colour `(r, g, b)` in 0..1: warm colours burn
/// towards gold, cool ones (blue fire, sparkle on a blue accent) towards white.
pub fn tint((r, g, b): (f32, f32, f32), heat: f32) -> (f32, f32, f32) {
    let hot = if r > b { (1., 0.82, 0.32) } else { (0.94, 0.98, 1.) };
    let mix = |a: f32, b: f32| a + (b - a) * heat.clamp(0., 1.);
    (mix(r, hot.0), mix(g, hot.1), mix(b, hot.2))
}

/// Seconds since an effect started, wrapped so `f32` keeps sub-frame precision on long uptimes.
pub fn clock(elapsed: std::time::Duration) -> f32 {
    (elapsed.as_secs_f64() % 600.) as f32
}

/// One soft dot, relative to the field's top-left corner. `heat` runs from 0 (the effect colour)
/// to 1 (its hot or light end).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub alpha: f32,
    pub heat: f32,
}

/// How far particles may reach outside the field, so the shell can size the layer behind it.
pub fn reach(effect: Effect, height: f32) -> (f32, f32) {
    let (side, rise) = span(effect, height);
    let halo = match effect {
        Effect::None => 0.,
        Effect::Glow => GLOW_BLUR + GLOW_SPREAD,
        Effect::Flames => 13. * HALO,
        Effect::Aurora => height * 0.4 * HALO,
        Effect::Sparkle => 5. * HALO,
    };
    ((side + halo).min(MAX_SIDE), rise + halo)
}

/// The most an effect reaches sideways past the field: the shells' padding around the search card.
pub const MAX_SIDE: f32 = 16.;

/// The glow ring's blur and spread, in points; together they stay within `MAX_SIDE`.
pub const GLOW_BLUR: f32 = 12.;
pub const GLOW_SPREAD: f32 = 2.;

/// How much bigger than a dot the shells draw its soft halo.
pub const HALO: f32 = 1.8;

/// How far particle centres travel outside the field.
fn span(effect: Effect, height: f32) -> (f32, f32) {
    match effect {
        Effect::None | Effect::Glow | Effect::Sparkle => (0., 0.),
        Effect::Flames => (6., height * 1.1),
        Effect::Aurora => (0., height * 0.6),
    }
}

/// The glow ring's strength at `t`, between 0.35 and 1.
pub fn pulse(t: f32) -> f32 {
    0.675 + 0.325 * (t * std::f32::consts::TAU / 2.4).sin()
}

/// The outline of the field's rounded rectangle, `out` points outside it, walked clockwise from
/// the start of its top edge.
struct Edge {
    width: f32,
    height: f32,
    corner: f32,
    out: f32,
    length: f32,
}

impl Edge {
    fn new(width: f32, height: f32, corner: f32, out: f32) -> Edge {
        let corner = corner.clamp(0., width.min(height) / 2.);
        let length = 2. * (width + height - 4. * corner) + TAU * (corner + out);
        Edge { width, height, corner, out, length }
    }

    fn at(&self, along: f32) -> (f32, f32) {
        let (w, h, r, o) = (self.width, self.height, self.corner, self.out);
        let arc = TAU / 4. * (r + o);
        let mut d = along.rem_euclid(self.length);
        let legs = [
            (w - 2. * r, (r, -o), (1., 0.), (w - r, r)),
            (h - 2. * r, (w + o, r), (0., 1.), (w - r, h - r)),
            (w - 2. * r, (w - r, h + o), (-1., 0.), (r, h - r)),
            (h - 2. * r, (-o, h - r), (0., -1.), (r, r)),
        ];
        for ((len, (sx, sy), (dx, dy), (cx, cy)), turn) in legs.into_iter().zip([-0.25, 0., 0.25, 0.5_f32]) {
            if d <= len {
                return (sx + dx * d, sy + dy * d);
            }
            d -= len;
            if d <= arc {
                let angle = (turn + d / arc / 4.) * TAU;
                return (cx + (r + o) * angle.cos(), cy + (r + o) * angle.sin());
            }
            d -= arc;
        }
        (r, -o)
    }
}

/// A stable pseudo-random value in 0..1 for particle `i` and salt `k`.
fn hash(i: u32, k: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65_535.
}

/// The particles of `effect` around a `width` × `height` field with `corner` radius at `t` seconds.
pub fn frame(effect: Effect, width: f32, height: f32, corner: f32, t: f32) -> Vec<Particle> {
    if !(width.is_finite() && height.is_finite() && t.is_finite()) || width <= 0. || height <= 0. {
        return Vec::new();
    }
    let count = ((width / 10.) as usize).clamp(8, MAX_PARTICLES);
    let (_, rise) = span(effect, height);
    let particles: Vec<Particle> = match effect {
        Effect::None => Vec::new(),
        Effect::Glow => {
            // A light runs around the card's edge, its tail fading behind it.
            let edge = Edge::new(width, height, corner, GLOW_OUT);
            let head = t * GLOW_SPEED;
            (0..GLOW_TAIL)
                .map(|k| {
                    let f = k as f32 / GLOW_TAIL as f32;
                    let (x, y) = edge.at(head - f * GLOW_TAIL_LENGTH.min(edge.length / 2.));
                    let fade = (1. - f).powf(1.6);
                    Particle { x, y, radius: 1.2 + 2.6 * fade, alpha: 0.9 * fade, heat: fade }
                })
                .collect()
        }
        Effect::Flames => {
            // Tongues: circles stacked from a hot base to a thin tip. Each flickers on two rhythms
            // and sways in a wave that travels up it; embers drift and twinkle above them.
            let tongues = ((width / 12.) as usize).clamp(4, MAX_PARTICLES / (TONGUE + 1));
            let mut out = Vec::with_capacity(tongues * TONGUE + tongues);
            for i in 0..tongues as u32 {
                let phase = hash(i, 5) * TAU;
                let slow = (t * (3.5 + hash(i, 4) * 3.) + phase).sin();
                let fast = (t * (9. + hash(i, 9) * 6.) + phase * 1.7).sin();
                let flicker = 0.7 + 0.2 * slow + 0.1 * fast;
                let tall = rise * (0.45 + 0.55 * hash(i, 6)) * flicker;
                let base = (i as f32 + 0.5) / tongues as f32 * width + (t * 0.8 + phase).sin() * 2.;
                for k in 0..TONGUE {
                    let f = k as f32 / (TONGUE - 1) as f32;
                    let wave = (t * 6. + i as f32 * 1.3 - f * 2.4).sin() * 0.7 + (t * 11. + i as f32 * 0.7 - f * 3.2).sin() * 0.3;
                    out.push(Particle {
                        x: base + wave * f.powf(1.3) * 6.,
                        y: height * 0.12 - f * tall,
                        radius: (9. + hash(i, 7) * 4.) * (1. - f * 0.82),
                        alpha: (0.92 - f * 0.6) * (0.7 + 0.3 * flicker),
                        heat: (1. - f).powf(1.5),
                    });
                }
            }
            for i in 0..tongues as u32 {
                let life = 1.1 + hash(i, 1) * 0.9;
                let p = ((t + hash(i, 2) * life) % life) / life;
                let twinkle = 0.65 + 0.35 * (t * (14. + hash(i, 10) * 8.) + i as f32).sin();
                out.push(Particle {
                    x: (hash(i, 3) * width + (t * 2.2 + i as f32).sin() * 10. * p).clamp(0., width),
                    y: -p * rise,
                    radius: (1.2 + hash(i, 8) * 1.8) * (1. - 0.4 * p),
                    alpha: (1. - p) * 0.95 * twinkle,
                    heat: 0.85 * (1. - p),
                });
            }
            out
        }
        Effect::Aurora => {
            // Curtains: three bands of overlapping dots waving across the field, with brighter
            // rays drifting along them and their ends fading out.
            let n = ((width / 14.) as usize).clamp(12, 96);
            (0..AURORA_BANDS)
                .flat_map(|band| {
                    let b = band as f32;
                    (0..n).map(move |j| {
                        let along = (j as f32 + 0.5) / n as f32;
                        let wave = (along * TAU * (1.1 + 0.35 * b) + t * (0.45 + 0.15 * b) + b * 2.1).sin();
                        let ray = 0.5 + 0.5 * (along * 23. - t * (1.1 + 0.3 * b) + b).sin();
                        let ends = (along * std::f32::consts::PI).sin().sqrt();
                        Particle {
                            x: along * width,
                            y: -rise * (0.2 + 0.26 * b + 0.16 * wave),
                            radius: height * (0.26 + 0.12 * ray),
                            alpha: (0.035 + 0.075 * ray) * ends,
                            heat: (0.3 * b + 0.25 * ray).min(1.),
                        }
                    })
                })
                .collect()
        }
        Effect::Sparkle => {
            let edge = Edge::new(width, height, corner, 0.);
            (0..count as u32)
                .map(|i| {
                    // Glitter glides slowly along the edge and flashes: a quick rise, a long fade.
                    let (x, y) = edge.at((hash(i, 1) + t * 0.015 * (0.5 + hash(i, 4))) * edge.length);
                    let beat = (t * (0.5 + hash(i, 2) * 0.8) + hash(i, 3)).fract();
                    let flash = if beat < 0.12 { beat / 0.12 } else { (1. - (beat - 0.12) / 0.88).powi(3) };
                    Particle { x, y, radius: 1.2 + flash * 3.3, alpha: flash, heat: flash }
                })
                .collect()
        }
    };
    // Every halo stays inside the reach sideways, where the shells clip the layer.
    let (side, _) = reach(effect, height);
    particles
        .into_iter()
        .map(|p| {
            let edge = p.radius * HALO - side;
            Particle { x: p.x.clamp(edge.min(width / 2.), (width - edge).max(width / 2.)), ..p }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_override_the_theme_and_colours_heat_by_warmth() {
        assert_eq!(chosen(Effect::Flames, None), Effect::Flames);
        assert_eq!(chosen(Effect::Flames, Some("theme")), Effect::Flames);
        assert_eq!(chosen(Effect::Flames, Some("none")), Effect::None);
        assert_eq!(chosen(Effect::None, Some("aurora")), Effect::Aurora);
        assert_eq!(tint((1., 0.4, 0.), 1.), (1., 0.82, 0.32), "warm burns gold");
        assert_eq!(tint((0.1, 0.4, 0.8), 1.), (0.94, 0.98, 1.), "cool burns white");
        assert_eq!(tint((0.2, 0.3, 0.4), 0.), (0.2, 0.3, 0.4));
        assert!(clock(std::time::Duration::from_secs(3 * 86_400)) < 600.);
        assert_eq!((Effect::None.fps(), Effect::Glow.fps(), Effect::Flames.fps()), (0., FPS, FPS));
    }

    #[test]
    fn names_round_trip_and_none_draws_nothing() {
        for e in Effect::ALL {
            assert_eq!(Effect::parse(e.name()), Some(e));
        }
        assert_eq!(Effect::parse("FLAMES"), Some(Effect::Flames));
        assert_eq!(Effect::parse("fire"), None);
        assert!(frame(Effect::None, 500., 40., 16., 1.).is_empty());
        assert!((0..100).all(|i| (0.35..=1.).contains(&pulse(i as f32 * 0.1))));
    }

    #[test]
    fn the_glow_fits_where_the_shells_clip() {
        let (side, rise) = reach(Effect::Glow, 54.);
        assert!((GLOW_BLUR + GLOW_SPREAD..=MAX_SIDE).contains(&side) && GLOW_BLUR + GLOW_SPREAD <= rise);
        for effect in Effect::ALL {
            assert!(reach(effect, 54.).0 <= MAX_SIDE, "{effect:?}");
        }
    }

    #[test]
    fn particles_stay_in_reach_bounded_and_deterministic() {
        for effect in [Effect::Glow, Effect::Flames, Effect::Aurora, Effect::Sparkle] {
            for (w, h) in [(40., 30.), (560., 48.), (4000., 60.)] {
                let (side, rise) = reach(effect, h);
                for step in 0..200 {
                    let t = step as f32 / FPS;
                    let frame = frame(effect, w, h, 16., t);
                    assert!(!frame.is_empty() && frame.len() <= MAX_PARTICLES, "{effect:?}");
                    for p in &frame {
                        let halo = p.radius * HALO;
                        assert!(p.x - halo >= -side && p.x + halo <= w + side, "{effect:?} x {} r {}", p.x, p.radius);
                        assert!(p.y - halo >= -rise && p.y + halo <= h + side, "{effect:?} y {} r {}", p.y, p.radius);
                        assert!((0. ..=1.).contains(&p.alpha) && (0. ..=1.).contains(&p.heat) && p.radius >= 0., "{effect:?} {p:?}");
                    }
                }
                assert_eq!(super::frame(effect, w, h, 16., 1.25), super::frame(effect, w, h, 16., 1.25));
            }
        }
        assert!(frame(Effect::Flames, f32::NAN, 40., 16., 1.).is_empty());
        assert!(frame(Effect::Flames, 400., 0., 16., 1.).is_empty());
    }

    #[test]
    fn flames_flicker_and_their_embers_rise() {
        let (a, b) = (frame(Effect::Flames, 400., 40., 16., 0.), frame(Effect::Flames, 400., 40., 16., 0.05));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).filter(|(a, b)| a != b).count() > a.len() * 3 / 4, "most of the fire moves between frames");
        let embers = (400. / 12.) as usize;
        let rising = a[a.len() - embers..].iter().zip(&b[b.len() - embers..]).filter(|(a, b)| b.y < a.y).count();
        assert!(rising > embers / 2, "embers rise: {rising} of {embers}");
        assert!(a.iter().any(|p| p.heat > 0.9 && p.radius > 8.), "a hot, wide base");
    }

    #[test]
    fn the_glow_light_runs_around_the_edge_with_its_tail_behind() {
        let (w, h, r) = (400., 50., 16.);
        // Distance from the card's rounded outline, negative inside.
        let outside = |p: &Particle| {
            let (qx, qy) = ((p.x - w / 2.).abs() - (w / 2. - r), (p.y - h / 2.).abs() - (h / 2. - r));
            qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - r
        };
        let at = |t: f32| frame(Effect::Glow, w, h, r, t);
        let head = |t: f32| at(t)[0];
        for step in 0..60 {
            assert!(at(step as f32 * 0.1).iter().all(|p| (outside(p) - GLOW_OUT).abs() < 0.01), "hugs the rounded outline");
        }
        assert!(head(0.5).x > head(0.).x, "runs clockwise along the top");
        assert!(at(0.5).windows(2).all(|p| p[0].alpha >= p[1].alpha), "brightest at the head");
        let lap = Edge::new(w, h, r, GLOW_OUT).length / GLOW_SPEED;
        let (a, b) = (head(0.3), head(0.3 + lap));
        assert!((a.x - b.x).abs() < 0.05 && (a.y - b.y).abs() < 0.05, "one lap comes back round");
    }

    #[test]
    fn aurora_waves_in_bands_that_fade_at_the_ends() {
        let a = frame(Effect::Aurora, 600., 50., 16., 2.);
        assert_eq!(a.len() % AURORA_BANDS, 0);
        let band = &a[..a.len() / AURORA_BANDS];
        let brightest = band.iter().map(|p| p.alpha).fold(0., f32::max);
        assert!(band[0].alpha < brightest * 0.3 && band[band.len() - 1].alpha < brightest * 0.3, "ends fade");
        let b = frame(Effect::Aurora, 600., 50., 16., 3.);
        assert!(a.iter().zip(&b).filter(|(a, b)| (a.y - b.y).abs() > 0.5).count() > a.len() / 2, "the curtains wave");
    }

    #[test]
    fn sparkle_flashes_and_glides() {
        let frames: Vec<Vec<Particle>> = (0..48).map(|i| frame(Effect::Sparkle, 400., 50., 16., i as f32 / FPS)).collect();
        assert!(frames.iter().flatten().any(|p| p.alpha > 0.95) && frames.iter().flatten().any(|p| p.alpha < 0.05), "flashes on and off");
        assert!(frames[0].iter().zip(&frames[47]).filter(|(a, b)| a.x != b.x || a.y != b.y).count() > frames[0].len() / 2, "glides");
    }

    #[test]
    fn a_frame_is_cheap() {
        let start = std::time::Instant::now();
        for step in 0..1_000 {
            frame(Effect::Flames, 1200., 48., 16., step as f32 / FPS);
        }
        if !cfg!(debug_assertions) {
            assert!(start.elapsed().as_millis() < 20, "{:?} for 1 000 frames", start.elapsed());
        }
    }
}
