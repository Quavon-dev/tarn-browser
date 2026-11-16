use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{PoisonError, RwLock};

use browser_core::effects::Effect;
use browser_core::theme::{Fog, Scheme, Tokens};
use nsh_core::element::{BoxShadow, Color};
use nsh_core::prelude::*;

pub struct Theme;

pub const RADIUS_SMALL: f32 = Tokens::DARK.radius[0];
pub const RADIUS_ROW: f32 = Tokens::DARK.radius[1];
pub const RADIUS_GROUP: f32 = Tokens::DARK.radius[2];
pub const RADIUS_PAGE: f32 = RADIUS_GROUP;
pub const RADIUS_TILE: f32 = Tokens::DARK.radius[3];
pub const RADIUS_PANEL: f32 = Tokens::DARK.radius[4];
pub const RADIUS_CARD: f32 = Tokens::DARK.radius[5];
pub const TOOLBAR_HEIGHT: f32 = 46.;
pub const ROW_HEIGHT: f32 = 34.;
pub const TEXT: f32 = 13.;
pub const TEXT_SMALL: f32 = 11.5;
pub const TEXT_TITLE: f32 = 15.;

pub const ACCENTS: [(&str, u32); 9] = [
    ("Tarn", 0x1f75b0),
    ("Blue", 0x0a7aff),
    ("Purple", 0xa550a7),
    ("Pink", 0xf74f9e),
    ("Red", 0xff5257),
    ("Orange", 0xf7821b),
    ("Yellow", 0xd9a400),
    ("Green", 0x62ba46),
    ("Graphite", 0x8c8c8c),
];

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

const MONOGRAM: [u32; 10] = [0xff453a, 0xff9f0a, 0xe6b800, 0x30d158, 0x40c8e0, 0x0a84ff, 0x5e5ce6, 0xbf5af2, 0xff375f, 0x8e8e93];

static TOKENS: RwLock<Tokens> = RwLock::new(Tokens::DARK);
pub const SYSTEM_FONT: &str = "system-ui";
static FONT: RwLock<Option<String>> = RwLock::new(None);
static LIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct Revision;

fn track() {
    if let Some(revision) = try_global::<Revision>() {
        revision.with(|_| ());
    }
}

fn changed() {
    if let Some(revision) = try_global::<Revision>() {
        Owner::detached().run(|| drop(spawn(async move { revision.set(Revision) })));
    }
}

fn get<T>(f: impl FnOnce(&Tokens) -> T) -> T {
    track();
    f(&TOKENS.read().unwrap_or_else(PoisonError::into_inner))
}

fn update(f: impl FnOnce(&mut Tokens)) -> bool {
    let mut tokens = TOKENS.write().unwrap_or_else(PoisonError::into_inner);
    let before = *tokens;
    f(&mut tokens);
    *tokens != before
}

fn gradient(angle: f32, from: u32, to: u32) -> Gradient {
    Gradient { angle, from: rgba(from), to: rgba(to) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub angle: f32,
    pub from: Rgba,
    pub to: Rgba,
}

pub fn themed<E: IntoElements>(f: impl Fn() -> E + 'static) -> AnyElement {
    dynamic(move || {
        track();
        f()
    })
}

impl Theme {
    pub fn set_tokens(tokens: Tokens, scheme: Scheme, font: Option<String>) {
        let font_changed = std::mem::replace(&mut *FONT.write().unwrap_or_else(PoisonError::into_inner), font.clone()) != font;
        let light_changed = LIGHT.swap(scheme == Scheme::Light, Ordering::Relaxed) != (scheme == Scheme::Light);
        if update(|t| *t = tokens) | font_changed | light_changed {
            changed();
        }
    }
    pub fn tokens() -> Tokens {
        get(|t| *t)
    }
    pub fn is_light() -> bool {
        track();
        LIGHT.load(Ordering::Relaxed)
    }
    pub fn font() -> Option<SharedString> {
        track();
        FONT.read().unwrap_or_else(PoisonError::into_inner).clone().map(SharedString::from)
    }
    pub fn font_family() -> SharedString {
        Theme::font().map_or_else(|| SharedString::from(SYSTEM_FONT), |f| SharedString::from(format!("\"{f}\", {SYSTEM_FONT}")))
    }
    pub fn clear() -> Rgba {
        rgba(0)
    }
    pub fn backdrop() -> Gradient {
        get(|t| gradient(t.backdrop.angle, t.backdrop.from, t.backdrop.to))
    }
    pub fn sidebar() -> Gradient {
        get(|t| gradient(t.sidebar.angle, t.sidebar.from, t.sidebar.to))
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
    pub fn asleep() -> Rgba {
        rgba(if Theme::is_light() { 0x3c4a5a8c } else { 0xb4c2d08c })
    }
    pub fn accent() -> Rgba {
        get(|t| rgba(t.accent))
    }
    pub fn accent_soft() -> Rgba {
        get(|t| rgba(t.accent & 0xffffff00 | 0x47))
    }
    pub fn on_accent() -> Rgba {
        rgb(0xf4f9fd)
    }
    /// Text and glyphs on `fill`, such as the accent a theme sets: white unless that drops under 3:1 there.
    pub fn on_fill(fill: Rgba) -> Rgba {
        let b = |v: f32| (v * 255.).round() as u32;
        let packed = |c: Rgba| b(c.r) << 24 | b(c.g) << 16 | b(c.b) << 8 | b(c.a);
        rgba(browser_core::theme::on_fill(packed(fill), packed(Self::surface())))
    }
    pub fn set_accent(color: u32) {
        if update(|t| t.accent = color << 8 | 0xff) {
            changed();
        }
    }
    pub fn fog(shade: u8) -> Rgba {
        get(|t| rgba(t.fog[usize::from(shade.clamp(1, 4)) - 1]))
    }
    pub fn fog_mode() -> Fog {
        get(|t| t.fog_mode)
    }
    pub fn search_effect() -> Effect {
        get(|t| t.search_effect)
    }
    pub fn search_color() -> Rgba {
        get(|t| rgba(t.search_color()))
    }
    pub fn set_search_effect(effect: Effect) {
        if update(|t| t.search_effect = effect) {
            changed();
        }
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
            offset_x: Points(0.),
            offset_y: Points(y),
            blur: Points(blur),
            spread: Points(0.),
            color: Color((alpha * strength * 255.).round().clamp(0., 255.) as u32),
            inset: false,
        };
        vec![shadow(0.18, y * 3., blur * 3.), shadow(0.28, y, blur), shadow(0.25, 1., 2.)]
    }
    pub fn profile_color(name: &str) -> Option<Rgba> {
        PROFILE_COLORS.iter().find(|(n, _)| *n == name).map(|(_, c)| rgb(*c))
    }
    pub fn monogram(host: &str) -> Rgba {
        let hash = host.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        rgb(MONOGRAM[hash as usize % MONOGRAM.len()])
    }
}

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
    pub fn chip() -> Rgba {
        rgba(Self::base().panel_nav)
    }
    pub fn hairline() -> Rgba {
        rgba(Self::base().hairline)
    }
    pub fn control() -> Rgba {
        rgba(Self::base().control)
    }
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

pub trait Paint: Styled + Sized {
    fn paint(self, gradient: Gradient) -> Self {
        self.bg_linear(deg(gradient.angle)).gradient_from(gradient.from).gradient_to(gradient.to)
    }

    fn surface(self) -> Self {
        self.bg(Theme::surface()).border_1().border_color(Theme::surface_edge()).shadow(Theme::elevation(4., 12.))
    }
}

impl<T: Styled + Sized> Paint for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::theme::contrast;

    fn hex(c: Rgba) -> u32 {
        let b = |v: f32| (v * 255.).round() as u32;
        b(c.r) << 24 | b(c.g) << 16 | b(c.b) << 8 | b(c.a)
    }

    #[test]
    fn the_palette_matches_the_gpui_theme() {
        let gpui = include_str!("../../crates/ui/src/theme.rs");
        for (name, hex) in ACCENTS.iter().chain(PROFILE_COLORS.iter()) {
            assert!(gpui.contains(&format!("(\"{name}\", {hex:#08x})")), "{name}");
        }
        let monogram = MONOGRAM.iter().map(|c| format!("{c:#08x}")).collect::<Vec<_>>().join(", ");
        assert!(gpui.contains(&format!("const MONOGRAM: [u32; 10] = [{monogram}];")));
        let source = include_str!("theme.rs").split("#[cfg(test)]").next().unwrap();
        let secure = |src: &str| -> Vec<String> {
            src[src.find("impl Secure {").unwrap()..]
                .split("\n}\n")
                .next()
                .unwrap()
                .lines()
                .map(str::trim)
                .filter(|l| !l.starts_with("//"))
                .map(String::from)
                .collect()
        };
        assert_eq!(secure(source), secure(gpui), "Secure is the Mac's, line for line");
        for line in [
            "0x3c4a5a8c } else { 0xb4c2d08c",
            "t.accent & 0xffffff00 | 0x52",
            "t.accent & 0xffffff00 | 0x47",
            "rgb(0xf4f9fd)",
            "shadow(0.18, y * 3., blur * 3.), shadow(0.28, y, blur), shadow(0.25, 1., 2.)",
        ] {
            assert!(gpui.contains(line) && source.contains(line), "{line}");
        }
        assert_eq!([RADIUS_SMALL, RADIUS_ROW, RADIUS_GROUP, RADIUS_TILE, RADIUS_PANEL, RADIUS_CARD], [6., 9., 12., 14., 12., 16.]);
        assert_eq!((TEXT, TEXT_SMALL, TEXT_TITLE, ROW_HEIGHT, TOOLBAR_HEIGHT), (13., 11.5, 15., 34., 46.));
        for line in ["TOOLBAR_HEIGHT: f32 = 46.;", "ROW_HEIGHT: f32 = 34.;", "TEXT: f32 = 13.;", "TEXT_SMALL: f32 = 11.5;", "TEXT_TITLE: f32 = 15.;"] {
            assert!(gpui.contains(line) && source.contains(line), "{line}");
        }
    }

    #[test]
    fn a_hostile_theme_cannot_reach_security_colours() {
        let dark = [Secure::text(), Secure::chip(), Secure::warning(), Secure::danger(), Secure::shield_raised(), Secure::muted()];
        assert_eq!(hex(Theme::text()), Tokens::DARK.text);
        assert_eq!(hex(Theme::accent()), 0x1f75b0ff);
        assert_eq!(Theme::radius_card(), RADIUS_CARD);
        let hostile = browser_core::theme::parse(
            ":root { --tarn-text: #173350; --tarn-raised: #f4f9fd; --tarn-panel-nav: #f4f9fd; --tarn-accent: #d9ecfa; \
             --tarn-muted: #d9ecfa; --tarn-backdrop: #ffb340; --tarn-warning: #ffb340; --tarn-danger: #000; --tarn-radius-card: 3px; \
             --tarn-search-effect: flames }",
            browser_core::theme::Source::Unverified,
        )
        .unwrap();
        Theme::set_tokens(hostile.tokens, Scheme::Dark, None);
        assert_eq!([Secure::text(), Secure::chip(), Secure::warning(), Secure::danger(), Secure::shield_raised(), Secure::muted()], dark);
        assert_eq!(Theme::warning(), dark[2]);
        assert_eq!((hex(Theme::accent()), Theme::radius_card(), Theme::search_effect()), (0xd9ecfaff, 3., Effect::Flames));
        assert_eq!(hex(Theme::on_fill(Theme::accent())), Tokens::LIGHT.text, "a pale accent takes a dark label");
        assert_eq!(Theme::on_fill(Secure::accent()), Theme::on_accent(), "the secure accent keeps white");
        for bg in [Secure::chip(), Secure::surface()] {
            assert!(contrast(hex(Secure::text()), hex(bg)) >= 4.5);
            assert!(contrast(hex(Secure::warning()), hex(bg)) >= 4.5);
        }
        Theme::set_tokens(Tokens::LIGHT, Scheme::Light, Some("Inter".into()));
        assert_eq!(Theme::font().as_deref(), Some("Inter"));
        assert_eq!(Theme::font_family().as_ref(), "\"Inter\", system-ui");
        for bg in [Secure::chip(), Secure::surface()] {
            for fg in [Secure::text(), Secure::warning(), Secure::danger(), Secure::shield_raised()] {
                assert!(contrast(hex(fg), hex(bg)) >= 4.5, "{fg:?} on {bg:?}");
            }
            assert!(contrast(hex(Secure::success()), hex(bg)) >= 3.);
        }
        Theme::set_accent(0x0a7aff);
        assert_eq!((hex(Theme::accent()), hex(Theme::selected()), hex(Theme::accent_soft())), (0x0a7affff, 0x0a7aff52, 0x0a7aff47));
        Theme::set_search_effect(Effect::Glow);
        assert_eq!(Theme::search_effect(), Effect::Glow);
        assert_eq!(hex(Theme::search_color()), 0x0a7affff);
        Theme::set_tokens(Tokens::DARK, Scheme::Dark, None);
        assert_eq!((Theme::font(), Theme::fog(4), Theme::elevation(2., 4.).len()), (None, rgba(0x8ed1f7ff), 3));
        assert!(Theme::profile_color("indigo").is_some() && Theme::profile_color("").is_none());
        assert_eq!(Theme::monogram("a.test"), Theme::monogram("a.test"));
    }
}
