use std::fmt::Write;

use shield::ShieldLevel;

pub const W: u32 = 360;
pub const H: u32 = 216;
const FONT: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub name: String,
    pub level: ShieldLevel,
    pub seed: i64,
    pub since: Option<String>,
    pub days: Option<i64>,
    pub counters: [(&'static str, i64); 3],
    pub stamps: Vec<String>,
    pub week: Vec<store::ShieldDay>,
    pub sites: Vec<(String, i64)>,
}

const DAY_MS: i64 = 86_400_000;

pub fn date(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn read(store: &store::Store, profile_id: store::Id, private: bool, name: String, now: i64) -> Option<Card> {
    let profile = store.profile(profile_id).ok()?;
    let stats = store.shield_stats(profile_id).ok()?;
    let created = (!private && stats.created_at > 0).then_some(stats.created_at);
    let year = |ms: i64| date(ms).get(..4).and_then(|y| y.parse::<i64>().ok());
    let total = stats.tracking_params + stats.https_upgrades + stats.local_blocked;
    Some(Card {
        name,
        level: profile.shield_level,
        seed: stats.seed,
        since: created.map(|c| long_date(&date(c))),
        days: created.map(|c| (now - c).max(0) / DAY_MS + 1),
        counters: [
            ("Tracking parameters removed", stats.tracking_params),
            ("Pages upgraded to HTTPS", stats.https_upgrades),
            ("Local network visits blocked", stats.local_blocked),
        ],
        stamps: stamps(created.and_then(year), year(now).unwrap_or_default(), total),
        week: store.shield_week(profile_id).unwrap_or_default(),
        sites: store.shield_sites(profile_id, 5).unwrap_or_default(),
    })
}

pub fn week_rows(week: &[store::ShieldDay]) -> Vec<(&'static str, Vec<i64>)> {
    const NAMES: [&str; 5] =
        ["Tracking parameters removed", "Pages upgraded to HTTPS", "Local network visits blocked", "Pages on block lists stopped", "Pop-ups blocked"];
    NAMES
        .iter()
        .enumerate()
        .map(|(ix, name)| {
            (*name, week.iter().map(|d| [d.tracking_params, d.https_upgrades, d.local_blocked, d.listed_stopped, d.popups_blocked][ix].max(0)).collect())
        })
        .collect()
}

pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn days_text(days: i64) -> String {
    format!("{} {}", thousands(days.max(0) as u64), if days == 1 { "day" } else { "days" })
}

pub const SHORT: [(&str, &str); 3] = [("Parameters", "removed"), ("HTTPS", "upgrades"), ("Local visits", "blocked")];

pub fn motto(level: ShieldLevel) -> (&'static str, &'static str) {
    match level {
        ShieldLevel::Standard => ("Standard", "Trackers blocked, links cleaned"),
        ShieldLevel::Strict => ("Strict", "No cross-site referrers"),
        ShieldLevel::Lockdown => ("Lockdown", "No scripts, nothing kept"),
    }
}

pub fn stamps(created_year: Option<i64>, this_year: i64, total: i64) -> Vec<String> {
    let mut out: Vec<String> = created_year.map(|y| (y..=this_year).map(|y| y.to_string()).collect()).unwrap_or_default();
    out.extend([(100, "100"), (1_000, "1K"), (10_000, "10K")].into_iter().filter(|(n, _)| total >= *n).map(|(_, l)| format!("{l} ✓")));
    out
}

pub fn long_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = iso.splitn(3, '-').map(|p| p.parse::<usize>().ok());
    match (parts.next().flatten(), parts.next().flatten(), parts.next().flatten()) {
        (Some(year), Some(month @ 1..=12), Some(day)) => format!("{day} {} {year}", MONTHS[month - 1]),
        _ => iso.to_owned(),
    }
}

struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u32 {
        (self.next() % n) as u32
    }
}

pub struct Scene {
    pub sky: &'static str,
    pub far: &'static str,
    pub snow: &'static str,
    pub band: &'static str,
    pub near: &'static str,
    pub lake: &'static str,
    pub shallow: &'static str,
    pub ink: &'static str,
    pub dark: bool,
}

const SCENES: [Scene; 4] = [
    Scene {
        sky: "#EAE3DF",
        far: "#98A0B5",
        snow: "#CDCFDF",
        band: "#4C5D75",
        near: "#B7BCCC",
        lake: "#1567A3",
        shallow: "#3E95CB",
        ink: "#1B2A3D",
        dark: false,
    },
    Scene {
        sky: "#DCEAF4",
        far: "#8E9AB2",
        snow: "#E6EAF2",
        band: "#45586F",
        near: "#AEB6C8",
        lake: "#1567A3",
        shallow: "#3E95CB",
        ink: "#14304A",
        dark: false,
    },
    Scene {
        sky: "#EBD0C4",
        far: "#9C8DA6",
        snow: "#DCCBD4",
        band: "#4E4866",
        near: "#B9AABD",
        lake: "#1B5E93",
        shallow: "#3A86B8",
        ink: "#33213A",
        dark: false,
    },
    Scene {
        sky: "#0E1F33",
        far: "#3B4D6B",
        snow: "#6E7F9E",
        band: "#1A2A42",
        near: "#4A5C7C",
        lake: "#0F5288",
        shallow: "#1F75B0",
        ink: "#E6EEF6",
        dark: true,
    },
];

const LAKE: u32 = 112;
pub const RIDGES: (u32, u32) = (88, 48);

pub fn landscape(seed: i64) -> (&'static Scene, String, String) {
    let mut draw = Draw(seed as u64);
    let s = &SCENES[draw.below(4) as usize];
    let (px, py) = (214 + draw.below(84), 40 + draw.below(20));
    let (lx, ly) = (96 + draw.below(70), 64 + draw.below(14));
    let (cut, edge, band) = (ly.max(py) + 18 + draw.below(10), 76 + draw.below(14), 92 + draw.below(8));
    let (left, right) = (124 + draw.below(30), 206 + draw.below(30));
    let l = LAKE;
    let mut far = format!(
        r#"<path d="M0 {y0} Q{c0} {c0y} {lx} {ly} Q{dip} {cut} {px} {py} Q{c1} {c1y} 360 {edge} V{l} H0 Z" fill="{}"/>"#,
        s.far,
        y0 = ly + 22,
        c0 = lx / 2,
        c0y = ly + 8,
        dip = (lx + px) / 2,
        c1 = px + 36,
        c1y = py + 22
    );
    let _ = write!(
        far,
        r#"<path d="M{px} {py} Q{} {} {} {} Q{} {} {} {} Q{} {} {px} {py} Z" fill="{}"/>"#,
        px + 30,
        py + 16,
        px + 64,
        py + 38,
        px + 28,
        py + 30,
        px + 8,
        py + 32,
        px + 4,
        py + 14,
        s.snow
    );
    let _ = write!(far, r#"<path d="M{lx} {ly} Q{} {} {} {} Q{} {} {lx} {ly} Z" fill="{}"/>"#, lx + 18, ly + 10, lx + 34, ly + 22, lx + 14, ly + 18, s.snow);
    let _ = write!(far, r#"<path d="M0 {} Q90 {band} 180 96 Q270 94 360 {l} V{} H0 Z" fill="{}"/>"#, l + 2, l + 10, s.band);
    let _ = write!(far, r#"<rect y="{l}" width="360" height="{}" fill="{}"/>"#, H - l, s.lake);
    let _ = write!(far, r#"<path d="M0 {l} H360 V{} Q270 {} 180 {} Q90 {} 0 {} Z" fill="{}"/>"#, l + 9, l + 14, l + 9, l + 5, l + 12, s.shallow);
    let near = format!(
        r#"<path d="M0 94 Q{} 90 {} {} Q{} {} {left} {} Q70 {} 0 {} Z" fill="{fill}"/><path d="M360 90 Q{} 94 {} {} Q{} {} {right} {} Q290 {} 360 {} Z" fill="{fill}"/>"#,
        left / 2,
        left - 34,
        l - 4,
        left - 18,
        l + 6,
        l + 8,
        l + 10,
        l + 16,
        (360 + right) / 2,
        right + 34,
        l - 6,
        right + 16,
        l + 4,
        l + 8,
        l + 10,
        l + 14,
        fill = s.near,
    );
    (s, far, near)
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            c if c.is_control() => String::new(),
            c => c.to_string(),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub x: u32,
    pub y: u32,
    pub anchor: &'static str,
    pub size: f32,
    pub weight: u32,
    pub fill: &'static str,
    pub opacity: f32,
    pub rotate: i32,
    pub text: String,
}

type Font = (&'static str, f32, u32, &'static str, f32);

fn text(out: &mut Vec<Label>, (x, y): (u32, u32), (anchor, size, weight, fill, opacity): Font, body: &str) {
    out.push(Label { x, y, anchor, size, weight, fill, opacity, rotate: 0, text: body.to_owned() });
}

fn labels_svg(labels: &[Label]) -> String {
    let mut out = String::new();
    for l in labels {
        let turn = if l.rotate == 0 { String::new() } else { format!(r#" transform="rotate({} {} {})""#, l.rotate, l.x, l.y - 4) };
        let _ = write!(
            out,
            r#"<text x="{}" y="{}"{turn} text-anchor="{}" font-family="{FONT}" font-size="{}" font-weight="{}" fill="{}" fill-opacity="{}">{}</text>"#,
            l.x,
            l.y,
            l.anchor,
            l.size,
            l.weight,
            l.fill,
            l.opacity,
            escape(&l.text)
        );
    }
    out
}

pub fn tint(level: ShieldLevel, dark: bool) -> (&'static str, u32) {
    match (level, dark) {
        (ShieldLevel::Standard, false) => ("#4C5D75", 60),
        (ShieldLevel::Standard, true) => ("#B7BCCC", 60),
        (ShieldLevel::Strict, false) => ("#1567A3", 42),
        (ShieldLevel::Strict, true) => ("#8ED1F7", 42),
        (ShieldLevel::Lockdown, false) => ("#9A5A0E", 64),
        (ShieldLevel::Lockdown, true) => ("#F2C14E", 64),
    }
}

pub fn face(sky: &str, body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}"><defs><clipPath id="card"><rect width="{W}" height="{H}" rx="18"/></clipPath></defs><g clip-path="url(#card)"><rect width="{W}" height="{H}" fill="{sky}"/>{body}</g></svg>"#
    )
}

pub fn front_svg(card: &Card, near: bool) -> String {
    let (sky, shapes, labels) = front(card, near);
    face(sky, &(shapes + &labels_svg(&labels)))
}

pub fn back_svg(card: &Card) -> String {
    let (sky, shapes, labels) = back(card);
    face(sky, &(shapes + &labels_svg(&labels)))
}

pub fn front(card: &Card, near: bool) -> (&'static str, String, Vec<Label>) {
    let (s, mut out, ridges) = landscape(card.seed);
    let mut labels = Vec::new();
    if near {
        out.push_str(&ridges);
    }
    let mut name: String = card.name.chars().take(22).collect();
    if card.name.chars().count() > 22 {
        name.push('…');
    }
    let (level, promise) = motto(card.level);
    let (color, pill) = tint(card.level, s.dark);
    text(&mut labels, (20, 38), ("start", 20., 700, s.ink, 1.), &name);
    let _ = write!(out, r#"<rect x="20" y="49" width="{pill}" height="18" rx="9" fill="{color}"/>"#);
    text(&mut labels, (20 + pill / 2, 62), ("middle", 11., 600, s.sky, 1.), level);
    text(&mut labels, (20 + pill + 7, 62), ("start", 11., 400, s.ink, 0.72), promise);
    for (ix, ((_, n), (first, second))) in card.counters.iter().zip(SHORT).enumerate() {
        let x = 20 + ix as u32 * 112;
        let count = thousands((*n).max(0) as u64);
        text(&mut labels, (x, 154), ("start", if count.len() > 6 { 16. } else { 24. }, 700, "white", if *n > 0 { 1. } else { 0.45 }), &count);
        text(&mut labels, (x, 168), ("start", 10.5, 400, "#CFE8F8", 0.85), first);
        text(&mut labels, (x, 180), ("start", 10.5, 400, "#CFE8F8", 0.85), second);
    }
    if let (Some(days), Some(since)) = (card.days, &card.since) {
        text(&mut labels, (20, 204), ("start", 11., 400, "white", 0.8), &format!("Protected for {} · since {since}", days_text(days)));
    }
    text(&mut labels, (340, 204), ("end", 10.5, 700, "white", 0.55), "Tarn Shield");
    (s.sky, out, labels)
}

pub fn back(card: &Card) -> (&'static str, String, Vec<Label>) {
    let mut labels = Vec::new();
    const TILTS: [i32; 5] = [-9, 6, -4, 10, -7];
    let (s, far, near) = landscape(card.seed);
    let mut out = format!(r#"<g transform="translate(0 120) scale(1 0.5)">{far}{near}</g>"#);
    let total: i64 = card.counters.iter().map(|(_, n)| (*n).max(0)).sum();
    let (color, _) = tint(card.level, s.dark);
    text(&mut labels, (20, 36), ("start", 15., 700, s.ink, 1.), "Stamps");
    text(&mut labels, (340, 36), ("end", 11., 400, s.ink, 0.7), &format!("{} protected", thousands(total as u64)));
    let shown = &card.stamps[card.stamps.len().saturating_sub(10)..];
    for (ix, stamp) in shown.iter().enumerate() {
        let (x, y) = (52 + ix as u32 % 5 * 64, 74 + ix as u32 / 5 * 48);
        let _ = write!(
            out,
            r#"<g transform="rotate({} {x} {y})"><circle cx="{x}" cy="{y}" r="21" fill="{}" stroke="{color}" stroke-width="1.6" stroke-dasharray="3 2.5"/><circle cx="{x}" cy="{y}" r="16.5" fill="none" stroke="{color}" stroke-opacity="0.5" stroke-width="0.8"/></g>"#,
            TILTS[ix % 5],
            s.sky,
        );
        labels.push(Label { x, y: y + 4, anchor: "middle", size: 11., weight: 700, fill: color, opacity: 1., rotate: TILTS[ix % 5], text: stamp.clone() });
    }
    if shown.is_empty() {
        text(&mut labels, (180, 100), ("middle", 12., 400, s.ink, 0.55), "No stamps yet");
    }
    let next = [(100, "100"), (1_000, "1K"), (10_000, "10K")].into_iter().find(|(n, _)| total < *n);
    let (note, part) = match next {
        Some((n, label)) => (format!("{} more to the {label} stamp", thousands((n - total) as u64)), total as f32 / n as f32),
        None => ("Every milestone reached".to_owned(), 1.),
    };
    text(&mut labels, (20, 196), ("start", 10.5, 400, "white", 0.85), &note);
    let _ = write!(
        out,
        r#"<rect x="20" y="202" width="320" height="4" rx="2" fill="white" fill-opacity="0.2"/><rect x="20" y="202" width="{:.1}" height="4" rx="2" fill="white"/>"#,
        (320. * part).max(4.)
    );
    (s.sky, out, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str) -> Card {
        Card {
            name: name.to_owned(),
            level: ShieldLevel::Strict,
            seed: 42,
            since: Some("2026-01-02".into()),
            days: Some(316),
            counters: [("Tracking parameters removed", 12), ("Pages upgraded to HTTPS", 3), ("Local network visits blocked", 0)],
            stamps: stamps(Some(2025), 2026, 150),
            week: Vec::new(),
            sites: Vec::new(),
        }
    }

    #[test]
    fn designs_follow_the_seed_through_four_times_of_day() {
        assert_eq!(landscape(7).1, landscape(7).1);
        assert_ne!(landscape(7).1, landscape(8).1);
        let skies: std::collections::HashSet<_> = (0..40).map(|seed| landscape(seed).0.sky).collect();
        assert_eq!(skies.len(), SCENES.len());
    }

    #[test]
    fn the_card_escapes_its_text_and_fits_a_private_profile_and_many_stamps() {
        let svg = front_svg(&card("<script>&\"x\u{7}"), true);
        assert!(svg.contains("&lt;script&gt;&amp;&quot;x<") && !svg.contains('\u{7}'));
        let private = Card { since: None, days: None, stamps: Vec::new(), ..card("Private") };
        assert!(!front_svg(&private, true).contains("Protected for") && back_svg(&private).contains("No stamps yet"));
        let old = Card { stamps: stamps(Some(2012), 2026, 20_000), counters: [("", 1_234_567), ("", 0), ("", 0)], ..card("A name far too long for the card") };
        let (front, back) = (front_svg(&old, true), back_svg(&old));
        assert!(front.contains(r#"font-size="16""#) && front.contains("A name far too long fo…"));
        assert_eq!(back.matches("<g transform=\"rotate").count(), 10);
        assert!(back.contains("10K ✓") && !back.contains(">2012<") && back.contains("Every milestone reached"));
    }

    #[test]
    fn a_profile_reads_its_card_and_a_private_one_shows_no_dates() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let profile = store.create_profile("Personal", "u", false).unwrap();
        let card = read(&store, profile.id, false, "Personal".into(), crate::now()).unwrap();
        assert_eq!((card.name.as_str(), card.days), ("Personal", Some(1)));
        assert!(card.since.is_some() && card.stamps.len() == 1);
        let private = read(&store, profile.id, true, "Private".into(), crate::now()).unwrap();
        assert!(private.since.is_none() && private.days.is_none() && private.stamps.is_empty());
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400_000), "2000-02-29");
        assert_eq!(date(1_794_441_600_000), "2026-11-12");
        let rows = week_rows(&card.week);
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|(_, days)| days.len() == card.week.len()));
    }

    #[test]
    fn numbers_days_dates_and_stamps_read_naturally() {
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(days_text(1), "1 day");
        assert_eq!(days_text(1200), "1,200 days");
        assert_eq!(long_date("2025-03-02"), "2 Mar 2025");
        assert_eq!(long_date("bad"), "bad");
        assert_eq!(stamps(Some(2024), 2026, 0), ["2024", "2025", "2026"]);
        assert_eq!(stamps(None, 2026, 1_500), ["100 ✓", "1K ✓"]);
    }
}
