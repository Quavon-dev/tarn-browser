//! Themes: one CSS file whose `:root` custom properties set the shell's tokens and whose
//! `@tarn-site` blocks style web pages. Pure and shared; the shells only apply the result.
//! Rules and limits: `docs/themes.md`, threat model S29.

use cssparser::{ParseError, Parser, ToCss, Token};
use store::{Id, InstalledTheme, Store};

use crate::effects::Effect;

pub const MAX_THEME_BYTES: usize = 256 * 1024;
pub const MAX_ANY_SITE_BYTES: usize = 16 * 1024;
pub const MAX_RULES: usize = 2_000;
const MAX_DEPTH: u8 = 12;
const MAX_TEXT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
}

/// Who wrote the CSS, which decides how much its page CSS may do (`docs/themes.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Yours,
    BuiltIn,
    Official,
    Unverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fog {
    On,
    Still,
    Off,
}

/// A two-stop gradient; a plain colour has `from == to`. Colours are `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub angle: f32,
    pub from: u32,
    pub to: u32,
}

/// The shell's themable values. Colours are `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub backdrop: Gradient,
    pub sidebar: Gradient,
    pub sidebar_edge: u32,
    pub space_label: u32,
    pub surface: u32,
    pub surface_edge: u32,
    pub page: u32,
    pub panel: u32,
    pub panel_nav: u32,
    pub panel_row: u32,
    pub separator: u32,
    pub control: u32,
    pub raised: u32,
    pub group: u32,
    pub hover: u32,
    pub pressed: u32,
    pub hairline: u32,
    pub text: u32,
    pub muted: u32,
    pub faint: u32,
    pub accent: u32,
    pub fog: [u32; 4],
    /// small, row, group, tile, panel, card.
    pub radius: [f32; 6],
    pub shadow: f32,
    pub fog_mode: Fog,
    /// The decoration around the search field (`crate::effects`) and its colour; `None` means the
    /// accent.
    pub search_effect: Effect,
    pub search_color: Option<u32>,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        backdrop: Gradient { angle: 160., from: 0x10243aff, to: 0x0c1d31ff },
        sidebar: Gradient { angle: 180., from: 0x173a5aff, to: 0x112d48ff },
        sidebar_edge: 0x8ed1f729,
        space_label: 0x8ed1f7ff,
        surface: 0x173350ff,
        surface_edge: 0x8ed1f72e,
        page: 0x0e2134ff,
        panel: 0x10243aff,
        panel_nav: 0x153049ff,
        panel_row: 0x1a3854ff,
        separator: 0x8ed1f71f,
        control: 0xffffff1c,
        raised: 0x8ed1f714,
        group: 0x8ed1f717,
        hover: 0x8ed1f71a,
        pressed: 0x8ed1f733,
        hairline: 0x8ed1f71f,
        text: 0xf4f9fdff,
        muted: 0xd9ecfabf,
        faint: 0xd9ecfa9a,
        accent: 0x1f75b0ff,
        fog: [0x2a5478ff, 0x4f8bb8ff, 0xe3f3fdff, 0x8ed1f7ff],
        radius: [6., 9., 12., 14., 12., 16.],
        shadow: 1.,
        fog_mode: Fog::On,
        search_effect: Effect::None,
        search_color: None,
    };

    pub const LIGHT: Tokens = Tokens {
        backdrop: Gradient { angle: 160., from: 0xeef3f8ff, to: 0xe2eaf2ff },
        sidebar: Gradient { angle: 180., from: 0xe4edf6ff, to: 0xd9e5f0ff },
        sidebar_edge: 0x1567a326,
        space_label: 0x1567a3ff,
        surface: 0xffffffff,
        surface_edge: 0x1567a32e,
        page: 0xf7fafcff,
        panel: 0xf1f5f9ff,
        panel_nav: 0xe7eef5ff,
        panel_row: 0xffffffff,
        separator: 0x1567a31f,
        control: 0x0f22360f,
        raised: 0xffffffb3,
        group: 0x1567a30d,
        hover: 0x1567a314,
        pressed: 0x1567a329,
        hairline: 0x1567a31f,
        text: 0x0f2236ff,
        muted: 0x1b3550c7,
        faint: 0x1b3550b8,
        accent: 0x1f75b0ff,
        fog: [0xd6e4f1ff, 0xaac7e1ff, 0x4f7fa8ff, 0x1f75b0ff],
        radius: [6., 9., 12., 14., 12., 16.],
        shadow: 0.6,
        fog_mode: Fog::On,
        search_effect: Effect::None,
        search_color: None,
    };

    /// The search effect's colour: the theme's, else the accent.
    pub fn search_color(&self) -> u32 {
        self.search_color.unwrap_or(self.accent)
    }

    pub fn of(scheme: Scheme) -> Tokens {
        match scheme {
            Scheme::Dark => Self::DARK,
            Scheme::Light => Self::LIGHT,
        }
    }
}

type Slot = fn(&mut Tokens) -> &mut u32;

const COLORS: [(&str, Slot); 23] = [
    ("sidebar-edge", |t| &mut t.sidebar_edge),
    ("space-label", |t| &mut t.space_label),
    ("surface", |t| &mut t.surface),
    ("surface-edge", |t| &mut t.surface_edge),
    ("page", |t| &mut t.page),
    ("panel", |t| &mut t.panel),
    ("panel-nav", |t| &mut t.panel_nav),
    ("panel-row", |t| &mut t.panel_row),
    ("separator", |t| &mut t.separator),
    ("control", |t| &mut t.control),
    ("raised", |t| &mut t.raised),
    ("group", |t| &mut t.group),
    ("hover", |t| &mut t.hover),
    ("pressed", |t| &mut t.pressed),
    ("hairline", |t| &mut t.hairline),
    ("text", |t| &mut t.text),
    ("muted", |t| &mut t.muted),
    ("faint", |t| &mut t.faint),
    ("accent", |t| &mut t.accent),
    ("fog-1", |t| &mut t.fog[0]),
    ("fog-2", |t| &mut t.fog[1]),
    ("fog-3", |t| &mut t.fog[2]),
    ("fog-4", |t| &mut t.fog[3]),
];

const RADII: [&str; 6] = ["small", "row", "group", "tile", "panel", "card"];

/// Security colours and anything else no theme may set (`ui::theme::Secure`).
const FIXED: [&str; 3] = ["danger", "warning", "success"];

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub name: String,
    pub author: String,
    pub version: String,
    pub scheme: Scheme,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostPattern {
    Any,
    Exact(String),
    /// The host and its subdomains.
    Suffix(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SiteSheet {
    pub hosts: Vec<HostPattern>,
    pub css: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub meta: Meta,
    pub tokens: Tokens,
    pub font: Option<String>,
    /// Token names whose colour was changed to reach the contrast floor.
    pub adjusted: Vec<String>,
    pub warnings: Vec<String>,
    pub sites: Vec<SiteSheet>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeError {
    TooLarge,
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::TooLarge => write!(f, "The theme is larger than {} KB.", MAX_THEME_BYTES / 1024),
        }
    }
}

#[derive(Clone, Debug)]
enum Node<'i> {
    Tok(Token<'i>),
    Block(Token<'i>, Vec<Node<'i>>),
}

fn tree<'i>(p: &mut Parser<'i>) -> Result<Vec<Node<'i>>, ParseError<()>> {
    let mut out = Vec::new();
    while let Ok(token) = p.next_including_whitespace_and_comments() {
        let token = token.clone();
        match token {
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let inner = p.parse_nested_block(tree)?;
                out.push(Node::Block(token, inner));
            }
            _ => out.push(Node::Tok(token)),
        }
    }
    Ok(out)
}

fn is_trivia(n: &Node) -> bool {
    matches!(n, Node::Tok(Token::WhiteSpace(_) | Token::Comment(_)))
}

fn trim<'a, 'i>(mut nodes: &'a [Node<'i>]) -> &'a [Node<'i>] {
    while nodes.first().is_some_and(is_trivia) {
        nodes = &nodes[1..];
    }
    while nodes.last().is_some_and(is_trivia) {
        nodes = &nodes[..nodes.len() - 1];
    }
    nodes
}

fn split<'a, 'i>(nodes: &'a [Node<'i>], at: fn(&Token) -> bool) -> Vec<&'a [Node<'i>]> {
    nodes.split(|n| matches!(n, Node::Tok(t) if at(t))).map(trim).collect()
}

enum Item<'a, 'i> {
    Rule(&'a [Node<'i>], &'a [Node<'i>]),
    Stray,
}

/// Rules (`prelude { body }`) and anything else ended by `;` or the input.
fn items<'a, 'i>(nodes: &'a [Node<'i>]) -> Vec<Item<'a, 'i>> {
    let (mut out, mut start) = (Vec::new(), 0);
    for (i, n) in nodes.iter().enumerate() {
        match n {
            Node::Block(Token::CurlyBracketBlock, body) => {
                out.push(Item::Rule(trim(&nodes[start..i]), body));
                start = i + 1;
            }
            Node::Tok(Token::Semicolon) => {
                if !trim(&nodes[start..i]).is_empty() {
                    out.push(Item::Stray);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if !trim(&nodes[start..]).is_empty() {
        out.push(Item::Stray);
    }
    out
}

/// `name: value` pairs of a declaration block; nested rules and malformed parts are `None`.
fn declarations<'a, 'i>(body: &'a [Node<'i>]) -> Vec<Option<(String, &'a [Node<'i>])>> {
    split(body, |t| matches!(t, Token::Semicolon))
        .into_iter()
        .filter(|d| !d.is_empty())
        .map(|d| {
            if d.iter().any(|n| matches!(n, Node::Block(Token::CurlyBracketBlock, _))) {
                return None;
            }
            let Some(Node::Tok(Token::Ident(name))) = d.first() else { return None };
            let rest = trim(&d[1..]);
            let Some(Node::Tok(Token::Colon)) = rest.first() else { return None };
            Some((name.to_ascii_lowercase(), trim(&rest[1..])))
        })
        .collect()
}

fn write(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Tok(Token::WhiteSpace(_)) => out.push(' '),
            // An empty comment keeps the two tokens around it apart, as the input had them.
            Node::Tok(Token::Comment(_)) => out.push_str("/**/"),
            Node::Tok(t) => {
                let _ = t.to_css(out);
            }
            Node::Block(open, inner) => {
                let _ = open.to_css(out);
                write(inner, out);
                out.push(match open {
                    Token::SquareBracketBlock => ']',
                    Token::CurlyBracketBlock => '}',
                    _ => ')',
                });
            }
        }
    }
}

/// Parses a theme. Invalid values fall back and are listed in `warnings`; refused page CSS is
/// left out. Only an oversized file fails.
pub fn parse(css: &str, source: Source) -> Result<Parsed, ThemeError> {
    parse_inner(css, source, true)
}

/// `parse` without the contrast floor, for editing: the colours as the file states them.
pub fn parse_as_written(css: &str, source: Source) -> Result<Parsed, ThemeError> {
    parse_inner(css, source, false)
}

fn parse_inner(css: &str, source: Source, readable: bool) -> Result<Parsed, ThemeError> {
    if css.len() > MAX_THEME_BYTES {
        return Err(ThemeError::TooLarge);
    }
    let mut p = Parser::new(css);
    p.set_nested_block_limit(MAX_DEPTH);
    let mut warnings = Vec::new();
    let nodes = tree(&mut p).unwrap_or_else(|_| {
        warnings.push("The theme nests blocks too deeply; it was ignored.".to_owned());
        Vec::new()
    });
    let mut meta = Meta { name: String::new(), author: String::new(), version: String::new(), scheme: Scheme::Dark };
    let (mut root, mut sites) = (Vec::new(), Vec::new());
    for item in items(&nodes) {
        let Item::Rule(prelude, body) = item else {
            warnings.push("Text outside a rule was ignored.".to_owned());
            continue;
        };
        match prelude {
            [Node::Tok(Token::AtKeyword(k))] if k.eq_ignore_ascii_case("tarn-theme") => read_meta(body, &mut meta, &mut warnings),
            [Node::Tok(Token::Colon), Node::Tok(Token::Ident(r))] if r.eq_ignore_ascii_case("root") => root.push(body),
            [Node::Tok(Token::AtKeyword(k)), hosts @ ..] if k.eq_ignore_ascii_case("tarn-site") => sites.push((hosts, body)),
            _ => warnings.push("Only :root, @tarn-theme and @tarn-site are read; another rule was ignored.".to_owned()),
        }
    }
    let mut tokens = Tokens::of(meta.scheme);
    let mut font = None;
    for body in root {
        read_tokens(body, &mut tokens, &mut font, &mut warnings);
    }
    let adjusted = if readable { enforce_contrast(&mut tokens, meta.scheme) } else { Vec::new() };
    let mut sheets = Vec::new();
    let (mut rules, mut any_bytes) = (0, 0);
    for (hosts, body) in sites {
        let Some(hosts) = host_patterns(hosts) else {
            warnings.push("An @tarn-site block names no valid host; it was ignored.".to_owned());
            continue;
        };
        let any = hosts.contains(&HostPattern::Any);
        let tier = if any || source == Source::Unverified { Tier::Cosmetic } else { Tier::Full };
        let mut s = Sanitizer { tier, any, rules: &mut rules, warnings: &mut warnings };
        let mut css = String::new();
        s.rules(body, &mut css, 0);
        if any {
            any_bytes += css.len();
            if any_bytes > MAX_ANY_SITE_BYTES {
                warnings.push(format!("The styles for every site are larger than {} KB in all; the rest were left out.", MAX_ANY_SITE_BYTES / 1024));
                continue;
            }
        }
        if !css.is_empty() {
            sheets.push(SiteSheet { hosts, css });
        }
    }
    warnings.dedup();
    Ok(Parsed { meta, tokens, font, adjusted, warnings, sites: sheets })
}

fn text_value(value: &[Node]) -> Option<String> {
    let [Node::Tok(Token::QuotedString(s) | Token::Ident(s))] = value else { return None };
    let clean: String = s.chars().filter(|c| !c.is_control() && !crate::text::is_bidi(*c)).take(MAX_TEXT).collect();
    let clean = clean.trim().to_owned();
    (!clean.is_empty()).then_some(clean)
}

fn read_meta(body: &[Node], meta: &mut Meta, warnings: &mut Vec<String>) {
    for (name, value) in declarations(body).into_iter().flatten() {
        let text = text_value(value);
        match (name.as_str(), text) {
            ("name", Some(t)) => meta.name = t,
            ("author", Some(t)) => meta.author = t,
            ("version", Some(t)) => meta.version = t,
            ("scheme", Some(t)) if t.eq_ignore_ascii_case("light") => meta.scheme = Scheme::Light,
            ("scheme", Some(t)) if t.eq_ignore_ascii_case("dark") => meta.scheme = Scheme::Dark,
            _ => warnings.push(format!("@tarn-theme: \"{name}\" was ignored.")),
        }
    }
}

fn read_tokens(body: &[Node], tokens: &mut Tokens, font: &mut Option<String>, warnings: &mut Vec<String>) {
    for decl in declarations(body) {
        let Some((name, value)) = decl else {
            warnings.push(":root: a malformed declaration was ignored.".to_owned());
            continue;
        };
        let Some(key) = name.strip_prefix("--tarn-") else { continue };
        let ok = if let Some((_, slot)) = COLORS.iter().find(|(n, _)| *n == key) {
            color(value).is_some_and(|c| {
                let floor = matches!(key, "text" | "muted") && (c & 0xff) < 153;
                if !floor {
                    *slot(tokens) = c;
                }
                !floor
            })
        } else if key == "backdrop" || key == "sidebar" {
            gradient(value).map(|g| *if key == "backdrop" { &mut tokens.backdrop } else { &mut tokens.sidebar } = g).is_some()
        } else if let Some(i) = key.strip_prefix("radius-").and_then(|r| RADII.iter().position(|n| *n == r)) {
            px(value).map(|v| tokens.radius[i] = v.clamp(0., 24.)).is_some()
        } else if key == "shadow" {
            number(value).map(|v| tokens.shadow = v.clamp(0., 1.5)).is_some()
        } else if key == "fog" {
            match text_value(value).as_deref() {
                Some("on") => Some(Fog::On),
                Some("still") => Some(Fog::Still),
                Some("off") => Some(Fog::Off),
                _ => None,
            }
            .map(|f| tokens.fog_mode = f)
            .is_some()
        } else if key == "search-color" {
            color(value).map(|c| tokens.search_color = Some(c)).is_some()
        } else if key == "search-effect" {
            text_value(value).and_then(|e| Effect::parse(&e)).map(|e| tokens.search_effect = e).is_some()
        } else if key == "font" {
            text_value(value).filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric() || " -_".contains(c))).map(|f| *font = Some(f)).is_some()
        } else if FIXED.contains(&key) {
            warnings.push(format!("--tarn-{key} is fixed for security and cannot be themed."));
            continue;
        } else {
            warnings.push(format!("--tarn-{key} is not a theme token."));
            continue;
        };
        if !ok {
            warnings.push(format!("--tarn-{key} has a value Tarn cannot use; the default was kept."));
        }
    }
}

fn number(value: &[Node]) -> Option<f32> {
    match value {
        [Node::Tok(Token::Number { value, .. })] => Some(*value).filter(|v| v.is_finite()),
        _ => None,
    }
}

fn px(value: &[Node]) -> Option<f32> {
    match value {
        [Node::Tok(Token::Dimension { value, unit, .. })] if unit.eq_ignore_ascii_case("px") && value.is_finite() => Some(*value),
        [Node::Tok(Token::Number { value, .. })] if *value == 0. => Some(0.),
        _ => None,
    }
}

fn args<'a, 'i>(inner: &'a [Node<'i>]) -> Vec<f32> {
    inner
        .iter()
        .filter_map(|n| match n {
            Node::Tok(Token::Number { value, .. }) => Some(*value),
            Node::Tok(Token::Percentage { unit_value, .. }) => Some(*unit_value * 100.),
            Node::Tok(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("deg") => Some(*value),
            _ => None,
        })
        .collect()
}

/// A colour as `0xRRGGBBAA`: hex, `rgb[a]()`, `hsl[a]()`, a CSS colour name or `transparent`.
fn color(value: &[Node]) -> Option<u32> {
    let pack = |r: f32, g: f32, b: f32, a: f32| {
        let c = |v: f32| v.clamp(0., 255.).round() as u32;
        (c(r) << 24) | (c(g) << 16) | (c(b) << 8) | c(a * 255.)
    };
    let alpha = |inner: &[Node], v: &[f32]| match v.get(3) {
        None => Some(1.),
        Some(a) => {
            let percent = inner.iter().filter(|n| !is_trivia(n) && !matches!(n, Node::Tok(Token::Comma | Token::Delim('/')))).nth(3);
            Some(if matches!(percent, Some(Node::Tok(Token::Percentage { .. }))) { a / 100. } else { *a }.clamp(0., 1.))
        }
    };
    match value {
        [Node::Tok(Token::Hash(h) | Token::IDHash(h))] => {
            let digits: Vec<u32> = h.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
            let full: Vec<u32> = match digits.len() {
                3 | 4 => digits.iter().map(|d| d * 17).collect(),
                6 | 8 => digits.chunks(2).map(|p| p[0] * 16 + p[1]).collect(),
                _ => return None,
            };
            Some((full[0] << 24) | (full[1] << 16) | (full[2] << 8) | full.get(3).copied().unwrap_or(255))
        }
        [Node::Tok(Token::Ident(i))] if i.eq_ignore_ascii_case("transparent") => Some(0),
        [Node::Tok(Token::Ident(i))] => {
            cssparser::color::parse_named_color(i).ok().map(|(r, g, b)| (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | 0xff)
        }
        // Only numbers, separators and `/`: a keyword such as `none` or `from` would change the
        // colour or its alpha in ways the numbers here do not show.
        [Node::Block(_, inner)]
            if !inner.iter().all(|n| {
                is_trivia(n)
                    || matches!(n, Node::Tok(Token::Number { .. } | Token::Percentage { .. } | Token::Comma | Token::Delim('/')))
                    || matches!(n, Node::Tok(Token::Dimension { unit, .. }) if unit.eq_ignore_ascii_case("deg"))
            }) =>
        {
            None
        }
        [Node::Block(Token::Function(f), inner)] if f.eq_ignore_ascii_case("rgb") || f.eq_ignore_ascii_case("rgba") => {
            let v = args(inner);
            if !(3..=4).contains(&v.len()) {
                return None;
            }
            let channel = |i: usize| {
                let n = inner.iter().filter(|n| !is_trivia(n) && !matches!(n, Node::Tok(Token::Comma | Token::Delim('/')))).nth(i);
                if matches!(n, Some(Node::Tok(Token::Percentage { .. }))) { v[i] * 2.55 } else { v[i] }
            };
            Some(pack(channel(0), channel(1), channel(2), alpha(inner, &v)?))
        }
        [Node::Block(Token::Function(f), inner)] if f.eq_ignore_ascii_case("hsl") || f.eq_ignore_ascii_case("hsla") => {
            let v = args(inner);
            if !(3..=4).contains(&v.len()) {
                return None;
            }
            let (h, s, l) = (v[0].rem_euclid(360.) / 60., (v[1] / 100.).clamp(0., 1.), (v[2] / 100.).clamp(0., 1.));
            let c = (1. - (2. * l - 1.).abs()) * s;
            let x = c * (1. - (h % 2. - 1.).abs());
            let (r, g, b) = match h as u32 {
                0 => (c, x, 0.),
                1 => (x, c, 0.),
                2 => (0., c, x),
                3 => (0., x, c),
                4 => (x, 0., c),
                _ => (c, 0., x),
            };
            let m = l - c / 2.;
            Some(pack((r + m) * 255., (g + m) * 255., (b + m) * 255., alpha(inner, &v)?))
        }
        _ => None,
    }
}

fn gradient(value: &[Node]) -> Option<Gradient> {
    if let Some(c) = color(value) {
        return Some(Gradient { angle: 180., from: c, to: c });
    }
    let [Node::Block(Token::Function(f), inner)] = value else { return None };
    if !f.eq_ignore_ascii_case("linear-gradient") {
        return None;
    }
    match split(inner, |t| matches!(t, Token::Comma)).as_slice() {
        [angle, from, to] => Some(Gradient { angle: deg(angle)?, from: color(from)?, to: color(to)? }),
        [from, to] => Some(Gradient { angle: 180., from: color(from)?, to: color(to)? }),
        _ => None,
    }
}

fn deg(value: &[Node]) -> Option<f32> {
    match value {
        [Node::Tok(Token::Dimension { value, unit, .. })] if unit.eq_ignore_ascii_case("deg") && value.is_finite() => Some(value.rem_euclid(360.)),
        _ => None,
    }
}

fn host_patterns(prelude: &[Node]) -> Option<Vec<HostPattern>> {
    let mut out = Vec::new();
    for part in split(prelude, |t| matches!(t, Token::Comma)) {
        let mut raw = String::new();
        for n in part {
            match n {
                Node::Tok(Token::QuotedString(s)) if part.len() == 1 => raw.push_str(s),
                Node::Tok(Token::Ident(s)) => raw.push_str(s),
                Node::Tok(Token::Number { int_value: Some(i), has_sign: false, .. }) => raw.push_str(&i.to_string()),
                Node::Tok(Token::Dimension { int_value: Some(i), has_sign: false, unit, .. }) => {
                    raw.push_str(&i.to_string());
                    raw.push_str(unit);
                }
                Node::Tok(Token::Delim(c @ ('.' | '*' | '-'))) => raw.push(*c),
                _ => return None,
            }
        }
        let raw = raw.to_ascii_lowercase();
        let valid = |h: &str| {
            !h.is_empty()
                && h.len() <= 253
                && h.split('.').all(|l| {
                    !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        };
        // A public suffix (`com`, `co.uk`, `github.io`) would reach countless sites without the
        // limits of `*`.
        let site = |h: &str| valid(h) && psl::suffix_str(h) != Some(h);
        out.push(match raw.strip_prefix("*.") {
            _ if raw == "*" => HostPattern::Any,
            Some(h) if site(h) => HostPattern::Suffix(h.to_owned()),
            None if site(&raw) => HostPattern::Exact(raw),
            _ => return None,
        });
    }
    (!out.is_empty()).then_some(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    Cosmetic,
    Full,
}

const COSMETIC: [&str; 14] = [
    "color",
    "background-color",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "outline-color",
    "accent-color",
    "caret-color",
    "text-decoration-color",
    "font-family",
    "scrollbar-color",
    "column-rule-color",
];

/// Value functions that compute a value from their arguments and fetch nothing.
const VALUE_FUNCTIONS: [&str; 19] = [
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "oklab",
    "oklch",
    "color",
    "color-mix",
    "calc",
    "min",
    "max",
    "clamp",
    "var",
    "linear-gradient",
    "radial-gradient",
    "conic-gradient",
];

const SELECTOR_FUNCTIONS: [&str; 10] = ["not", "is", "where", "has", "nth-child", "nth-last-child", "nth-of-type", "nth-last-of-type", "lang", "dir"];

struct Sanitizer<'w> {
    tier: Tier,
    any: bool,
    rules: &'w mut usize,
    warnings: &'w mut Vec<String>,
}

impl Sanitizer<'_> {
    fn refuse(&mut self, what: &str) {
        self.warnings.push(format!("Page styles: {what} was left out."));
    }

    fn rules(&mut self, nodes: &[Node], out: &mut String, depth: u8) {
        for item in items(nodes) {
            let Item::Rule(prelude, body) = item else {
                self.refuse("a statement such as @import");
                continue;
            };
            if *self.rules >= MAX_RULES {
                self.refuse(&format!("everything after {MAX_RULES} rules"));
                return;
            }
            match prelude.first() {
                Some(Node::Tok(Token::AtKeyword(k))) => self.at_rule(&k.to_ascii_lowercase(), trim(&prelude[1..]), body, out, depth),
                _ => self.style_rule(prelude, body, out),
            }
        }
    }

    fn at_rule(&mut self, name: &str, prelude: &[Node], body: &[Node], out: &mut String, depth: u8) {
        let ok = match name {
            "media" | "supports" => depth < 3 && self.condition(prelude),
            "keyframes" => !self.any && matches!(prelude, [Node::Tok(Token::Ident(_) | Token::QuotedString(_))]),
            _ => false,
        };
        if !ok {
            self.refuse(&format!("@{name}"));
            return;
        }
        let mut inner = String::new();
        if name == "keyframes" {
            for item in items(body) {
                match item {
                    Item::Rule(stops, decls) if self.keyframe_selector(stops) => self.block(stops, decls, &mut inner),
                    _ => self.refuse("a keyframe"),
                }
            }
        } else {
            self.rules(body, &mut inner, depth + 1);
        }
        if inner.is_empty() {
            return;
        }
        *self.rules += 1;
        out.push('@');
        out.push_str(name);
        out.push(' ');
        write(prelude, out);
        out.push('{');
        out.push_str(&inner);
        out.push('}');
    }

    fn condition(&self, nodes: &[Node]) -> bool {
        nodes.iter().all(|n| match n {
            Node::Tok(t) => matches!(
                t,
                Token::Ident(_)
                    | Token::Number { .. }
                    | Token::Dimension { .. }
                    | Token::Percentage { .. }
                    | Token::Colon
                    | Token::Comma
                    | Token::WhiteSpace(_)
                    | Token::Comment(_)
                    | Token::Delim('<' | '>' | '=' | '/')
            ),
            Node::Block(Token::ParenthesisBlock, inner) => self.condition(inner),
            _ => false,
        })
    }

    fn keyframe_selector(&self, nodes: &[Node]) -> bool {
        !nodes.is_empty() && nodes.iter().all(|n| matches!(n, Node::Tok(Token::Percentage { .. } | Token::Ident(_) | Token::Comma | Token::WhiteSpace(_))))
    }

    fn style_rule(&mut self, prelude: &[Node], body: &[Node], out: &mut String) {
        if prelude.is_empty() || !self.selector(prelude, 0) {
            self.refuse("a rule with a selector Tarn does not allow");
            return;
        }
        self.block(prelude, body, out);
    }

    fn block(&mut self, prelude: &[Node], body: &[Node], out: &mut String) {
        let mut decls = String::new();
        for decl in declarations(body) {
            match decl {
                Some((name, value)) if self.declaration(&name, value) => {
                    let _ = cssparser::serialize_identifier(&name, &mut decls);
                    decls.push(':');
                    write(value, &mut decls);
                    decls.push(';');
                }
                Some((name, _)) => self.refuse(&format!("\"{name}\"")),
                None => self.refuse("a nested rule or malformed declaration"),
            }
        }
        if decls.is_empty() {
            return;
        }
        *self.rules += 1;
        write(prelude, out);
        out.push('{');
        out.push_str(&decls);
        out.push('}');
    }

    fn selector(&mut self, nodes: &[Node], has_depth: u8) -> bool {
        nodes.iter().all(|n| match n {
            Node::Tok(t) => matches!(
                t,
                Token::Ident(_)
                    | Token::Hash(_)
                    | Token::IDHash(_)
                    | Token::Colon
                    | Token::Comma
                    | Token::WhiteSpace(_)
                    | Token::Comment(_)
                    | Token::Number { .. }
                    | Token::Dimension { .. }
                    | Token::Delim('.' | '*' | '>' | '+' | '~' | '|' | '-')
            ),
            Node::Block(Token::SquareBracketBlock, inner) => inner.iter().all(|n| {
                matches!(
                    n,
                    Node::Tok(
                        Token::Ident(_)
                            | Token::QuotedString(_)
                            | Token::WhiteSpace(_)
                            | Token::Delim('=' | '|')
                            | Token::IncludeMatch
                            | Token::DashMatch
                            | Token::PrefixMatch
                            | Token::SuffixMatch
                            | Token::SubstringMatch
                    )
                )
            }),
            Node::Block(Token::Function(f), inner) => {
                let f = f.to_ascii_lowercase();
                let depth = has_depth + u8::from(f == "has");
                // `:has` makes a style recalculation costly; limited sheets may run on every site.
                let allowed = f != "has" || self.tier == Tier::Full;
                SELECTOR_FUNCTIONS.contains(&f.as_str()) && allowed && depth <= 1 && self.selector(inner, depth)
            }
            _ => false,
        })
    }

    fn declaration(&self, name: &str, value: &[Node]) -> bool {
        let property_ok = match self.tier {
            Tier::Full => !name.starts_with("-webkit-binding") && name != "behavior",
            Tier::Cosmetic => COSMETIC.contains(&name),
        };
        property_ok && visible_text(name, strip_important(value)) && !value.is_empty() && self.value(value)
    }

    fn value(&self, nodes: &[Node]) -> bool {
        nodes.iter().all(|n| match n {
            Node::Tok(t) => matches!(
                t,
                Token::Ident(_)
                    | Token::Number { .. }
                    | Token::Percentage { .. }
                    | Token::Dimension { .. }
                    | Token::Hash(_)
                    | Token::IDHash(_)
                    | Token::QuotedString(_)
                    | Token::Comma
                    | Token::WhiteSpace(_)
                    | Token::Comment(_)
                    | Token::Delim('/' | '+' | '-' | '*' | '!' | '.' | '%')
            ),
            Node::Block(Token::Function(f), inner) => {
                let f = f.to_ascii_lowercase();
                VALUE_FUNCTIONS.contains(&f.as_str()) && !(self.tier == Tier::Cosmetic && f == "var") && self.value(inner)
            }
            Node::Block(Token::ParenthesisBlock, inner) => self.value(inner),
            _ => false,
        })
    }
}

/// A text colour must be one Tarn can read and at least 60 % opaque (a hex, numeric `rgb`/`hsl` or
/// CSS colour name), or `currentcolor` / a CSS-wide keyword. Anything else (system colours,
/// `var()`, `color-mix()`, `oklch()` and the like, whose alpha cannot be checked) is refused.
fn visible_text(name: &str, value: &[Node]) -> bool {
    if name != "color" && name != "-webkit-text-fill-color" {
        return true;
    }
    const KEYWORDS: [&str; 6] = ["currentcolor", "inherit", "initial", "unset", "revert", "revert-layer"];
    match value {
        // System colours (`-apple-system-*`, `Canvas`) can be translucent or match the page.
        [Node::Tok(Token::Ident(i))] if KEYWORDS.iter().any(|k| i.eq_ignore_ascii_case(k)) => true,
        _ => color(value).is_some_and(|c| (c & 0xff) >= 153),
    }
}

fn strip_important<'a, 'i>(value: &'a [Node<'i>]) -> &'a [Node<'i>] {
    match value.iter().position(|n| matches!(n, Node::Tok(Token::Delim('!')))) {
        Some(i) => trim(&value[..i]),
        None => value,
    }
}

fn channel(v: u32) -> f32 {
    let c = v as f32 / 255.;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn luminance(rgb: u32) -> f32 {
    0.2126 * channel(rgb >> 24 & 0xff) + 0.7152 * channel(rgb >> 16 & 0xff) + 0.0722 * channel(rgb >> 8 & 0xff)
}

/// `fg` drawn over `bg`, both `0xRRGGBBAA`; the result is opaque.
pub fn over(fg: u32, bg: u32) -> u32 {
    let a = (fg & 0xff) as f32 / 255.;
    let mix = |shift: u32| ((fg >> shift & 0xff) as f32 * a + (bg >> shift & 0xff) as f32 * (1. - a)).round() as u32;
    (mix(24) << 24) | (mix(16) << 16) | (mix(8) << 8) | 0xff
}

/// WCAG contrast ratio of two colours; `fg`'s alpha is composited over `bg`.
pub fn contrast(fg: u32, bg: u32) -> f32 {
    let bg = bg | 0xff;
    let (a, b) = (luminance(over(fg, bg)), luminance(bg));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The label colour on `fill` (`0xRRGGBBAA`) drawn over `behind`, such as a primary button in a
/// theme's accent, which may be translucent: white as on macOS while it keeps 3:1 there,
/// otherwise the light scheme's text.
pub fn on_fill(fill: u32, behind: u32) -> u32 {
    const WHITE: u32 = 0xf4f9fdff;
    let fill = over(fill, behind | 0xff);
    if contrast(WHITE, fill) >= 3. { WHITE } else { Tokens::LIGHT.text }
}

/// Text colours and the contrast each needs on every background: body text, secondary text, the
/// space name and placeholders.
const FLOORS: [(&str, f32); 4] = [("text", 4.5), ("muted", 4.5), ("space-label", 3.), ("faint", 4.5)];

/// The accent's contrast floor where it is drawn as text or a focus ring (`Tokens::accent_text`).
pub const ACCENT_TEXT_FLOOR: f32 = 4.5;

fn text_slot<'a>(t: &'a mut Tokens, name: &str) -> &'a mut u32 {
    match name {
        "text" => &mut t.text,
        "muted" => &mut t.muted,
        "space-label" => &mut t.space_label,
        _ => &mut t.faint,
    }
}

/// `fg` itself when it reaches `floor` on every one of `backgrounds`, else the first step towards
/// white, then towards black, that does; text alpha stays at least 0.6.
fn readable(fg: u32, backgrounds: &[u32], floor: f32) -> Option<u32> {
    let passes = |c: u32| backgrounds.iter().all(|bg| contrast(c, *bg) >= floor);
    let toward = |target: u32| {
        (1..=20).map(move |step| {
            let k = step as f32 / 20.;
            let mix = |shift: u32| ((fg >> shift & 0xff) as f32 * (1. - k) + (target >> shift & 0xff) as f32 * k).round() as u32;
            (mix(24) << 24) | (mix(16) << 16) | (mix(8) << 8) | (fg & 0xff).max(153)
        })
    };
    std::iter::once(fg).chain(toward(0xffffff00)).chain(toward(0)).find(|c| passes(*c))
}

impl Tokens {
    /// Every background text sits on, over each end of the backdrop drawn opaque and over the
    /// black an opaque window clears to.
    fn text_backgrounds(&self) -> Vec<u32> {
        self.text_backgrounds_over(&[0x000000ff])
    }

    fn text_backgrounds_over(&self, desktops: &[u32]) -> Vec<u32> {
        let layers =
            [self.page, self.panel, self.panel_nav, self.panel_row, self.surface, self.sidebar.from, self.sidebar.to, self.backdrop.from, self.backdrop.to];
        let ends = [self.backdrop.from, self.backdrop.to];
        let mut bases: Vec<u32> = ends.iter().map(|e| e | 0xff).chain(ends.iter().flat_map(|e| desktops.iter().map(move |d| over(*e, *d)))).collect();
        bases.sort_unstable();
        bases.dedup();
        bases.into_iter().flat_map(|base| layers.map(|bg| over(bg, base))).collect()
    }

    pub fn translucent(&self) -> bool {
        self.backdrop.from & 0xff < 0xff || self.backdrop.to & 0xff < 0xff
    }

    /// The accent where it is text or a focus ring: the accent itself while it keeps
    /// `ACCENT_TEXT_FLOOR` on every background, otherwise lightened or darkened until it does.
    pub fn accent_text(&self) -> u32 {
        readable(self.accent | 0xff, &self.text_backgrounds(), ACCENT_TEXT_FLOOR).unwrap_or(self.text)
    }
}

/// Moves each text colour towards white or black until it reaches its floor on every background
/// it sits on. When no colour can (light and dark backgrounds mixed), the text and background
/// colours go back to the scheme's defaults. Returns what it changed.
fn enforce_contrast(t: &mut Tokens, scheme: Scheme) -> Vec<String> {
    let backgrounds = t.text_backgrounds();
    let mut adjusted = Vec::new();
    for (name, floor) in FLOORS {
        let slot = text_slot(t, name);
        let original = *slot;
        match readable(original, &backgrounds, floor) {
            Some(c) if c == original => {}
            Some(c) => {
                *slot = c;
                adjusted.push(name.to_owned());
            }
            None => {
                let d = Tokens::of(scheme);
                (t.page, t.panel, t.panel_nav, t.panel_row, t.surface, t.sidebar, t.backdrop) =
                    (d.page, d.panel, d.panel_nav, d.panel_row, d.surface, d.sidebar, d.backdrop);
                (t.text, t.muted, t.space_label, t.faint) = (d.text, d.muted, d.space_label, d.faint);
                return vec!["text and backgrounds".to_owned()];
            }
        }
    }
    adjusted
}

impl std::fmt::Display for HostPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostPattern::Any => f.write_str("*"),
            HostPattern::Exact(h) => f.write_str(h),
            HostPattern::Suffix(h) => write!(f, "*.{h}"),
        }
    }
}

/// A theme file for `parsed`: the `@tarn-theme` header, the `:root` tokens and the page styles as
/// they were accepted. The colour editor writes through it, so a theme's text is regenerated.
pub fn to_css(parsed: &Parsed) -> String {
    let mut out = tokens_css(&parsed.meta, &parsed.tokens, parsed.font.as_deref());
    for sheet in &parsed.sites {
        let hosts: Vec<String> = sheet.hosts.iter().map(ToString::to_string).collect();
        out.push_str(&format!("\n@tarn-site {} {{\n  {}\n}}\n", hosts.join(", "), sheet.css));
    }
    out
}

fn tokens_css(meta: &Meta, tokens: &Tokens, font: Option<&str>) -> String {
    let mut out = String::from("@tarn-theme {");
    for (k, v) in [("name", &meta.name), ("author", &meta.author), ("version", &meta.version)] {
        if !v.is_empty() {
            out.push_str(&format!(" {k}: "));
            let _ = cssparser::serialize_string(v, &mut out);
            out.push(';');
        }
    }
    out.push_str(&format!(" scheme: {}; }}\n\n:root {{\n", if meta.scheme == Scheme::Light { "light" } else { "dark" }));
    let hex = |c: u32| if c & 0xff == 0xff { format!("#{:06x}", c >> 8) } else { format!("#{c:08x}") };
    for (name, g) in [("backdrop", tokens.backdrop), ("sidebar", tokens.sidebar)] {
        out.push_str(&format!("  --tarn-{name}: linear-gradient({}deg, {}, {});\n", g.angle, hex(g.from), hex(g.to)));
    }
    let mut copy = *tokens;
    for (name, slot) in COLORS {
        out.push_str(&format!("  --tarn-{name}: {};\n", hex(*slot(&mut copy))));
    }
    for (name, r) in RADII.iter().zip(tokens.radius) {
        out.push_str(&format!("  --tarn-radius-{name}: {r}px;\n"));
    }
    out.push_str(&format!("  --tarn-shadow: {};\n", tokens.shadow));
    let fog = match tokens.fog_mode {
        Fog::On => "on",
        Fog::Still => "still",
        Fog::Off => "off",
    };
    out.push_str(&format!("  --tarn-fog: {fog};\n"));
    out.push_str(&format!("  --tarn-search-effect: {};\n", tokens.search_effect.name()));
    if let Some(c) = tokens.search_color {
        out.push_str(&format!("  --tarn-search-color: {};\n", hex(c)));
    }
    if let Some(f) = font {
        out.push_str("  --tarn-font: ");
        let _ = cssparser::serialize_string(f, &mut out);
        out.push_str(";\n");
    }
    out.push_str("}\n");
    out
}

/// Themes compiled into the app: id, CSS. The first is the default dark theme, the second the
/// default light one.
pub const BUILT_IN: [(&str, &str); 7] = [
    ("tarn", include_str!("../themes/tarn.css")),
    ("tarn-light", include_str!("../themes/tarn-light.css")),
    ("graphite", include_str!("../themes/graphite.css")),
    ("high-contrast", include_str!("../themes/high-contrast.css")),
    ("ember", include_str!("../themes/ember.css")),
    ("glass", include_str!("../themes/glass.css")),
    ("glass-light", include_str!("../themes/glass-light.css")),
];

pub const DARK_SETTING: &str = "theme.dark";
pub const LIGHT_SETTING: &str = "theme.light";
pub const MATCH_SYSTEM_SETTING: &str = "theme.match_system";

/// A theme the user can pick: compiled in, or a row of the `themes` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeRef {
    BuiltIn(&'static str),
    Stored(Id),
}

impl ThemeRef {
    pub const DARK: ThemeRef = ThemeRef::BuiltIn(BUILT_IN[0].0);
    pub const LIGHT: ThemeRef = ThemeRef::BuiltIn(BUILT_IN[1].0);

    pub fn parse(s: &str) -> Option<ThemeRef> {
        match s.split_once(':')? {
            ("builtin", id) => BUILT_IN.iter().find(|(b, _)| *b == id).map(|(b, _)| ThemeRef::BuiltIn(b)),
            ("theme", id) => id.parse().ok().map(ThemeRef::Stored),
            _ => None,
        }
    }

    pub fn key(&self) -> String {
        match self {
            ThemeRef::BuiltIn(id) => format!("builtin:{id}"),
            ThemeRef::Stored(id) => format!("theme:{id}"),
        }
    }

    fn fallback(scheme: &str) -> ThemeRef {
        if scheme == "light" { ThemeRef::LIGHT } else { ThemeRef::DARK }
    }
}

/// Which theme shows in dark and in light appearance, and whether macOS's appearance picks
/// between them; off by default, so the dark theme always shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub dark: ThemeRef,
    pub light: ThemeRef,
    pub match_system: bool,
}

impl Selection {
    pub fn load(store: &Store) -> Selection {
        let get = |key: &str, default: ThemeRef| store.setting(key).ok().flatten().and_then(|v| ThemeRef::parse(&v)).unwrap_or(default);
        Selection {
            dark: get(DARK_SETTING, ThemeRef::DARK),
            light: get(LIGHT_SETTING, ThemeRef::LIGHT),
            match_system: store.setting(MATCH_SYSTEM_SETTING).ok().flatten().as_deref() == Some("on"),
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), store::Error> {
        store.set_setting(DARK_SETTING, &self.dark.key())?;
        store.set_setting(LIGHT_SETTING, &self.light.key())?;
        store.set_setting(MATCH_SYSTEM_SETTING, if self.match_system { "on" } else { "off" })
    }

    /// Turns Match system on or off. With it off every pick lands in the dark slot, so turning it
    /// on moves a light theme found there, whose scheme is `dark_scheme`, to the light slot.
    pub fn set_match_system(&mut self, on: bool, dark_scheme: Scheme) {
        if on && !self.match_system && dark_scheme == Scheme::Light {
            self.light = std::mem::replace(&mut self.dark, ThemeRef::DARK);
        }
        self.match_system = on;
    }

    /// The theme to show, given whether macOS is in light appearance.
    pub fn active(&self, system_light: bool) -> &ThemeRef {
        if self.match_system && system_light { &self.light } else { &self.dark }
    }
}

/// How much a stored theme's `source` is trusted.
pub fn source_of(stored: &str) -> Source {
    match stored {
        "yours" => Source::Yours,
        s if s.starts_with("official:") => Source::Official,
        _ => Source::Unverified,
    }
}

/// The theme in use and why it may differ from the one chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub id: ThemeRef,
    pub parsed: Parsed,
    pub source: Source,
    /// Whether its page styles may apply at all (per-profile switches come on top).
    pub page_css: bool,
    /// Set when the chosen theme could not be used and a built-in one shows instead.
    pub notice: Option<String>,
}

fn built_in(id: &ThemeRef) -> Option<Active> {
    let ThemeRef::BuiltIn(name) = id else { return None };
    let css = BUILT_IN.iter().find(|(b, _)| b == name)?.1;
    let parsed = parse(css, Source::BuiltIn).ok()?;
    Some(Active { id: id.clone(), parsed, source: Source::BuiltIn, page_css: true, notice: None })
}

/// Loads `wanted`. A missing, unreadable or revoked stored theme falls back to the built-in theme
/// of its scheme with a notice, and its row is kept.
/// `withdrawn` gets a stored theme's file hash and GitHub repository id (0 for none).
pub fn resolve(store: &Store, wanted: &ThemeRef, withdrawn: &dyn Fn(&str, i64) -> bool) -> Active {
    let fallback = |scheme: &str, notice: String| Active { notice: Some(notice), ..built_in(&ThemeRef::fallback(scheme)).expect("built-in themes parse") };
    match wanted {
        ThemeRef::BuiltIn(_) => built_in(wanted).unwrap_or_else(|| fallback("dark", "The chosen theme is not part of this version of Tarn.".to_owned())),
        ThemeRef::Stored(id) => match store.theme(*id) {
            Ok(Some(row)) => load_row(&row).unwrap_or_else(|why| fallback(&row.scheme, why)).revoked_if(withdrawn(&row.sha256, row.repo_id), || {
                fallback(&row.scheme, format!("\u{201C}{}\u{201D} was withdrawn as unsafe and is no longer used.", row.name))
            }),
            _ => fallback("dark", "The chosen theme was removed.".to_owned()),
        },
    }
}

fn load_row(row: &InstalledTheme) -> Result<Active, String> {
    let parsed = parse(&row.css, source_of(&row.source)).map_err(|e| format!("\u{201C}{}\u{201D} could not be used: {e}", row.name))?;
    Ok(Active { id: ThemeRef::Stored(row.id), parsed, source: source_of(&row.source), page_css: row.page_css, notice: None })
}

/// The setting that turns theme styles on websites off for one profile (`"off"`).
pub fn pages_setting(profile_id: Id) -> String {
    format!("theme_pages.{profile_id}")
}

/// The website sheets of `active` for the style script, as (host patterns joined by `,`, CSS):
/// every-site sheets first, each sheet once, in total at most `store::MAX_SITE_CSS`. Empty unless
/// `allowed`: the caller passes false in safe mode, in private profiles and when the profile
/// turned theme styles off.
pub fn page_sheets(active: &Active, allowed: bool) -> Vec<(String, String)> {
    if !allowed || !active.page_css {
        return Vec::new();
    }
    let (any, hosts): (Vec<_>, Vec<_>) = active.parsed.sites.iter().partition(|s| s.hosts.contains(&HostPattern::Any));
    let mut total = 0;
    let mut out = Vec::new();
    for sheet in any.into_iter().chain(hosts) {
        total += sheet.css.len();
        if total > store::MAX_SITE_CSS {
            break;
        }
        let patterns: Vec<String> = sheet.hosts.iter().map(ToString::to_string).collect();
        out.push((patterns.join(","), sheet.css.clone()));
    }
    out
}

/// A theme's text, as stored or compiled in.
pub fn css_of(store: &Store, id: &ThemeRef) -> Option<String> {
    match id {
        ThemeRef::BuiltIn(name) => BUILT_IN.iter().find(|(b, _)| b == name).map(|(_, css)| (*css).to_owned()),
        ThemeRef::Stored(id) => store.theme(*id).ok().flatten().map(|t| t.css),
    }
}

/// Where a downloaded theme came from; zero and empty for other sources.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    pub repo_id: i64,
    pub owner_id: i64,
    pub tag: String,
    pub sha256: String,
}

/// Keeps `css` as the theme `id` (0 for a new one) from `source`. Page styles start on unless the
/// source is unverified; an existing row keeps its choice, and its origin when `origin` is empty
/// (an edit), so a copy stays tied to the file it came from.
pub fn save(store: &Store, id: Id, source: &str, css: &str, origin: Origin, now: i64) -> Result<Id, String> {
    let parsed = parse(css, source_of(source)).map_err(|e| e.to_string())?;
    let previous = (id != 0).then(|| store.theme(id).ok().flatten()).flatten();
    let origin = match &previous {
        Some(p) if origin == Origin::default() => Origin { repo_id: p.repo_id, owner_id: p.owner_id, tag: p.tag.clone(), sha256: p.sha256.clone() },
        _ => origin,
    };
    let row = InstalledTheme {
        id,
        source: source.to_owned(),
        repo_id: origin.repo_id,
        owner_id: origin.owner_id,
        tag: origin.tag,
        name: if parsed.meta.name.is_empty() { "Untitled".to_owned() } else { parsed.meta.name },
        version: parsed.meta.version,
        scheme: if parsed.meta.scheme == Scheme::Light { "light" } else { "dark" }.to_owned(),
        css: css.to_owned(),
        sha256: origin.sha256,
        page_css: previous.map_or(source_of(source) != Source::Unverified, |p| p.page_css),
        installed_at: now,
    };
    store.put_theme(&row).map_err(|e| e.to_string())
}

/// A copy of `id` the user may edit. An unverified theme's page styles are left out, since the
/// copy would be trusted as the user's own. The copy keeps the original's origin (repository and
/// file hash), so withdrawing the original withdraws the copy, and its website-styles choice.
pub fn duplicate(store: &Store, id: &ThemeRef, now: i64) -> Result<Id, String> {
    let css = css_of(store, id).ok_or("The theme is gone.")?;
    let original = match id {
        ThemeRef::BuiltIn(_) => None,
        ThemeRef::Stored(id) => store.theme(*id).ok().flatten(),
    };
    let source = match (id, &original) {
        (ThemeRef::BuiltIn(_), _) => Source::BuiltIn,
        (_, Some(t)) => source_of(&t.source),
        (_, None) => Source::Unverified,
    };
    let mut parsed = parse_as_written(&css, source).map_err(|e| e.to_string())?;
    if source == Source::Unverified {
        parsed.sites.clear();
    }
    parsed.meta.name = format!("{} Copy", if parsed.meta.name.is_empty() { "Theme" } else { &parsed.meta.name });
    parsed.meta.name.truncate(parsed.meta.name.char_indices().nth(MAX_TEXT).map_or(parsed.meta.name.len(), |(i, _)| i));
    let origin =
        original.as_ref().map_or_else(Origin::default, |t| Origin { repo_id: t.repo_id, owner_id: t.owner_id, tag: t.tag.clone(), sha256: t.sha256.clone() });
    let copy = save(store, 0, "yours", &to_css(&parsed), origin, now)?;
    if let (Some(original), Ok(Some(mut row))) = (&original, store.theme(copy)) {
        row.page_css = original.page_css;
        store.put_theme(&row).map_err(|e| e.to_string())?;
    }
    Ok(copy)
}

/// Deletes a stored theme; a dark or light choice that named it goes back to the default.
pub fn remove(store: &Store, id: Id) -> Result<(), store::Error> {
    store.delete_theme(id)?;
    let mut selection = Selection::load(store);
    if selection.dark == ThemeRef::Stored(id) {
        selection.dark = ThemeRef::DARK;
    }
    if selection.light == ThemeRef::Stored(id) {
        selection.light = ThemeRef::LIGHT;
    }
    selection.save(store)
}

/// Sets one colour token of the user's theme `id` and rewrites its file.
pub fn set_color(store: &Store, id: Id, token: &str, value: &str, now: i64) -> Result<(), String> {
    let row = store.theme(id).ok().flatten().filter(|t| t.source == "yours").ok_or("Only your own themes can be edited.")?;
    let mut parsed = parse_as_written(&row.css, Source::Yours).map_err(|e| e.to_string())?;
    let mut p = Parser::new(value.trim());
    let nodes = tree(&mut p).map_err(|_| "Not a colour.")?;
    let color = color(trim(&nodes)).ok_or("Use a colour such as #1f75b0 or #1f75b080.")?;
    match token {
        "backdrop-from" => parsed.tokens.backdrop.from = color,
        "backdrop-to" => parsed.tokens.backdrop.to = color,
        "sidebar-from" => parsed.tokens.sidebar.from = color,
        "sidebar-to" => parsed.tokens.sidebar.to = color,
        "search-color" => parsed.tokens.search_color = Some(color),
        name => *COLORS.iter().find(|(n, _)| *n == name).ok_or("Not a theme colour.")?.1(&mut parsed.tokens) = color,
    }
    save(store, id, "yours", &to_css(&parsed), Origin::default(), now).map(drop)
}

/// The colours the colour editor lists: name and `0xRRGGBBAA`.
pub fn editable_colors(tokens: &Tokens) -> Vec<(&'static str, u32)> {
    let mut copy = *tokens;
    let mut out = vec![
        ("backdrop-from", tokens.backdrop.from),
        ("backdrop-to", tokens.backdrop.to),
        ("sidebar-from", tokens.sidebar.from),
        ("sidebar-to", tokens.sidebar.to),
    ];
    out.extend(COLORS.iter().map(|(name, slot)| (*name, *slot(&mut copy))));
    out.push(("search-color", tokens.search_color()));
    out
}

impl Active {
    fn revoked_if(self, revoked: bool, fallback: impl FnOnce() -> Active) -> Active {
        if revoked { fallback() } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_on_a_fill_keep_three_to_one() {
        for fill in [0x1f75b0ff, 0x0a7affff, 0xa550a7ff, 0xf74f9eff, 0xff5257ff, 0xf7821bff, 0xd9a400ff, 0x62ba46ff, 0x8c8c8cff, 0xd9ecfaff, 0xffffffff] {
            assert!(contrast(on_fill(fill, 0), fill) >= 3., "{fill:08x}");
        }
        assert_eq!(on_fill(0x1f75b0ff, 0), 0xf4f9fdff, "the default accent keeps white");
        assert_eq!(on_fill(0xd9a400ff, 0), Tokens::LIGHT.text);
        assert_eq!(on_fill(0xffffff00, Tokens::DARK.surface), 0xf4f9fdff, "a clear fill shows the surface behind it");
        assert_eq!(on_fill(0xffffff00, Tokens::LIGHT.surface), Tokens::LIGHT.text);
    }

    #[test]
    fn the_authoring_reference_lists_every_token() {
        let reference = include_str!("../../../docs/themes.md");
        let reference = &reference[reference.find("## Authoring reference").unwrap()..];
        let names = COLORS
            .iter()
            .map(|(n, _)| (*n).to_owned())
            .chain(RADII.iter().map(|r| format!("radius-{r}")))
            .chain(["backdrop", "sidebar", "shadow", "fog", "font", "search-effect", "search-color"].map(String::from));
        for name in names {
            assert!(reference.contains(&format!("`--tarn-{name}`")), "--tarn-{name} is missing from the authoring reference");
        }
        for name in FIXED {
            assert!(reference.contains(&format!("`--tarn-{name}`")));
        }
    }

    #[test]
    fn built_in_themes_parse_cleanly_and_tarn_is_todays_look() {
        for (id, css) in BUILT_IN {
            let p = parse(css, Source::BuiltIn).unwrap();
            assert!(p.warnings.is_empty() && p.adjusted.is_empty(), "{id}: {:?} {:?}", p.warnings, p.adjusted);
            assert!(!p.meta.name.is_empty(), "{id}");
            assert!(contrast(0xf4f9fdff, p.tokens.accent) >= 4.5, "white on the accent of {id}");
        }
        assert_eq!(parse(BUILT_IN[0].1, Source::BuiltIn).unwrap().tokens, Tokens::DARK);
        let light = parse(BUILT_IN[1].1, Source::BuiltIn).unwrap();
        assert_eq!((light.tokens, light.meta.scheme), (Tokens::LIGHT, Scheme::Light));
        assert!(!Tokens::DARK.translucent() && !Tokens::LIGHT.translucent());
        for id in ["glass", "glass-light"] {
            let css = BUILT_IN.iter().find(|(b, _)| *b == id).unwrap().1;
            let mut t = parse(css, Source::BuiltIn).unwrap().tokens;
            assert!(t.translucent(), "{id}");
            let backgrounds = t.text_backgrounds_over(&[0xffffffff, 0x000000ff]);
            for (name, floor) in FLOORS {
                let text = *text_slot(&mut t, name);
                for bg in &backgrounds {
                    assert!(contrast(text, *bg) >= floor, "{id} {name} on {bg:08x} over any desktop");
                }
            }
        }
    }

    #[test]
    fn accent_text_is_readable_on_every_background_with_any_accent() {
        for (id, css) in BUILT_IN {
            let tokens = parse(css, Source::BuiltIn).unwrap().tokens;
            for accent in std::iter::once(tokens.accent).chain(crate::prefs::ACCENTS.map(|(_, c)| c << 8 | 0xff)) {
                let t = Tokens { accent, ..tokens };
                let text = t.accent_text();
                for bg in t.text_backgrounds() {
                    assert!(contrast(text, bg) >= ACCENT_TEXT_FLOOR, "{id}: {accent:08x} as {text:08x} on {bg:08x}");
                }
            }
        }
        let t = Tokens { accent: 0xf4f9fdff, ..Tokens::DARK };
        assert_eq!(t.accent_text(), 0xf4f9fdff, "an accent that already reads stays as it is");
    }

    #[test]
    fn match_system_moves_a_light_pick_to_the_light_slot() {
        let mut s = Selection { dark: ThemeRef::Stored(3), light: ThemeRef::LIGHT, match_system: false };
        s.set_match_system(true, Scheme::Light);
        assert_eq!(s, Selection { dark: ThemeRef::DARK, light: ThemeRef::Stored(3), match_system: true });
        let mut s = Selection { dark: ThemeRef::Stored(4), light: ThemeRef::LIGHT, match_system: false };
        s.set_match_system(true, Scheme::Dark);
        assert_eq!(s, Selection { dark: ThemeRef::Stored(4), light: ThemeRef::LIGHT, match_system: true });
        s.set_match_system(false, Scheme::Light);
        assert_eq!(s.dark, ThemeRef::Stored(4));
        assert!(!s.match_system);
    }

    #[test]
    fn selection_defaults_to_tarn_and_follows_the_system_only_when_asked() {
        let store = Store::open_in_memory().unwrap();
        let mut s = Selection::load(&store);
        assert_eq!(s, Selection { dark: ThemeRef::DARK, light: ThemeRef::LIGHT, match_system: false });
        assert_eq!(s.active(true), &ThemeRef::DARK, "Match system is off by default");
        s.match_system = true;
        s.dark = ThemeRef::Stored(7);
        s.save(&store).unwrap();
        let s = Selection::load(&store);
        assert_eq!((s.active(false), s.active(true)), (&ThemeRef::Stored(7), &ThemeRef::LIGHT));
        store.set_setting(DARK_SETTING, "builtin:nope").unwrap();
        assert_eq!(Selection::load(&store).dark, ThemeRef::DARK, "an unknown id falls back");
        for r in [ThemeRef::DARK, ThemeRef::BuiltIn("graphite"), ThemeRef::Stored(3)] {
            assert_eq!(ThemeRef::parse(&r.key()), Some(r));
        }
        assert_eq!(ThemeRef::parse("theme:x"), None);
    }

    #[test]
    fn duplicates_edits_and_removal() {
        let store = Store::open_in_memory().unwrap();
        let copy = duplicate(&store, &ThemeRef::BuiltIn("graphite"), 1).unwrap();
        let row = store.theme(copy).unwrap().unwrap();
        assert_eq!((row.source.as_str(), row.name.as_str(), row.page_css), ("yours", "Graphite Copy", true));
        assert_eq!(parse(&row.css, Source::Yours).unwrap().tokens, parse(BUILT_IN[2].1, Source::BuiltIn).unwrap().tokens);
        set_color(&store, copy, "accent", "#123456", 2).unwrap();
        set_color(&store, copy, "sidebar-to", "#000000aa", 2).unwrap();
        let t = parse(&store.theme(copy).unwrap().unwrap().css, Source::Yours).unwrap().tokens;
        assert_eq!((t.accent, t.sidebar.to), (0x123456ff, 0x000000aa));
        set_color(&store, copy, "page", "#ffffff", 2).unwrap();
        set_color(&store, copy, "accent", "#654321", 2).unwrap();
        let written = parse_as_written(&store.theme(copy).unwrap().unwrap().css, Source::Yours).unwrap().tokens;
        assert_eq!(
            (written.page, written.accent, written.sidebar.to),
            (0xffffffff, 0x654321ff, 0x000000aa),
            "edits keep the colours as written, not the readable fallback"
        );
        assert!(set_color(&store, copy, "accent", "url(x)", 2).is_err());
        assert!(set_color(&store, copy, "danger", "#000", 2).is_err());

        let css = "@tarn-theme { name: Got; } :root { --tarn-accent: #222 } @tarn-site a.test { a { content: 'x'; color: red } }";
        let got = save(&store, 0, "github:1:a/b", css, Origin { repo_id: 1, sha256: "s".into(), ..Default::default() }, 3).unwrap();
        assert!(!store.theme(got).unwrap().unwrap().page_css, "unverified page styles start off");
        assert!(set_color(&store, got, "accent", "#000", 3).is_err(), "only your own themes are edited");
        let copy = store.theme(duplicate(&store, &ThemeRef::Stored(got), 4).unwrap()).unwrap().unwrap();
        let copied = parse(&copy.css, Source::Yours).unwrap();
        assert!(copied.sites.is_empty(), "a copy of an unverified theme drops its page styles");
        assert_eq!(copied.tokens.accent, 0x222222ff);
        assert_eq!((copy.repo_id, copy.sha256.as_str(), copy.page_css), (1, "s", false), "a copy keeps its origin and choice");
        set_color(&store, copy.id, "accent", "#111", 5).unwrap();
        assert_eq!(store.theme(copy.id).unwrap().unwrap().sha256, "s", "an edit keeps the origin");
        assert!(resolve(&store, &ThemeRef::Stored(copy.id), &|sha, _| sha == "s").notice.is_some(), "withdrawing the original withdraws the copy");

        Selection { dark: ThemeRef::Stored(got), light: ThemeRef::Stored(got), match_system: true }.save(&store).unwrap();
        remove(&store, got).unwrap();
        assert_eq!(Selection::load(&store), Selection { dark: ThemeRef::DARK, light: ThemeRef::LIGHT, match_system: true });
        assert!(editable_colors(&Tokens::DARK).iter().any(|(n, c)| *n == "accent" && *c == 0x1f75b0ff));
    }

    #[test]
    fn stored_themes_fall_back_by_scheme_and_keep_their_row() {
        let store = Store::open_in_memory().unwrap();
        let none = |_: &str, _: i64| false;
        let row = |source: &str, css: &str, scheme: &str| InstalledTheme {
            source: source.into(),
            name: "T".into(),
            scheme: scheme.into(),
            css: css.into(),
            sha256: "abc".into(),
            installed_at: 1,
            ..Default::default()
        };
        let good = store.put_theme(&InstalledTheme { repo_id: 1, ..row("github:1:a/b", ":root { --tarn-accent: #123456 }", "dark") }).unwrap();
        let a = resolve(&store, &ThemeRef::Stored(good), &none);
        assert_eq!((a.parsed.tokens.accent, a.source, a.notice), (0x123456ff, Source::Unverified, None));
        let withdrawn = resolve(&store, &ThemeRef::Stored(good), &|sha, _| sha == "abc");
        assert_eq!(resolve(&store, &ThemeRef::Stored(good), &|_, repo| repo == 1).id, ThemeRef::DARK, "a withdrawn repository");
        assert_eq!(withdrawn.id, ThemeRef::DARK);
        assert!(withdrawn.notice.unwrap().contains("withdrawn"));
        let full = store.put_theme(&row("yours", &" ".repeat(MAX_THEME_BYTES), "light")).unwrap();
        assert_eq!(resolve(&store, &ThemeRef::Stored(full), &none).source, Source::Yours, "the store's cap is the parser's");
        let gone = resolve(&store, &ThemeRef::Stored(999), &none);
        assert_eq!((gone.id, gone.notice.is_some()), (ThemeRef::DARK, true));
        assert_eq!(source_of("official:x"), Source::Official);
        assert!(store.theme(good).unwrap().is_some(), "the row is kept");
    }

    fn page(css: &str, source: Source) -> String {
        parse(&format!("@tarn-site a.test {{ {css} }}"), source).unwrap().sites.into_iter().map(|s| s.css).collect()
    }

    #[test]
    fn an_empty_theme_is_the_dark_default() {
        let p = parse("", Source::Yours).unwrap();
        assert_eq!(p.tokens, Tokens::DARK);
        assert!(p.warnings.is_empty() && p.adjusted.is_empty() && p.sites.is_empty());
        assert_eq!(parse("@tarn-theme { scheme: light; }", Source::Yours).unwrap().tokens, Tokens::LIGHT);
    }

    #[test]
    fn the_defaults_pass_the_contrast_floor_untouched() {
        for (t, scheme) in [(Tokens::DARK, Scheme::Dark), (Tokens::LIGHT, Scheme::Light)] {
            let mut copy = t;
            assert!(enforce_contrast(&mut copy, scheme).is_empty());
        }
    }

    #[test]
    fn every_token_kind_is_read() {
        let p = parse(
            r#"@tarn-theme { name: "Glacier"; author: Quavon; version: "1.2"; }
            :root {
              --tarn-accent: #abc; --tarn-surface: rgb(10 20 30 / 50%); --tarn-page: hsl(0, 100%, 25%);
              --tarn-hover: #11223344; --tarn-backdrop: linear-gradient(90deg, #000, #111111);
              --tarn-sidebar: #222; --tarn-radius-card: 99px; --tarn-radius-small: 0;
              --tarn-shadow: 3; --tarn-fog: still; --tarn-font: "SF Pro"; --tarn-search-effect: aurora; --tarn-search-color: #00ff88;
            }"#,
            Source::Yours,
        )
        .unwrap();
        assert_eq!((p.meta.name.as_str(), p.meta.author.as_str(), p.meta.version.as_str()), ("Glacier", "Quavon", "1.2"));
        let t = p.tokens;
        assert_eq!(t.accent, 0xaabbccff);
        assert_eq!(t.surface, 0x0a141e80);
        assert_eq!(t.page, 0x800000ff);
        assert_eq!(t.hover, 0x11223344);
        assert_eq!(t.backdrop, Gradient { angle: 90., from: 0x000000ff, to: 0x111111ff });
        assert_eq!(t.sidebar, Gradient { angle: 180., from: 0x222222ff, to: 0x222222ff });
        assert_eq!((t.radius[5], t.radius[0], t.shadow, t.fog_mode), (24., 0., 1.5, Fog::Still));
        assert_eq!((t.search_effect, t.search_color), (Effect::Aurora, Some(0x00ff88ff)));
        assert_eq!(p.font.as_deref(), Some("SF Pro"));
    }

    #[test]
    fn bad_unknown_and_fixed_tokens_warn_and_keep_the_default() {
        let p = parse(":root { --tarn-accent: url(x); --tarn-nope: 1; --tarn-danger: #000; --tarn-text: #fff8; --other: 1 }", Source::Yours).unwrap();
        assert_eq!(p.tokens.accent, Tokens::DARK.accent);
        assert_eq!(p.tokens.text, Tokens::DARK.text, "text below the alpha floor is refused");
        assert_eq!(p.warnings.len(), 4, "{:?}", p.warnings);
        assert!(p.warnings.iter().any(|w| w.contains("fixed for security")));
    }

    #[test]
    fn unreadable_text_is_lifted_on_every_background_and_gradient_stop() {
        let p = parse(":root { --tarn-text: #173350; --tarn-muted: #112d48; --tarn-sidebar: linear-gradient(0deg, #000, #1a1a1a) }", Source::Yours).unwrap();
        assert_eq!(p.adjusted, ["text", "muted"]);
        let t = p.tokens;
        for bg in [t.page, t.panel, t.surface, t.sidebar.from, t.sidebar.to, t.backdrop.from, t.backdrop.to] {
            assert!(contrast(t.text, bg) >= 4.5 && contrast(t.muted, bg) >= 3.);
        }
        let light = parse("@tarn-theme { scheme: light } :root { --tarn-text: #eeeeee }", Source::Yours).unwrap();
        assert!(contrast(light.tokens.text, light.tokens.page) >= 4.5 && luminance(light.tokens.text) < 0.2, "darkened on light");
    }

    #[test]
    fn mixed_light_and_dark_backgrounds_fall_back_to_the_defaults() {
        let p = parse(
            ":root { --tarn-backdrop: #ffb340; --tarn-sidebar: #ffb340; --tarn-panel-nav: #f4f9fd; --tarn-text: #ffb341; --tarn-space-label: #ffb342 }",
            Source::Unverified,
        )
        .unwrap();
        assert_eq!(p.adjusted, ["text and backgrounds"]);
        let (t, d) = (p.tokens, Tokens::DARK);
        assert_eq!((t.text, t.backdrop, t.panel_nav, t.space_label), (d.text, d.backdrop, d.panel_nav, d.space_label));
        let label = parse(":root { --tarn-space-label: #10243a }", Source::Yours).unwrap();
        assert_eq!(label.adjusted, ["space-label"]);
        assert!(contrast(label.tokens.space_label, Tokens::DARK.sidebar.to) >= 3.);
    }

    #[test]
    fn meta_text_is_bounded_and_stripped_of_bidi_and_controls() {
        let p = parse("@tarn-theme { name: \"Tarn\u{202e}lanoiciffO\u{7}\"; author: \"".to_owned().as_str(), Source::Yours).unwrap();
        assert!(!p.meta.name.contains('\u{202e}') && !p.meta.name.contains('\u{7}'));
        let long = format!("@tarn-theme {{ name: \"{}\" }}", "x".repeat(500));
        assert_eq!(parse(&long, Source::Yours).unwrap().meta.name.len(), MAX_TEXT);
    }

    #[test]
    fn oversized_themes_fail_and_deep_nesting_is_ignored() {
        assert_eq!(parse(&" ".repeat(MAX_THEME_BYTES + 1), Source::Yours), Err(ThemeError::TooLarge));
        assert!(parse(&" ".repeat(MAX_THEME_BYTES), Source::Yours).is_ok());
        let deep = format!("@tarn-site a.test {{ a {{ color: {}red{} }} }}", "calc(".repeat(40), ")".repeat(40));
        let p = parse(&deep, Source::Yours).unwrap();
        assert!(p.sites.is_empty() && !p.warnings.is_empty());
    }

    #[test]
    fn hosts_are_exact_suffix_or_any() {
        let p = parse("@tarn-site github.com, *.example.org, 1password.com { a { color: red } } @tarn-site * { a { color: red } }", Source::Yours).unwrap();
        assert_eq!(
            p.sites[0].hosts,
            [HostPattern::Exact("github.com".into()), HostPattern::Suffix("example.org".into()), HostPattern::Exact("1password.com".into())]
        );
        assert_eq!(p.sites[1].hosts, [HostPattern::Any]);
        let quoted = parse("@tarn-site \"www.163.com\", \"*.4chan.org\", 1password.com { a { color: red } }", Source::Yours).unwrap();
        assert_eq!(
            quoted.sites[0].hosts,
            [HostPattern::Exact("www.163.com".into()), HostPattern::Suffix("4chan.org".into()), HostPattern::Exact("1password.com".into())],
            "labels starting with a digit are written in quotes"
        );
        for bad in ["a..b", "-a.test", "a_b.test", "*.", "**.a", "a test", "\"a test\"", "\"a.test\" b", "127.0.0.1", "*.com", "*.co.uk", "*.github.io", "com"]
        {
            assert!(parse(&format!("@tarn-site {bad} {{ a {{ color: red }} }}"), Source::Yours).unwrap().sites.is_empty(), "{bad}");
        }
    }

    #[test]
    fn page_css_cannot_reach_the_network() {
        for value in [
            "url(https://x.test/a)",
            "url('https://x.test/a')",
            "URL(x)",
            "\\75rl(x)",
            "u\\72l(x)",
            "image-set('a.png' 1x)",
            "-webkit-image-set(url(a) 1x)",
            "cross-fade(url(a), url(b))",
            "-webkit-cross-fade(url(a), url(b), 50%)",
            "element(#a)",
            "src('a')",
            "attr(data-x url)",
            "image(a.png)",
        ] {
            let css = page(&format!("a {{ background: {value} }}"), Source::Yours);
            assert!(css.is_empty(), "{value}: {css}");
        }
        for at in [
            "@import 'x';",
            "@\\69mport 'x';",
            "@font-face { src: local(a) }",
            "@namespace x url(y);",
            "@property --x { syntax: '*' }",
            "@counter-style x { symbols: a }",
            "@page { color: red }",
        ] {
            assert!(page(at, Source::Yours).is_empty(), "{at}");
        }
    }

    #[test]
    fn output_is_rebuilt_from_accepted_tokens_only() {
        let css = page("a/**/b, .c { color: red; background: url(x) } d { color: blue } @media (min-width: 1px) { e { color: red } }", Source::Yours);
        assert_eq!(css, "a/**/b, .c{color:red;}d{color:blue;}@media (min-width: 1px){e{color:red;}}");
        let again = parse(&format!("@tarn-site a.test {{ {css} }}"), Source::Yours).unwrap();
        assert_eq!(again.sites[0].css, css, "re-parsing the output changes nothing");
        assert!(page("a { color: red; } } b { color: red }", Source::Yours).ends_with('}'));
        assert!(page("a { content: \"\\\"}</style>\" }", Source::Yours).contains("\\\""));
    }

    #[test]
    fn tiers_limit_what_unverified_and_every_site_css_may_do() {
        let defacing = "a { content: \"x\"; display: none; color: red; position: fixed }";
        assert_eq!(page(defacing, Source::Unverified), "a{color:red;}");
        assert!(page(defacing, Source::Yours).contains("display:none"));
        assert!(page(defacing, Source::Official).contains("content:"));
        for source in [Source::Yours, Source::BuiltIn, Source::Official, Source::Unverified] {
            let any = parse(&format!("@tarn-site * {{ {defacing} }}"), source).unwrap();
            assert_eq!(any.sites[0].css, "a{color:red;}");
            assert!(parse("@tarn-site * { @keyframes k { from { color: red } } }", source).unwrap().sites.is_empty());
        }
        assert_eq!(page("a { background-color: var(--x) }", Source::Unverified), "", "no var() in cosmetic CSS");
        assert_eq!(page("a { --x: red }", Source::Unverified), "");
        assert_eq!(page("a { background-color: var(--x) }", Source::Yours), "a{background-color:var(--x);}");
    }

    #[test]
    fn see_through_text_is_refused_in_every_tier() {
        for source in [Source::Yours, Source::Unverified] {
            for v in ["transparent", "#0000", "rgba(0, 0, 0, 0.5)", "rgb(0 0 0 / 10%) !important"] {
                assert!(page(&format!("a {{ color: {v} }}"), source).is_empty(), "{v}");
            }
            assert!(!page("a { color: rgba(0, 0, 0, 0.7) }", source).is_empty());
        }
    }

    #[test]
    fn hidden_text_and_costly_selectors_are_refused() {
        for source in [Source::Yours, Source::Unverified] {
            for v in [
                "rgb(0 0 0 / calc(0))",
                "rgb(0 0 0 / none)",
                "hsl(0.5turn 0% 0% / 0)",
                "hsl(1rad 0% 0% / 0)",
                "rgb(from transparent 0 0 0)",
                "hsl(from transparent 0 0% 0%)",
                "oklch(0 0 0 / 0)",
                "color-mix(in srgb, red 0%, transparent)",
                "var(--x)",
                "TRANSPARENT",
                "-apple-system-quaternary-label",
                "-webkit-link",
                "Canvas",
                "GrayText",
            ] {
                assert!(page(&format!("a {{ color: {v} }}"), source).is_empty(), "{v}");
            }
            assert!(page("a { color: red; } b { color: currentcolor } c { color: rebeccapurple }", source).contains("currentcolor"));
            assert!(!page("a { color-scheme: dark !important; color: red }", Source::Unverified).contains("color-scheme"));
            assert!(page("a { color: rebeccapurple }", source).contains("rebeccapurple"));
        }
        assert!(page("a { -webkit-text-fill-color: transparent }", Source::Yours).is_empty());
        assert!(page(":has(a) { color: red }", Source::Unverified).is_empty(), ":has only in full sheets");
        assert!(parse("@tarn-site * { *:has(*) { color: red } }", Source::Yours).unwrap().sites.is_empty());
    }

    #[test]
    fn every_site_styles_share_one_budget() {
        let block = format!("@tarn-site * {{ {} }}\n", "a { color: red } ".repeat(900));
        let p = parse(&block.repeat(4), Source::Yours).unwrap();
        let total: usize = p.sites.iter().map(|s| s.css.len()).sum();
        assert!(total <= MAX_ANY_SITE_BYTES && p.sites.len() == 1, "{} sheets, {total} bytes", p.sites.len());
    }

    #[test]
    fn page_sheets_come_once_each_every_site_first_and_capped() {
        let sheet = |hosts: Vec<HostPattern>, css: &str| SiteSheet { hosts, css: css.into() };
        let mut active = built_in(&ThemeRef::DARK).unwrap();
        active.parsed.sites =
            vec![sheet(vec![HostPattern::Exact("a.test".into()), HostPattern::Suffix("b.test".into())], "a{}"), sheet(vec![HostPattern::Any], "b{}")];
        assert_eq!(page_sheets(&active, true), [("*".to_owned(), "b{}".to_owned()), ("a.test,*.b.test".into(), "a{}".into())]);
        assert!(page_sheets(&active, false).is_empty());
        active.page_css = false;
        assert!(page_sheets(&active, true).is_empty(), "the theme's website styles are off");
        active.page_css = true;
        let big = "x".repeat(store::MAX_SITE_CSS / 2 + 1);
        active.parsed.sites = vec![sheet(vec![HostPattern::Exact("a.test".into())], &big), sheet(vec![HostPattern::Exact("b.test".into())], &big)];
        assert_eq!(page_sheets(&active, true).len(), 1, "the sheet that would pass the cap is left out");
        assert_eq!(pages_setting(4), "theme_pages.4");
    }

    #[test]
    fn selectors_are_limited() {
        assert!(!page(":has(a) b { color: red }", Source::Yours).is_empty());
        assert!(page(":has(:has(a)) { color: red }", Source::Yours).is_empty());
        assert!(page(":is(:has(:has(a))) { color: red }", Source::Yours).is_empty());
        assert!(!page("input[type=\"password\" i], a:nth-child(2n+1)::selection { color: red }", Source::Yours).is_empty());
        assert!(page(":host-context(x) { color: red }", Source::Yours).is_empty());
        assert!(page("a { b { color: red } }", Source::Yours).is_empty(), "no nested rules");
    }

    #[test]
    fn the_rule_count_is_capped() {
        let many = "a { color: red } ".repeat(MAX_RULES + 5);
        let p = parse(&format!("@tarn-site a.test {{ {many} }}"), Source::Yours).unwrap();
        assert_eq!(p.sites[0].css.matches('{').count(), MAX_RULES);
        assert!(p.warnings.iter().any(|w| w.contains("rules")));
    }

    #[test]
    fn styles_for_every_site_are_capped() {
        let big = "a { color: red } ".repeat(1_500);
        assert!(parse(&format!("@tarn-site * {{ {big} }}"), Source::Yours).unwrap().sites.is_empty());
        assert!(!parse(&format!("@tarn-site a.test {{ {big} }}"), Source::Yours).unwrap().sites.is_empty());
    }

    #[test]
    fn to_css_round_trips() {
        let meta = Meta { name: "Mine \"x\"".into(), author: String::new(), version: "1".into(), scheme: Scheme::Light };
        let mut tokens = Tokens::LIGHT;
        tokens.accent = 0x12345678;
        tokens.radius[2] = 3.5;
        tokens.fog_mode = Fog::Off;
        tokens.search_effect = Effect::Flames;
        tokens.search_color = Some(0x3e95cbff);
        let sites = vec![
            SiteSheet { hosts: vec![HostPattern::Any], css: "a{color:red;}".into() },
            SiteSheet { hosts: vec![HostPattern::Exact("a.test".into()), HostPattern::Suffix("b.test".into())], css: "b{display:none;}".into() },
        ];
        let original = Parsed { meta, tokens, font: Some("Inter".into()), adjusted: vec![], warnings: vec![], sites };
        let p = parse(&to_css(&original), Source::Yours).unwrap();
        assert_eq!(p, original);
    }

    #[test]
    fn never_panics_and_output_is_stable() {
        const PIECES: [&str; 32] = [
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            ";",
            ":",
            ",",
            "\"",
            "'",
            "\\",
            "/*",
            "*/",
            "@",
            "#",
            "-",
            "url(",
            "@tarn-site",
            "*",
            "a",
            ".b",
            "color",
            "red",
            " ",
            "\n",
            "\u{0}",
            "é",
            "!important",
            "rgb(",
            "calc(",
            "--tarn-text",
        ];
        let mut seed = 0x2545f4914f6cdd1du64;
        for _ in 0..10_000 {
            let mut s = String::new();
            for _ in 0..(seed % 64) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push_str(PIECES[(seed % PIECES.len() as u64) as usize]);
            }
            for source in [Source::Yours, Source::Unverified] {
                let p = parse(&s, source).unwrap();
                for sheet in &p.sites {
                    let again = parse(&format!("@tarn-site a.test {{ {} }}", sheet.css), Source::Yours).unwrap();
                    assert_eq!(again.sites.first().map(|x| x.css.as_str()), Some(sheet.css.as_str()), "input {s:?}");
                }
            }
        }
    }

    #[test]
    fn a_full_size_theme_parses_within_budget() {
        let rule = "a.b > c:hover, d[e=\"f\"] { color: rgb(1 2 3); background-color: #123456; } ";
        let css = format!("@tarn-site a.test {{ {} }}", rule.repeat((MAX_THEME_BYTES - 64) / rule.len()));
        let start = std::time::Instant::now();
        parse(&css, Source::Yours).unwrap();
        // Timed only in release: debug builds on a loaded CI runner are too slow to judge.
        if !cfg!(debug_assertions) {
            assert!(start.elapsed().as_millis() <= 10, "{:?}", start.elapsed());
        }
    }
}
