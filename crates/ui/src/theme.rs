use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{PoisonError, RwLock};

use browser_core::effects::Effect;
use browser_core::theme::{Fog, Scheme, Tokens};
use gpui::{Background, BoxShadow, Rgba, SharedString, linear_color_stop, linear_gradient, point, px, rgb, rgba};

pub struct Theme;

pub const TOOLBAR_HEIGHT: f32 = 46.;
pub const ROW_HEIGHT: f32 = 34.;
pub const TEXT: f32 = 13.;
pub const TEXT_SMALL: f32 = 11.5;
pub const TEXT_TITLE: f32 = 15.;
/// A modal card's heading.
pub const TEXT_HEADING: f32 = 18.;
/// A letter in a small badge.
pub const TEXT_TINY: f32 = 10.;
/// Clearance for the window's traffic lights, matching `traffic_light_position` in the app.
/// Windows draws its window controls at the toolbar's right instead.
pub const TRAFFIC_LIGHT_INSET: f32 = if cfg!(target_os = "macos") { 84. } else { 10. };
/// GPUI's name for the platform's interface font.
pub const SYSTEM_FONT: &str = ".SystemUIFont";

pub use browser_core::prefs::ACCENTS;

/// Profile colours, stored by name in `Profile.color`.
pub const PROFILE_COLORS: [(&str, u32); 8] = [
    ("graphite", 0x8e8e93),
    ("green", 0x30d158),
    ("blue", 0x0a84ff),
    ("indigo", 0x5e5ce6),
    ("orange", 0xff9f0a),
    ("pink", 0xff375f),
    ("red", 0xff453a),
    ("vermilion", 0xff6a3d),
];

/// The active theme's tokens, shared by every window on the main thread.
static TOKENS: RwLock<Tokens> = RwLock::new(Tokens::DARK);
static FONT: RwLock<Option<SharedString>> = RwLock::new(None);
static LIGHT: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU32 = AtomicU32::new(0);

const MONOGRAM: [u32; 10] = [0xff453a, 0xff9f0a, 0xe6b800, 0x30d158, 0x40c8e0, 0x0a84ff, 0x5e5ce6, 0xbf5af2, 0xff375f, 0x8e8e93];

fn get<T>(f: impl FnOnce(&Tokens) -> T) -> T {
    f(&TOKENS.read().unwrap_or_else(PoisonError::into_inner))
}

fn gradient(g: browser_core::theme::Gradient) -> Background {
    linear_gradient(g.angle, linear_color_stop(rgba(g.from), 0.), linear_color_stop(rgba(g.to), 1.))
}

impl Theme {
    /// Makes `tokens` the look of every window; the caller refreshes the windows.
    pub fn set_tokens(tokens: Tokens, scheme: Scheme, font: Option<String>) {
        *TOKENS.write().unwrap_or_else(PoisonError::into_inner) = tokens;
        *FONT.write().unwrap_or_else(PoisonError::into_inner) = font.map(SharedString::from);
        LIGHT.store(scheme == Scheme::Light, Ordering::Relaxed);
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    /// Changes whenever the tokens change, for caches of drawn colours.
    pub fn generation() -> u32 {
        GENERATION.load(Ordering::Relaxed)
    }
    pub fn tokens() -> Tokens {
        get(|t| *t)
    }
    pub fn is_light() -> bool {
        LIGHT.load(Ordering::Relaxed)
    }
    /// The theme's interface font, else the system font.
    pub fn font() -> SharedString {
        FONT.read().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or(SharedString::new_static(SYSTEM_FONT))
    }
    pub fn backdrop() -> Background {
        get(|t| gradient(t.backdrop))
    }
    pub fn sidebar() -> Background {
        get(|t| gradient(t.sidebar))
    }
    /// The sidebar gradient washed with a space's colour, kept close to the theme so text keeps
    /// its contrast; the tint's alpha scales the wash.
    pub fn sidebar_tinted(tint: Rgba) -> Background {
        let mix = |base: u32, t: f32| {
            let (base, t) = (rgba(base), t * tint.a);
            Rgba { r: base.r + (tint.r - base.r) * t, g: base.g + (tint.g - base.g) * t, b: base.b + (tint.b - base.b) * t, a: base.a }
        };
        let g = get(|t| t.sidebar);
        linear_gradient(g.angle, linear_color_stop(mix(g.from, 0.12), 0.), linear_color_stop(mix(g.to, 0.2), 1.))
    }
    pub fn sidebar_edge() -> Rgba {
        get(|t| rgba(t.sidebar_edge))
    }
    pub fn space_label() -> Rgba {
        get(|t| rgba(t.space_label))
    }
    pub fn surface() -> Rgba {
        get(|t| rgba(t.surface))
    }
    pub fn surface_edge() -> Rgba {
        get(|t| rgba(t.surface_edge))
    }
    pub fn page() -> Rgba {
        get(|t| rgba(t.page))
    }
    pub fn panel() -> Rgba {
        get(|t| rgba(t.panel))
    }
    pub fn panel_nav() -> Rgba {
        get(|t| rgba(t.panel_nav))
    }
    pub fn panel_row() -> Rgba {
        get(|t| rgba(t.panel_row))
    }
    pub fn separator() -> Rgba {
        get(|t| rgba(t.separator))
    }
    pub fn control() -> Rgba {
        get(|t| rgba(t.control))
    }
    pub fn raised() -> Rgba {
        get(|t| rgba(t.raised))
    }
    pub fn group() -> Rgba {
        get(|t| rgba(t.group))
    }
    pub fn hover() -> Rgba {
        get(|t| rgba(t.hover))
    }
    pub fn selected() -> Rgba {
        get(|t| rgba(t.accent & 0xffffff00 | 0x52))
    }
    pub fn pressed() -> Rgba {
        get(|t| rgba(t.pressed))
    }
    pub fn hairline() -> Rgba {
        get(|t| rgba(t.hairline))
    }
    pub fn text() -> Rgba {
        get(|t| rgba(t.text))
    }
    pub fn muted() -> Rgba {
        get(|t| rgba(t.muted))
    }
    pub fn faint() -> Rgba {
        get(|t| rgba(t.faint))
    }
    /// A sleeping tab's title: grey rather than blue, so it reads apart from awake tabs in white.
    pub fn asleep() -> Rgba {
        rgba(if Theme::is_light() { 0x3c4a5a8c } else { 0xb4c2d08c })
    }
    pub fn accent() -> Rgba {
        get(|t| rgba(t.accent))
    }
    pub fn accent_soft() -> Rgba {
        get(|t| rgba(t.accent & 0xffffff00 | 0x47))
    }
    /// Text and glyphs drawn on the accent: white in every theme, as on macOS.
    pub fn on_accent() -> Rgba {
        rgb(0xf4f9fd)
    }
    /// Text and glyphs on `fill`, a user or theme colour such as the accent: white unless
    /// that drops under 3:1 there, a translucent fill taken over the surface.
    pub fn on_fill(fill: Rgba) -> Rgba {
        let b = |v: f32| (v * 255.).round() as u32;
        let packed = |c: Rgba| b(c.r) << 24 | b(c.g) << 16 | b(c.b) << 8 | b(c.a);
        rgba(browser_core::theme::on_fill(packed(fill), packed(Self::surface())))
    }
    /// Replaces the theme's accent with `color` (`0xRRGGBB`), the user's pick in Settings.
    pub fn set_accent(color: u32) {
        TOKENS.write().unwrap_or_else(PoisonError::into_inner).accent = color << 8 | 0xff;
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    /// Colours of `browser_core::fog` shades 1 to 4: faint, mid and peak cloud, and the lit edges
    /// between mid and peak.
    pub fn fog(shade: u8) -> Rgba {
        get(|t| rgba(t.fog[usize::from(shade.clamp(1, 4)) - 1]))
    }
    pub fn fog_mode() -> Fog {
        get(|t| t.fog_mode)
    }
    pub fn search_effect() -> Effect {
        get(|t| t.search_effect)
    }
    /// The search effect's colour, the accent unless the theme sets one.
    pub fn search_color() -> Rgba {
        get(|t| rgba(t.search_color()))
    }
    /// The user's choice in Settings over the theme's search bar effect.
    pub fn set_search_effect(effect: Effect) {
        TOKENS.write().unwrap_or_else(PoisonError::into_inner).search_effect = effect;
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    pub fn radius_small() -> f32 {
        get(|t| t.radius[0])
    }
    pub fn radius_row() -> f32 {
        get(|t| t.radius[1])
    }
    pub fn radius_group() -> f32 {
        get(|t| t.radius[2])
    }
    pub fn radius_page() -> f32 {
        Self::radius_group()
    }
    pub fn radius_tile() -> f32 {
        get(|t| t.radius[3])
    }
    pub fn radius_panel() -> f32 {
        get(|t| t.radius[4])
    }
    pub fn radius_card() -> f32 {
        get(|t| t.radius[5])
    }
    pub fn danger() -> Rgba {
        Secure::danger()
    }
    pub fn warning() -> Rgba {
        Secure::warning()
    }
    pub fn success() -> Rgba {
        Secure::success()
    }
    pub fn elevation(y: f32, blur: f32) -> Vec<BoxShadow> {
        let strength = get(|t| t.shadow);
        let shadow = |alpha: f32, y: f32, blur: f32| BoxShadow {
            color: Rgba { r: 0., g: 0., b: 0., a: (alpha * strength).min(1.) }.into(),
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(0.),
        };
        vec![shadow(0.18, y * 3., blur * 3.), shadow(0.28, y, blur), shadow(0.25, 1., 2.)]
    }

    pub fn profile_color(name: &str) -> Option<Rgba> {
        PROFILE_COLORS.iter().find(|(n, _)| *n == name).map(|(_, c)| rgb(*c))
    }

    /// A stable colour per host, so a site keeps its tint across tiles, rows and sessions.
    pub fn monogram(host: &str) -> Rgba {
        let hash = host.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        rgb(MONOGRAM[hash as usize % MONOGRAM.len()])
    }
}

/// The elevation scale: what each kind of card casts on what lies beneath it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Elevation {
    /// A card in a pop-up window; its widest layer (`Theme::elevation` triples the blur) stays
    /// inside the window's `controls::POPOVER_INSET` margin.
    Popover,
    /// A card floating in the window: banners, prompts, the find bar.
    Floating,
    /// The page and each split pane above the window's background.
    Page,
    /// A modal card over a dimmed window.
    Modal,
}

impl Elevation {
    /// The shadow offset and blur of the level's main layer.
    pub fn offset_blur(self) -> (f32, f32) {
        match self {
            Elevation::Popover => (2., 5.),
            Elevation::Floating => (4., 14.),
            Elevation::Page => (6., 24.),
            Elevation::Modal => (18., 48.),
        }
    }

    pub fn shadow(self) -> Vec<BoxShadow> {
        let (y, blur) = self.offset_blur();
        Theme::elevation(y, blur)
    }
}

/// Colours of security UI: the origin chip, "Not Secure", the private chip, the Shield level,
/// blocking pages, prompts and download warnings. Taken from the built-in palette of the active
/// theme's scheme, never from the theme, so no theme can hide or fake them (threat model S29).
pub struct Secure;

impl Secure {
    fn base() -> &'static Tokens {
        if Theme::is_light() { &Tokens::LIGHT } else { &Tokens::DARK }
    }
    fn pick(dark: u32, light: u32) -> Rgba {
        rgb(if Theme::is_light() { light } else { dark })
    }
    pub fn surface() -> Rgba {
        rgba(Self::base().surface)
    }
    pub fn surface_edge() -> Rgba {
        rgba(Self::base().surface_edge)
    }
    /// The origin chip's opaque fill, independent of the toolbar behind it.
    pub fn chip() -> Rgba {
        rgba(Self::base().panel_nav)
    }
    pub fn hairline() -> Rgba {
        rgba(Self::base().hairline)
    }
    pub fn control() -> Rgba {
        rgba(Self::base().control)
    }
    /// Tarn's own accent, for the default button of a prompt.
    pub fn accent() -> Rgba {
        rgba(Self::base().accent)
    }
    pub fn hover() -> Rgba {
        rgba(Self::base().hover)
    }
    pub fn pressed() -> Rgba {
        rgba(Self::base().pressed)
    }
    pub fn text() -> Rgba {
        rgba(Self::base().text)
    }
    pub fn muted() -> Rgba {
        rgba(Self::base().muted)
    }
    pub fn faint() -> Rgba {
        rgba(Self::base().faint)
    }
    /// The Shield glyph when a site runs at Strict or Lockdown.
    pub fn shield_raised() -> Rgba {
        Self::pick(0x5aaee0, 0x1567a3)
    }
    pub fn danger() -> Rgba {
        Self::pick(0xff453a, 0xd70015)
    }
    pub fn warning() -> Rgba {
        Self::pick(0xffb340, 0xa05a00)
    }
    pub fn success() -> Rgba {
        Self::pick(0x30d158, 0x248a3d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::theme::contrast;

    fn hex(c: Rgba) -> u32 {
        let b = |v: f32| (v * 255.).round() as u32;
        b(c.r) << 24 | b(c.g) << 16 | b(c.b) << 8 | b(c.a)
    }

    #[test]
    fn monogram_is_stable_per_host() {
        assert_eq!(Theme::monogram("github.com"), Theme::monogram("github.com"));
        let distinct: std::collections::HashSet<String> =
            ["a.test", "b.test", "c.test", "d.test", "e.test"].iter().map(|h| format!("{:?}", Theme::monogram(h))).collect();
        assert!(distinct.len() > 1);
    }

    #[test]
    fn profile_colours_are_named_and_distinct() {
        let names = ["graphite", "green", "blue", "indigo", "orange", "pink", "red", "vermilion"];
        assert_eq!(PROFILE_COLORS.map(|(n, _)| n), names);
        assert!(names.iter().all(|n| Theme::profile_color(n).is_some()));
        assert_eq!(Theme::profile_color(""), None);
        let colours: std::collections::HashSet<u32> = PROFILE_COLORS.iter().map(|(_, c)| *c).collect();
        assert_eq!(colours.len(), PROFILE_COLORS.len());
    }

    /// One test, since the tokens are process-wide.
    #[test]
    fn a_hostile_theme_cannot_reach_security_colours() {
        let dark = [Secure::text(), Secure::chip(), Secure::warning(), Secure::danger(), Secure::shield_raised(), Secure::muted()];
        assert_eq!(hex(Theme::text()), Tokens::DARK.text, "the default look is today's");
        assert_eq!(hex(Theme::accent()), 0x1f75b0ff);
        assert_eq!(Theme::radius_card(), 16.);
        let hostile = browser_core::theme::parse(
            ":root { --tarn-text: #173350; --tarn-raised: #f4f9fd; --tarn-panel-nav: #f4f9fd; --tarn-accent: #d9ecfa; \
             --tarn-muted: #d9ecfa; --tarn-backdrop: #ffb340; --tarn-warning: #ffb340; --tarn-danger: #000 }",
            browser_core::theme::Source::Unverified,
        )
        .unwrap();
        Theme::set_tokens(hostile.tokens, Scheme::Dark, None);
        assert_eq!([Secure::text(), Secure::chip(), Secure::warning(), Secure::danger(), Secure::shield_raised(), Secure::muted()], dark);
        assert_eq!(Theme::warning(), dark[2]);
        for bg in [Secure::chip(), Secure::surface()] {
            assert!(contrast(hex(Secure::text()), hex(bg)) >= 4.5);
            assert!(contrast(hex(Secure::warning()), hex(bg)) >= 4.5);
        }
        Theme::set_tokens(Tokens::LIGHT, Scheme::Light, Some("Inter".into()));
        assert_eq!(Theme::font(), "Inter");
        for bg in [Secure::chip(), Secure::surface()] {
            for fg in [Secure::text(), Secure::warning(), Secure::danger(), Secure::shield_raised()] {
                assert!(contrast(hex(fg), hex(bg)) >= 4.5, "{fg:?} on {bg:?}");
            }
            assert!(contrast(hex(Secure::success()), hex(bg)) >= 3.);
        }
        Theme::set_accent(0x0a7aff);
        assert_eq!(hex(Theme::accent()), 0x0a7affff);
        assert_eq!(hex(Theme::selected()), 0x0a7aff52);
        Theme::set_tokens(Tokens::DARK, Scheme::Dark, None);
        assert_eq!(Theme::font(), SYSTEM_FONT);
    }

    #[test]
    fn an_off_switch_track_shows_on_its_surface() {
        for t in [Tokens::LIGHT, Tokens::DARK] {
            for bg in [t.surface, t.panel, t.panel_row] {
                assert!(contrast(t.faint, bg) >= 2.5, "{:08x} on {bg:08x}", t.faint);
            }
        }
    }
}
