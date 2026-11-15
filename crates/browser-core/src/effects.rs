//! Animated decorations around the search field: flames, aurora, sparkle or a pulsing glow.
//! Pure and deterministic: a frame is a function of the field's size and the time, so every
//! shell draws the same thing and a still frame (Reduce Motion) is just `t = 0`.

/// Frames per second the shells request while an effect is on screen in the active window.
pub const FPS: f32 = 24.;
const MAX_PARTICLES: usize = 480;
/// Circles stacked in one flame tongue, base to tip.
const TONGUE: usize = 7;

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

    /// Frames per second while it shows: none for `None`, a slow pulse for `Glow`.
    pub fn fps(self) -> f32 {
        match self {
            Effect::None => 0.,
            Effect::Glow => FPS / 3.,
            _ => FPS,
        }
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

/// A stable pseudo-random value in 0..1 for particle `i` and salt `k`.
fn hash(i: u32, k: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65_535.
}

/// The particles of `effect` around a `width` × `height` field at `t` seconds.
pub fn frame(effect: Effect, width: f32, height: f32, t: f32) -> Vec<Particle> {
    if !(width.is_finite() && height.is_finite() && t.is_finite()) || width <= 0. || height <= 0. {
        return Vec::new();
    }
    let count = ((width / 10.) as usize).clamp(8, MAX_PARTICLES);
    let (_, rise) = span(effect, height);
    let particles: Vec<Particle> = match effect {
        Effect::None | Effect::Glow => Vec::new(),
        Effect::Flames => {
            // Tongues: circles stacked from a hot base to a thin tip, each tongue flickering in
            // height and swaying; embers drift above them.
            let tongues = ((width / 14.) as usize).clamp(4, 56);
            let mut out = Vec::with_capacity(tongues * TONGUE + tongues);
            for i in 0..tongues as u32 {
                let flicker = 0.55 + 0.45 * ((t * (5. + hash(i, 4) * 4.) + hash(i, 5) * 6.3).sin() * 0.5 + 0.5);
                let tall = rise * (0.45 + 0.55 * hash(i, 6)) * flicker;
                let base = (i as f32 + 0.5) / tongues as f32 * width;
                for k in 0..TONGUE {
                    let f = k as f32 / (TONGUE - 1) as f32;
                    out.push(Particle {
                        x: base + (t * 7. + i as f32 * 1.3 + k as f32 * 0.6).sin() * f * 5.,
                        y: height * 0.12 - f * tall,
                        radius: (9. + hash(i, 7) * 4.) * (1. - f * 0.8),
                        alpha: (0.9 - f * 0.55) * (0.75 + 0.25 * flicker),
                        heat: (1. - f).powf(1.5),
                    });
                }
            }
            for i in 0..tongues as u32 {
                let life = 1.1 + hash(i, 1) * 0.8;
                let p = ((t + hash(i, 2) * life) % life) / life;
                out.push(Particle {
                    x: (hash(i, 3) * width + (t * 3. + i as f32).sin() * 6. * p).clamp(0., width),
                    y: -p * rise,
                    radius: 1.2 + hash(i, 8) * 1.8,
                    alpha: (1. - p) * 0.9,
                    heat: 0.8 * (1. - p),
                });
            }
            out.truncate(MAX_PARTICLES);
            out
        }
        Effect::Aurora => (0..(count / 2).max(6) as u32)
            .map(|i| {
                let n = (count / 2).max(6) as f32;
                let drift = (t * 0.06 + hash(i, 1)).fract();
                let x = ((i as f32 / n + drift).fract()) * width;
                let wave = (t * 0.7 + i as f32 * 1.7).sin();
                Particle {
                    x,
                    y: -rise * (0.35 + 0.2 * wave),
                    radius: height * (0.22 + hash(i, 2) * 0.18),
                    alpha: 0.1 + 0.08 * wave.abs(),
                    heat: 0.25 * (0.5 + 0.5 * wave),
                }
            })
            .collect(),
        Effect::Sparkle => (0..count as u32)
            .map(|i| {
                let along = hash(i, 1) * 2. * (width + height);
                let (x, y) = if along < width {
                    (along, 0.)
                } else if along < width + height {
                    (width, along - width)
                } else if along < 2. * width + height {
                    (2. * width + height - along, height)
                } else {
                    (0., 2. * (width + height) - along)
                };
                let twinkle = (t * (2. + hash(i, 2) * 3.) + hash(i, 3) * 6.3).sin().max(0.);
                Particle { x, y, radius: 1.5 + twinkle * 3.5, alpha: twinkle, heat: twinkle }
            })
            .collect(),
    };
    // Every halo stays inside `MAX_SIDE`, where the shells clip the layer.
    particles
        .into_iter()
        .map(|p| {
            let edge = (p.radius * HALO - MAX_SIDE).max(0.);
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
        assert_eq!((Effect::None.fps(), Effect::Glow.fps(), Effect::Flames.fps()), (0., FPS / 3., FPS));
    }

    #[test]
    fn names_round_trip_and_none_draws_nothing() {
        for e in Effect::ALL {
            assert_eq!(Effect::parse(e.name()), Some(e));
        }
        assert_eq!(Effect::parse("FLAMES"), Some(Effect::Flames));
        assert_eq!(Effect::parse("fire"), None);
        assert!(frame(Effect::None, 500., 40., 1.).is_empty() && frame(Effect::Glow, 500., 40., 1.).is_empty());
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
        for effect in [Effect::Flames, Effect::Aurora, Effect::Sparkle] {
            for (w, h) in [(40., 30.), (560., 48.), (4000., 60.)] {
                let (side, rise) = reach(effect, h);
                for step in 0..200 {
                    let t = step as f32 / FPS;
                    let frame = frame(effect, w, h, t);
                    assert!(!frame.is_empty() && frame.len() <= MAX_PARTICLES, "{effect:?}");
                    for p in &frame {
                        let halo = p.radius * HALO;
                        assert!(p.x - halo >= -side && p.x + halo <= w + side, "{effect:?} x {} r {}", p.x, p.radius);
                        assert!(p.y - halo >= -rise && p.y + halo <= h + side, "{effect:?} y {} r {}", p.y, p.radius);
                        assert!((0. ..=1.).contains(&p.alpha) && (0. ..=1.).contains(&p.heat) && p.radius >= 0., "{effect:?} {p:?}");
                    }
                }
                assert_eq!(super::frame(effect, w, h, 1.25), super::frame(effect, w, h, 1.25));
            }
        }
        assert!(frame(Effect::Flames, f32::NAN, 40., 1.).is_empty());
        assert!(frame(Effect::Flames, 400., 0., 1.).is_empty());
    }

    #[test]
    fn flames_flicker_and_their_embers_rise() {
        let (a, b) = (frame(Effect::Flames, 400., 40., 0.), frame(Effect::Flames, 400., 40., 0.05));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).filter(|(a, b)| a != b).count() > a.len() * 3 / 4, "most of the fire moves between frames");
        let embers = (400. / 14.) as usize;
        let rising = a[a.len() - embers..].iter().zip(&b[b.len() - embers..]).filter(|(a, b)| b.y < a.y).count();
        assert!(rising > embers / 2, "embers rise: {rising} of {embers}");
        assert!(a.iter().any(|p| p.heat > 0.9 && p.radius > 8.), "a hot, wide base");
    }

    #[test]
    fn a_frame_is_cheap() {
        let start = std::time::Instant::now();
        for step in 0..1_000 {
            frame(Effect::Flames, 1200., 48., step as f32 / FPS);
        }
        if !cfg!(debug_assertions) {
            assert!(start.elapsed().as_millis() < 20, "{:?} for 1 000 frames", start.elapsed());
        }
    }
}
