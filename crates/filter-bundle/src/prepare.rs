//! Turns downloaded upstream lists into the files of a bundle. Runs on untrusted input in a job
//! without the signing key; `build` only hashes and signs what this writes.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;

use serde::{Deserialize, Serialize};
use shield::Category;
use shield::bundle::{Kind, Manifest, valid_name};
use shield::domains::{self, DomainSet, MAX_ENTRIES};

pub const ENTRIES: &str = "entries.json";
const MAX_RAW_BYTES: u64 = 16 << 20;
/// Longer `!#if` expressions are false, which also bounds the parser's recursion.
const MAX_CONDITION_TOKENS: usize = 64;
/// `!#include` lines a source may have; uBlock filters has ten today.
const MAX_INCLUDES: usize = 16;
/// `!#if` tokens that hold for Tarn: WebKit, with element hiding but without HTML filtering or
/// scriptlets. Every other token is false.
const TRUE_TOKENS: &[&str] = &["env_safari", "cap_user_stylesheet"];
/// A list may shrink by at most a fifth or grow by at most half from one bundle to the next.
const MAX_SHRINK: f64 = 0.8;
const MAX_GROWTH: f64 = 1.5;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub name: String,
    pub url: String,
    pub category: String,
    pub format: Format,
    pub licence: String,
    pub min: u64,
    pub max: u64,
    /// Also block the domains as subresources in every page, not only at navigation.
    #[serde(default)]
    pub rules: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Filters,
    Domains,
}

/// One bundle file as `build` signs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub name: String,
    pub category: String,
    pub kind: Kind,
    pub count: u64,
}

pub fn load_sources(path: &Path) -> Result<Sources, String> {
    let text = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let sources: Sources = serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut names = BTreeSet::new();
    for s in &sources.sources {
        let ok = valid_name(&s.name)
            && !s.name.contains('+')
            && names.insert(s.name.as_str())
            && s.url.starts_with("https://")
            && Category::parse(&s.category).is_some()
            && s.min <= s.max
            && !s.licence.is_empty();
        if !ok {
            return Err(format!("invalid source {}", s.name));
        }
    }
    Ok(sources)
}

/// Evaluates a `!#if` expression: tokens, `!`, `&&`, `||` and parentheses, `&&` binding tighter.
/// Anything malformed is false, so its block is left out.
fn eval(expr: &str) -> bool {
    let mut spaced = String::new();
    for c in expr.replace("&&", " && ").replace("||", " || ").chars() {
        match c {
            '(' | ')' | '!' => spaced.extend([' ', c, ' ']),
            c => spaced.push(c),
        }
    }
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    if tokens.len() > MAX_CONDITION_TOKENS {
        return false;
    }
    let mut pos = 0;
    let value = or(&tokens, &mut pos);
    value == Some(true) && pos == tokens.len()
}

fn or(tokens: &[&str], pos: &mut usize) -> Option<bool> {
    let mut value = and(tokens, pos)?;
    while tokens.get(*pos) == Some(&"||") {
        *pos += 1;
        value |= and(tokens, pos)?;
    }
    Some(value)
}

fn and(tokens: &[&str], pos: &mut usize) -> Option<bool> {
    let mut value = unary(tokens, pos)?;
    while tokens.get(*pos) == Some(&"&&") {
        *pos += 1;
        value &= unary(tokens, pos)?;
    }
    Some(value)
}

fn unary(tokens: &[&str], pos: &mut usize) -> Option<bool> {
    let token = *tokens.get(*pos)?;
    *pos += 1;
    match token {
        "!" => unary(tokens, pos).map(|v| !v),
        "(" => {
            let value = or(tokens, pos)?;
            (tokens.get(*pos) == Some(&")")).then(|| *pos += 1)?;
            Some(value)
        }
        ")" | "&&" | "||" => None,
        word => Some(TRUE_TOKENS.contains(&word)),
    }
}

/// A plain file name in the source's folder, as the workflow's download step matches it; EasyList
/// Cookie names one `easylist_cookie_specific_uBO.txt`.
fn valid_include(name: &str) -> bool {
    name.ends_with(".txt") && name.starts_with(|c: char| c.is_ascii_alphanumeric()) && valid_name(&name.to_ascii_lowercase())
}

/// Applies uBlock Origin's `!#if`/`!#else`/`!#endif` for Tarn's tokens and inlines `!#include`
/// files (one level, from the same folder upstream), since `adblock` treats both as comments.
pub fn preprocess(text: &str, include: &dyn Fn(&str) -> Result<String, String>, nested: bool) -> Result<String, String> {
    let included = RefCell::new(BTreeSet::new());
    preprocess_into(text, include, nested, &included)
}

fn preprocess_into(text: &str, include: &dyn Fn(&str) -> Result<String, String>, nested: bool, included: &RefCell<BTreeSet<String>>) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<bool> = Vec::new();
    for line in text.lines() {
        let active = stack.iter().all(|b| *b);
        if let Some(expr) = line.strip_prefix("!#if ") {
            stack.push(eval(expr));
        } else if line.trim_end() == "!#else" {
            let top = stack.last_mut().ok_or("!#else without !#if")?;
            *top = !*top;
        } else if line.trim_end() == "!#endif" {
            stack.pop().ok_or("!#endif without !#if")?;
        } else if let Some(name) = line.strip_prefix("!#include ") {
            if !active {
                continue;
            }
            let name = name.trim();
            if nested || !valid_include(name) || !included.borrow_mut().insert(name.to_owned()) || included.borrow().len() > MAX_INCLUDES {
                return Err(format!("unsupported, repeated or too many includes ({name:?})"));
            }
            out.push_str(&preprocess_into(&include(name)?, include, true, included)?);
        } else if active {
            out.push_str(line);
            out.push('\n');
        }
    }
    if stack.is_empty() { Ok(out) } else { Err("!#if without !#endif".into()) }
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && !line.starts_with(['!', '['])
}

/// `||host^` and `||host^$opts` with only the options in `allowed`.
fn network_host<'a>(line: &'a str, allowed: &[&str]) -> Option<(&'a str, Vec<&'a str>)> {
    let (host, rest) = line.strip_prefix("||")?.split_once('^')?;
    let options: Vec<&str> = match rest {
        "" => Vec::new(),
        rest => rest.strip_prefix('$')?.split(',').collect(),
    };
    options.iter().all(|o| allowed.contains(o)).then_some((host, options))
}

/// The host a domain-list line names, in hosts (`0.0.0.0 host`), plain, wildcard (`*.host`) or
/// network-filter (`||host^`) form; `None` for comments, cosmetic rules and anything narrower
/// than a whole host.
pub fn domain_line(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(['#', '!', '[', '@']) || line.contains("##") || line.contains("#@#") || line.contains("#?#") || line.contains("#$#")
    {
        return None;
    }
    let line = line.find([' ', '\t']).and_then(|i| line[i..].find('#').map(|j| &line[..i + j])).unwrap_or(line).trim();
    if line.starts_with("||") {
        return network_host(line, &["all", "doc", "document", "important", "popup", "third-party", "3p"]).map(|(h, _)| h);
    }
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let host = match (first, parts.next(), parts.next()) {
        ("0.0.0.0" | "127.0.0.1" | "::" | "::1", Some(host), None) => host,
        (host, None, None) => host.strip_prefix("*.").unwrap_or(host),
        _ => return None,
    };
    (!matches!(host, "localhost" | "localhost.localdomain" | "0.0.0.0" | "broadcasthost" | "local")).then_some(host)
}

/// Pure-domain `$popup` rules of a filter list, which `adblock` cannot convert.
fn popup_hosts(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines()
        .filter_map(|l| network_host(l.trim(), &["popup", "third-party", "3p"]))
        .filter(|(_, options)| options.contains(&"popup"))
        .filter_map(|(host, _)| set_entry(host))
}

/// A set entry for the public internet: no private, loopback, link-local, documentation or
/// otherwise unroutable IPv4 address, so no list can put a warning page on a router or a local
/// server.
fn routable(entry: &str) -> bool {
    entry.parse::<Ipv4Addr>().map_or(true, |ip| {
        !(ip.is_private()
            || ip.is_loopback()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_broadcast()
            || ip.is_documentation()
            || ip.octets()[0] == 0
            || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1])))
    })
}

fn set_entry(line: &str) -> Option<String> {
    domains::entry(line).filter(|e| routable(e))
}

fn check_count(name: &str, count: u64, min: u64, max: u64) -> Result<(), String> {
    if count < min || count > max {
        return Err(format!("{name}: {count} entries, expected {min}..={max}; refusing to sign"));
    }
    Ok(())
}

/// Counts of the last published bundle, so a list that suddenly shrinks or balloons is noticed.
fn previous_counts(dir: Option<&Path>) -> BTreeMap<String, u64> {
    let manifest = dir.and_then(|d| fs::read(d.join(shield::bundle::MANIFEST)).ok());
    let parsed = manifest.and_then(|m| serde_json::from_slice::<Manifest>(&m).ok());
    parsed.into_iter().flat_map(|m| m.lists).map(|l| (l.name, l.count)).collect()
}

pub struct Options<'a> {
    pub sources: &'a Path,
    pub raw: &'a Path,
    pub youtube: &'a Path,
    /// `{"sha256": [...]}`: theme files withdrawn as unsafe (`shield::revocations`).
    pub theme_revocations: Option<&'a Path>,
    pub previous: Option<&'a Path>,
    pub allow_change: bool,
    pub out: &'a Path,
}

pub fn prepare(o: &Options) -> Result<Vec<Prepared>, String> {
    let sources = load_sources(o.sources)?;
    fs::create_dir_all(o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let read = |name: &str| -> Result<String, String> {
        let file = fs::File::open(o.raw.join(name)).map_err(|e| format!("{name}: {e}"))?;
        let data = shield::cache::read_capped(file, MAX_RAW_BYTES).map_err(|e| format!("{name}: {e}"))?;
        Ok(String::from_utf8_lossy(&data).into_owned())
    };
    let write = |name: &str, data: &[u8]| fs::write(o.out.join(name), data).map_err(|e| format!("{name}: {e}"));
    let mut prepared = Vec::new();
    let mut sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut rules: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut dropped = 0usize;
    for s in &sources.sources {
        let raw = read(&s.name)?;
        match s.format {
            Format::Filters => {
                let text = preprocess(&raw, &|inc| read(&format!("{}+{inc}", s.name)), false).map_err(|e| format!("{}: {e}", s.name))?;
                let count = text.lines().filter(|l| is_rule(l)).count() as u64;
                check_count(&s.name, count, s.min, s.max)?;
                // A cookie list's `$popup` rule must not block a whole site under Block pop-ups.
                if s.category != "annoyances" {
                    sets.entry(Category::Popups.as_str().into()).or_default().extend(popup_hosts(&text));
                }
                let name = format!("{}.txt", s.name);
                write(&name, text.as_bytes())?;
                prepared.push(Prepared { name, category: s.category.clone(), kind: Kind::Filters, count });
            }
            Format::Domains => {
                let lines: Vec<&str> = raw.lines().filter_map(domain_line).collect();
                let accepted: BTreeSet<String> = lines.iter().filter_map(|l| set_entry(l)).collect();
                dropped += lines.len() - lines.iter().filter(|l| set_entry(l).is_some()).count();
                check_count(&s.name, accepted.len() as u64, s.min, s.max)?;
                if s.rules {
                    rules.entry(s.category.clone()).or_default().extend(accepted.iter().cloned());
                }
                sets.entry(s.category.clone()).or_default().extend(accepted);
            }
        }
    }
    for (category, set) in &sets {
        if set.len() > MAX_ENTRIES {
            return Err(format!("{category}: {} domains exceed the set limit", set.len()));
        }
        let name = format!("{category}.set");
        write(&name, &DomainSet::encode(set.iter().map(String::as_str)))?;
        prepared.push(Prepared { name, category: category.clone(), kind: Kind::Set, count: set.len() as u64 });
    }
    for (category, domains) in &rules {
        let name = format!("{category}-rules.txt");
        let text: String = domains.iter().flat_map(|d| [d.as_str(), "\n"]).collect();
        write(&name, text.as_bytes())?;
        prepared.push(Prepared { name, category: category.clone(), kind: Kind::Domains, count: domains.len() as u64 });
    }
    let youtube = fs::read(o.youtube).map_err(|e| format!("{}: {e}", o.youtube.display()))?;
    let config = shield::youtube::parse(&youtube).ok_or("youtube.json is invalid")?;
    write("youtube.json", &serde_json::to_vec(&config).map_err(|e| e.to_string())?)?;
    prepared.push(Prepared { name: "youtube.json".into(), category: "youtube".into(), kind: Kind::Config, count: config.keys.len() as u64 });
    if let Some(path) = o.theme_revocations {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let list = shield::revocations::parse(&bytes).ok_or("theme-revocations.json is invalid")?;
        let name = shield::revocations::FILE.to_owned();
        write(&name, &serde_json::to_vec(&list).map_err(|e| e.to_string())?)?;
        prepared.push(Prepared { name, category: shield::revocations::CATEGORY.into(), kind: Kind::Config, count: list.sha256.len() as u64 });
    }
    let mut licences = String::from(
        "Lists in this bundle, each under its own licence. Files built from them were preprocessed,\n\
         de-duplicated, reduced to hostnames or hashed by Tarn's filter-bundle, and are modified versions.\n\n",
    );
    for s in &sources.sources {
        licences.push_str(&format!("{}\t{}\t{}\n", s.name, s.licence, s.url));
    }
    write("licenses.txt", licences.as_bytes())?;
    prepared.push(Prepared { name: "licenses.txt".into(), category: "all".into(), kind: Kind::License, count: sources.sources.len() as u64 });

    let previous = previous_counts(o.previous);
    // The withdrawal list only grows, by hand; a first withdrawal must not need --allow-change.
    for p in prepared.iter().filter(|p| p.category != shield::revocations::CATEGORY) {
        let Some(&before) = previous.get(&p.name) else { continue };
        let (lo, hi) = ((before as f64 * MAX_SHRINK) as u64, (before as f64 * MAX_GROWTH) as u64);
        if !o.allow_change && !(lo..=hi).contains(&p.count) {
            return Err(format!("{}: {} entries, last bundle had {before}; rerun with --allow-change after checking upstream", p.name, p.count));
        }
    }
    eprintln!("filter-bundle: dropped {dropped} domain entries (invalid, public suffix or protected)");
    write(ENTRIES, &serde_json::to_vec_pretty(&prepared).map_err(|e| e.to_string())?)?;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocesses_conditions_and_includes() {
        let text = "a\n!#if env_mobile\nmobile\n!#else\ndesktop\n!#endif\n!#if !cap_html_filtering\nno-html\n!#endif\n!#if env_safari || env_chromium\nsafari\n!#endif\n!#include part.txt\n";
        let include = |name: &str| {
            if name == "part.txt" || name == "Part_uBO.txt" { Ok("!#if env_firefox\nfirefox\n!#endif\npart\n".to_owned()) } else { Err(name.to_owned()) }
        };
        assert_eq!(preprocess(text, &include, false).unwrap(), "a\ndesktop\nno-html\nsafari\npart\n");
        assert!(preprocess("!#include ../x.txt\n", &include, false).is_err());
        assert!(preprocess("!#include .x.txt\n", &include, false).is_err());
        assert!(preprocess("!#include x.json\n", &include, false).is_err());
        assert_eq!(preprocess("!#include Part_uBO.txt\n", &include, false).unwrap(), "part\n");
        assert!(preprocess("!#include part.txt\n!#include part.txt\n", &include, false).is_err());
        let many: String = (0..=MAX_INCLUDES).map(|i| format!("!#include p{i}.txt\n")).collect();
        assert!(preprocess(&many, &|_| Ok(String::new()), false).is_err());
        assert!(preprocess("!#include part.txt\n", &|_| Ok("!#include part.txt\n".into()), false).is_err());
        assert!(preprocess("!#if x\n", &include, false).is_err());
        assert!(preprocess("!#endif\n", &include, false).is_err());
        assert_eq!(preprocess("!#if env_mobile\n!#include missing.txt\n!#endif\n", &include, false).unwrap(), "");
    }

    #[test]
    fn evaluates_conditions_with_precedence_and_parentheses() {
        for (expr, expected) in [
            ("env_safari", true),
            ("!env_safari", false),
            ("env_chromium", false),
            ("!(env_chromium || env_safari)", false),
            ("!env_chromium || env_safari", true),
            ("env_chromium || env_safari && cap_user_stylesheet", true),
            ("(env_chromium || env_safari) && !cap_user_stylesheet", false),
            ("!!env_safari", true),
            ("(env_safari", false),
            ("env_safari)", false),
            ("env_safari &&", false),
            ("", false),
            (&"!".repeat(100_000), false),
            (&"(".repeat(100_000), false),
        ] {
            assert_eq!(eval(expr), expected, "{expr}");
        }
    }

    #[test]
    fn unroutable_addresses_never_enter_a_set() {
        for bad in ["127.0.0.1", "10.0.0.1", "192.168.1.1", "172.16.0.1", "169.254.1.1", "0.0.0.0", "0.1.2.3", "100.64.0.1", "203.0.113.7", "255.255.255.255"] {
            assert_eq!(set_entry(bad), None, "{bad}");
        }
        assert_eq!(set_entry("8.8.8.8"), Some("8.8.8.8".into()));
        assert_eq!(set_entry("evil.example"), Some("evil.example".into()));
    }

    #[test]
    fn reads_every_domain_list_format() {
        let cases = [
            ("0.0.0.0 evil.example", Some("evil.example")),
            ("127.0.0.1\tevil.example # note", Some("evil.example")),
            ("evil.example", Some("evil.example")),
            ("*.evil.example", Some("evil.example")),
            ("||evil.example^", Some("evil.example")),
            ("||evil.example^$all", Some("evil.example")),
            ("||evil.example^$doc,important", Some("evil.example")),
            ("||evil.example^$script", None),
            ("||evil.example^$domain=a.example", None),
            ("||evil.example/path^", Some("evil.example/path")),
            ("@@||good.example^", None),
            ("evil.example##.ad", None),
            ("# comment", None),
            ("! comment", None),
            ("0.0.0.0 localhost", None),
            ("0.0.0.0 a.example b.example", None),
            ("", None),
        ];
        for (line, expected) in cases {
            assert_eq!(domain_line(line), expected, "{line}");
        }
        assert_eq!(domain_line("||evil.example/path^").and_then(domains::entry), None);
    }

    #[test]
    fn finds_pure_domain_popup_rules() {
        let text = "||pop.example^$popup\n||pop2.example^$popup,third-party\n||path.example/x^$popup\n||script.example^$script,popup\n@@||ok.example^$popup\n||com^$popup\n";
        assert_eq!(popup_hosts(text).collect::<Vec<_>>(), ["pop.example", "pop2.example"]);
    }

    fn fixture(dir: &Path, previous: Option<(&str, u64)>) -> Result<Vec<Prepared>, String> {
        let raw = dir.join("raw");
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("ads"), "! title\n||ads.example^\n||pop.example^$popup\n!#include extra.txt\n").unwrap();
        fs::write(raw.join("ads+extra.txt"), "##.banner\n").unwrap();
        fs::write(raw.join("bad"), "0.0.0.0 evil.example\n0.0.0.0 a.evil.example\nco.uk\nmail.google.com\nphish.example\n").unwrap();
        let sources = dir.join("sources.json");
        fs::write(
            &sources,
            r#"{"sources":[
                {"name":"ads","url":"https://lists.example/ads.txt","category":"ads","format":"filters","licence":"GPL-3.0","min":2,"max":10},
                {"name":"bad","url":"https://lists.example/bad","category":"malware","format":"domains","licence":"MIT","min":2,"max":10,"rules":true}
            ]}"#,
        )
        .unwrap();
        let youtube = dir.join("youtube.json");
        fs::write(&youtube, r#"{"keys":["adPlacements"]}"#).unwrap();
        let prev = dir.join("prev");
        if let Some((name, count)) = previous {
            fs::create_dir_all(&prev).unwrap();
            let manifest = format!(
                r#"{{"format":2,"version":1,"created":"","lists":[{{"name":"{name}","sha256":"","size":0,"category":"malware","kind":"set","count":{count}}}]}}"#
            );
            fs::write(prev.join("manifest.json"), manifest).unwrap();
        }
        prepare(&Options {
            sources: &sources,
            raw: &raw,
            youtube: &youtube,
            theme_revocations: None,
            previous: Some(&prev),
            allow_change: false,
            out: &dir.join("out"),
        })
    }

    #[test]
    fn prepares_every_kind_and_drops_wide_entries() {
        let dir = tempfile::tempdir().unwrap();
        let prepared = fixture(dir.path(), None).unwrap();
        let names: Vec<(&str, Kind, u64)> = prepared.iter().map(|p| (p.name.as_str(), p.kind, p.count)).collect();
        assert_eq!(
            names,
            [
                ("ads.txt", Kind::Filters, 3),
                ("malware.set", Kind::Set, 3),
                ("popups.set", Kind::Set, 1),
                ("malware-rules.txt", Kind::Domains, 3),
                ("youtube.json", Kind::Config, 1),
                ("licenses.txt", Kind::License, 2),
            ]
        );
        let out = dir.path().join("out");
        assert_eq!(fs::read_to_string(out.join("malware-rules.txt")).unwrap(), "a.evil.example\nevil.example\nphish.example\n");
        let set = DomainSet::from_bytes(&fs::read(out.join("malware.set")).unwrap()).unwrap();
        assert!(set.contains("evil.example") && set.contains("x.phish.example") && !set.contains("example.co.uk"));
        assert!(fs::read_to_string(out.join("ads.txt")).unwrap().contains("##.banner"));
        let entries: Vec<Prepared> = serde_json::from_slice(&fs::read(out.join(ENTRIES)).unwrap()).unwrap();
        assert_eq!(entries, prepared);
    }

    #[test]
    fn theme_revocations_become_a_config_entry_when_given() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), None).unwrap();
        let d = dir.path();
        let list = d.join("revoked.json");
        let run = |out: &str| {
            prepare(&Options {
                sources: &d.join("sources.json"),
                raw: &d.join("raw"),
                youtube: &d.join("youtube.json"),
                theme_revocations: Some(&list),
                previous: None,
                allow_change: false,
                out: &d.join(out),
            })
        };
        fs::write(&list, format!(r#"{{"sha256":["{}"]}}"#, "b".repeat(64))).unwrap();
        let entry = run("a").unwrap().into_iter().find(|p| p.category == shield::revocations::CATEGORY).unwrap();
        assert_eq!((entry.kind, entry.count), (Kind::Config, 1));
        let written = fs::read(d.join("a").join(&entry.name)).unwrap();
        assert_eq!(shield::revocations::parse(&written).unwrap().sha256, ["b".repeat(64)]);
        fs::write(&list, r#"{"sha256":["not a hash"]}"#).unwrap();
        assert!(run("b").is_err());
    }

    #[test]
    fn refuses_sudden_changes_and_out_of_range_counts() {
        let dir = tempfile::tempdir().unwrap();
        assert!(fixture(dir.path(), Some(("malware.set", 3))).is_ok());
        let dir = tempfile::tempdir().unwrap();
        assert!(fixture(dir.path(), Some(("malware.set", 10))).unwrap_err().contains("last bundle had 10"));
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("raw")).unwrap();
        fixture(dir.path(), None).unwrap();
        fs::write(dir.path().join("raw/bad"), "evil.example\n").unwrap();
        let sources = dir.path().join("sources.json");
        let youtube = dir.path().join("youtube.json");
        let raw = dir.path().join("raw");
        let err = prepare(&Options {
            sources: &sources,
            raw: &raw,
            youtube: &youtube,
            theme_revocations: None,
            previous: None,
            allow_change: false,
            out: &dir.path().join("o2"),
        })
        .unwrap_err();
        assert!(err.contains("refusing to sign"), "{err}");
    }
}
