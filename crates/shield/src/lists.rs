//! Converts EasyList-style filter lists into WebKit content rule lists (S20).

use std::collections::{BTreeSet, HashMap, HashSet};

use adblock::content_blocking::CbType;
use adblock::lists::{FilterSet, ParseOptions, RuleTypes};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Host;

/// WebKit's documented per-list limit; current WebKit (`ContentExtensionParser.cpp`, `maxRuleCount`)
/// allows 150,000, but 50,000 also holds on older releases.
pub const MAX_RULES_PER_LIST: usize = 50_000;

/// Bumped when the conversion changes, so stored lists are rebuilt.
/// Includes the pinned `adblock` version, whose conversion output can change between releases.
const COMPILER_VERSION: &[u8] = b"tarn-shield-lists-2/adblock-0.13.5";

/// Element hiding applies to every page, so generic rules are capped harder than site rules.
pub const MAX_GENERIC_HIDE: usize = 10_000;
pub const MAX_SITE_HIDE: usize = 40_000;
const MAX_SELECTOR: usize = 256;
/// Procedural and extended syntax that WebKit's CSS cannot express, plus `:has()`, which can make
/// every style recalculation on a page expensive.
const UNSUPPORTED_SELECTOR: &[&str] = &[
    ":has",
    ":-abp-",
    ":xpath",
    ":contains",
    ":matches-",
    ":upward",
    ":remove",
    ":style",
    ":min-text-length",
    ":watch-attr",
    ":others",
    ":if",
    ":nth-ancestor",
    ":shadow",
    "[-ext-",
];

pub struct Compiled {
    /// Content rule list JSON, each with at most `MAX_RULES_PER_LIST` rules.
    pub chunks: Vec<String>,
    pub fingerprint: [u8; 32],
}

pub fn fingerprint(lists: &[&str]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(COMPILER_VERSION);
    for list in lists {
        hash.update((list.len() as u64).to_le_bytes());
        hash.update(list.as_bytes());
    }
    hash.finalize().into()
}

/// Network rules of `lists`. Every chunk repeats the exceptions of `lists` and of `shared`, so an
/// exception in one category still reaches block rules of another that is compiled separately.
/// Only `block` and `ignore-previous-rules` are emitted, whatever `adblock` produces.
pub fn compile(lists: &[&str], shared: &[&str]) -> Compiled {
    let (rules, _) = network_set(lists, shared, true).into_content_blocking().unwrap_or_default();
    let (mut exceptions, blocks): (Vec<_>, Vec<_>) = rules
        .into_iter()
        .filter(|r| matches!(r.action.typ, CbType::Block | CbType::IgnorePreviousRules))
        .partition(|r| matches!(r.action.typ, CbType::IgnorePreviousRules));

    // ignore-previous-rules only reaches rules in its own list, so every chunk repeats the
    // exceptions after its block rules. shortcut: beyond half the limit exceptions are dropped
    // (over-blocking), raise if real lists ever get there.
    exceptions.truncate(MAX_RULES_PER_LIST / 2);
    let exceptions: Vec<Value> = exceptions.iter().map(to_json).collect();
    let chunks = blocks
        .chunks(MAX_RULES_PER_LIST - exceptions.len())
        .map(|chunk| {
            let rules: Vec<Value> = chunk.iter().map(to_json).chain(exceptions.iter().cloned()).collect();
            Value::Array(rules).to_string()
        })
        .collect();
    Compiled { chunks, fingerprint: network_key(lists, shared) }
}

/// The network rules of `lists` plus only the exceptions of `shared`; `debug` keeps the rule text
/// that content-blocking conversion needs.
pub(crate) fn network_set(lists: &[&str], shared: &[&str], debug: bool) -> FilterSet {
    let opts = || ParseOptions { rule_types: RuleTypes::NetworkOnly, ..ParseOptions::default() };
    let mut set = FilterSet::new(debug);
    for list in lists {
        set.add_filter_list((*list).to_owned(), opts());
    }
    for list in shared {
        let exceptions: String = list.lines().filter(|l| l.starts_with("@@")).flat_map(|l| [l, "\n"]).collect();
        set.add_filter_list(exceptions, opts());
    }
    set
}

/// What `compile` would name its output, without compiling.
pub fn network_key(lists: &[&str], shared: &[&str]) -> [u8; 32] {
    fingerprint(&[lists, &["\0shared"], shared].concat())
}

pub fn cosmetic_key(lists: &[&str]) -> [u8; 32] {
    fingerprint(&[lists, &["\0cosmetic"]].concat())
}

fn valid_selector(selector: &str) -> bool {
    !selector.is_empty()
        && selector.len() <= MAX_SELECTOR
        && !selector.starts_with(['+', '^'])
        && selector.bytes().all(|b| (0x20..0x7f).contains(&b) && !matches!(b, b'{' | b'}' | b'\\'))
        && !UNSUPPORTED_SELECTOR.iter().any(|s| selector.contains(s))
        && mask(selector).is_some_and(|code| {
            !code.contains("/*") && !code.match_indices('|').any(|(i, _)| !code[i + 1..].starts_with('=')) && !code.split(',').any(page_wide)
        })
}

/// The selector as its structure: quoted values, the insides of parentheses and each attribute's
/// operator and value left out, and no whitespace inside attribute brackets, so none can hide a
/// combinator, comment or namespace from the checks; `None` when a quote, parenthesis or bracket
/// is left open.
fn mask(selector: &str) -> Option<String> {
    let (mut quote, mut parens, mut brackets, mut value) = (None, 0usize, 0usize, false);
    let mut out = String::new();
    let mut chars = selector.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => {
                if parens == 0 {
                    out.push(c);
                }
                parens += 1;
            }
            ')' => {
                parens = parens.checked_sub(1)?;
                if parens == 0 {
                    out.push(c);
                }
            }
            _ if parens > 0 => {}
            '[' => {
                brackets += 1;
                out.push(c);
            }
            ']' => {
                brackets = brackets.checked_sub(1)?;
                value = false;
                out.push(c);
            }
            _ if brackets > 0 && (value || c.is_whitespace()) => {}
            '=' if brackets > 0 => value = true,
            '~' | '^' | '$' | '*' | '|' if brackets > 0 && chars.peek() == Some(&'=') => value = true,
            _ => out.push(c),
        }
    }
    (quote.is_none() && parens == 0 && brackets == 0).then_some(out)
}

/// A masked selector part that hides the whole page or its frame, which no list may do (S20):
/// the subject may not be the root, body or head element with any qualifier, everything, or an
/// attribute that nearly every root element carries.
fn page_wide(part: &str) -> bool {
    let subject = part.trim().rsplit([' ', '>', '+', '~']).next().unwrap_or_default().to_ascii_lowercase();
    let element = subject.split(['.', '#', '[', ':']).next().unwrap_or_default();
    let rest = &subject[element.len()..];
    let root_attribute = ["[lang", "[dir", "[class", "[style", "[xmlns", "[data-theme"].iter().any(|a| rest.starts_with(a));
    let hides_its_subject = [":root", ":is(", ":where("].iter().any(|p| rest.contains(p));
    matches!(element, "html" | "body" | "head" | "*")
        || hides_its_subject
        || (element.is_empty() && (rest.is_empty() || rest.starts_with(':') || root_attribute))
}

/// A selector the element picker may store: compounds of a lowercase tag, `#id`, `.class` and
/// `:nth-of-type(n)`, joined by ` > `, none naming the root, body or head except a leading
/// `body > ` anchor. Stricter than `valid_selector`, so a stored pick names one element's path
/// rather than any selector a list may use (S20).
pub fn valid_zap(selector: &str) -> bool {
    let steps = selector.strip_prefix("body > ").unwrap_or(selector);
    valid_selector(selector) && steps.split(" > ").all(zap_compound)
}

fn zap_compound(part: &str) -> bool {
    let ident = |s: &str| {
        s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let end = part.find(['#', '.', ':']).unwrap_or(part.len());
    let (tag, mut rest) = part.split_at(end);
    let tag_ok =
        tag.is_empty() || (tag.starts_with(|c: char| c.is_ascii_lowercase()) && tag.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    if part.is_empty() || !tag_ok || matches!(tag, "html" | "body" | "head") {
        return false;
    }
    while !rest.is_empty() {
        if let Some((n, after)) = rest.strip_prefix(":nth-of-type(").and_then(|r| r.split_once(')')) {
            if n.is_empty() || n.len() > 4 || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            rest = after;
        } else if let Some(name) = rest.strip_prefix(['#', '.']) {
            let end = name.find(['#', '.', ':']).unwrap_or(name.len());
            if !ident(&name[..end]) {
                return false;
            }
            rest = &name[end..];
        } else {
            return false;
        }
    }
    true
}

/// The most elements one profile may hide on one host.
pub const MAX_ZAPS_PER_SITE: usize = 200;

/// One host's zapped elements as a style sheet, for engines without content rule lists; `None`
/// when no selector is valid. Stored rows are checked again, as in `compile_zaps`.
pub fn zap_css<'a>(selectors: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let rules: Vec<String> =
        selectors.into_iter().filter(|s| valid_zap(s)).take(MAX_ZAPS_PER_SITE).map(|s| format!("{s} {{ display: none !important; }}")).collect();
    (!rules.is_empty()).then(|| rules.join("\n"))
}

/// The zapped elements that may hide anything: a plain lowercase host and a selector of the
/// picker's shape, at most `MAX_RULES_PER_LIST`. Stored rows are checked again, as they may predate
/// a rule, by every engine.
pub fn kept_zaps<'a>(zaps: impl IntoIterator<Item = (&'a str, &'a str)>) -> impl Iterator<Item = (&'a str, &'a str)> {
    zaps.into_iter()
        .filter(|(host, selector)| !host.is_empty() && host.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_')) && valid_zap(selector))
        .take(MAX_RULES_PER_LIST)
}

/// One profile's zapped elements as a content rule list, each hidden only on its own host;
/// `None` when nothing valid is left.
pub fn compile_zaps<'a>(zaps: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<String> {
    let rules: Vec<Value> = kept_zaps(zaps).map(|(host, selector)| hide_rule(selector, &[host.to_owned()], &[])).collect();
    (!rules.is_empty()).then(|| Value::Array(rules).to_string())
}

/// `a.com,~b.com` as (if-domain, unless-domain) in WebKit's `*domain` form; `None` when a domain
/// is not a plain hostname (entity rules like `google.*`) or both kinds are mixed.
fn rule_domains(list: &str) -> Option<(Vec<String>, Vec<String>)> {
    let (mut include, mut exclude) = (Vec::new(), Vec::new());
    for raw in list.split(',').filter(|d| !d.is_empty()) {
        let (target, raw) = match raw.strip_prefix('~') {
            Some(rest) => (&mut exclude, rest),
            None => (&mut include, raw),
        };
        let Ok(Host::Domain(domain)) = Host::parse(raw) else { return None };
        if !domain.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_')) {
            return None;
        }
        target.push(format!("*{domain}"));
    }
    (include.is_empty() || exclude.is_empty()).then_some((include, exclude))
}

fn hide_rule(selector: &str, if_domain: &[String], unless_domain: &[String]) -> Value {
    let mut trigger = json!({ "url-filter": ".*" });
    if !if_domain.is_empty() {
        trigger["if-domain"] = json!(if_domain);
    }
    if !unless_domain.is_empty() {
        trigger["unless-domain"] = json!(unless_domain);
    }
    json!({ "trigger": trigger, "action": { "type": "css-display-none", "selector": selector } })
}

/// Element-hiding rules (`##`) as `css-display-none`, one selector per rule: WebKit drops a rule
/// whose selector it cannot parse and keeps the rest. Scriptlets, procedural selectors and
/// HTML filters are never emitted (S20). Generic rules carry the domains that unhide them (`#@#`).
pub fn compile_cosmetic(lists: &[&str]) -> Compiled {
    let (generic, site) = hide_rules(lists);
    let generic = generic.into_iter().map(|(selector, exclude)| hide_rule(selector, &[], &exclude));
    let rules: Vec<Value> = generic.chain(site.into_iter().map(|(selector, include)| hide_rule(selector, &include, &[]))).collect();
    Compiled { chunks: rules.chunks(MAX_RULES_PER_LIST).map(|c| Value::Array(c.to_vec()).to_string()).collect(), fingerprint: cosmetic_key(lists) }
}

/// Element hiding for engines without content rule lists: the rules `compile_cosmetic` emits, as
/// style sheets by host. A domain matches itself and its subdomains, as WebKit's `*domain`.
#[derive(Default)]
pub struct HideSheets {
    /// The generic rules no domain unhides, one CSS rule per selector, so a selector the engine
    /// cannot parse drops only its own rule.
    generic: String,
    unless: Vec<(String, Vec<String>)>,
    site: HashMap<String, Vec<String>>,
}

fn push_hide(css: &mut String, selector: &str) {
    css.push_str(selector);
    css.push_str("{display:none!important}\n");
}

impl HideSheets {
    pub fn new(lists: &[&str]) -> Self {
        let domains = |list: Vec<String>| list.into_iter().map(|d| d.trim_start_matches('*').to_owned()).collect::<Vec<_>>();
        let mut sheets = HideSheets::default();
        let (generic, site) = hide_rules(lists);
        // A leading `@` would make the rule an at-rule in a style sheet; WebKit parses it as a selector.
        for (selector, exclude) in generic.into_iter().filter(|(s, _)| !s.trim_start().starts_with('@')) {
            if exclude.is_empty() {
                push_hide(&mut sheets.generic, selector);
            } else {
                sheets.unless.push((selector.to_owned(), domains(exclude)));
            }
        }
        for (selector, include) in site.into_iter().filter(|(s, _)| !s.trim_start().starts_with('@')) {
            for domain in domains(include) {
                sheets.site.entry(domain).or_default().push(selector.to_owned());
            }
        }
        sheets
    }

    /// The style sheet for a page on `host`; empty when no rule applies.
    pub fn css(&self, host: &str) -> String {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let on = |domain: &str| host.strip_suffix(domain).is_some_and(|rest| rest.is_empty() || rest.ends_with('.'));
        let mut css = self.generic.clone();
        for (selector, _) in self.unless.iter().filter(|(_, exclude)| !exclude.iter().any(|d| on(d))) {
            push_hide(&mut css, selector);
        }
        let suffixes = std::iter::once(host.as_str()).chain(host.match_indices('.').map(|(i, _)| &host[i + 1..]));
        for selector in suffixes.filter_map(|d| self.site.get(d)).flatten() {
            push_hide(&mut css, selector);
        }
        css
    }
}

type Selectors<'a> = Vec<(&'a str, Vec<String>)>;

/// The element-hiding rules of `lists` with valid selectors and plain domains: generic ones (at
/// most `MAX_GENERIC_HIDE`) with the domains that unhide them, and site ones with their domains,
/// both in WebKit's `*domain` form.
fn hide_rules<'a>(lists: &[&'a str]) -> (Selectors<'a>, Selectors<'a>) {
    let mut unhide: HashMap<&str, BTreeSet<String>> = HashMap::new();
    let mut generic: Vec<(&str, BTreeSet<String>)> = Vec::new();
    let mut generic_index: HashMap<&str, usize> = HashMap::new();
    let mut site: Vec<(&str, Vec<String>)> = Vec::new();
    let mut site_seen: HashSet<(&str, &str)> = HashSet::new();
    for line in lists.iter().flat_map(|l| l.lines()).map(str::trim) {
        if let Some((domains, selector)) = line.split_once("#@#") {
            if let (true, Some((include, _))) = (valid_selector(selector), rule_domains(domains)) {
                unhide.entry(selector).or_default().extend(include);
            }
            continue;
        }
        let Some((domains, selector)) = line.split_once("##") else { continue };
        if line.starts_with('!') || !valid_selector(selector) {
            continue;
        }
        let Some((include, exclude)) = rule_domains(domains) else { continue };
        if include.is_empty() {
            let ix = *generic_index.entry(selector).or_insert_with(|| {
                generic.push((selector, BTreeSet::new()));
                generic.len() - 1
            });
            generic[ix].1.extend(exclude);
        } else if site.len() < MAX_SITE_HIDE && site_seen.insert((domains, selector)) {
            site.push((selector, include));
        }
    }
    let generic = generic.into_iter().take(MAX_GENERIC_HIDE).map(|(selector, mut exclude)| {
        exclude.extend(unhide.get(selector).into_iter().flatten().cloned());
        (selector, exclude.into_iter().collect())
    });
    (generic.collect(), site)
}

/// `resource-type` comes from a `HashSet`; sorting it keeps the output byte-identical across runs.
fn to_json(rule: &adblock::content_blocking::CbRule) -> Value {
    let mut value = serde_json::to_value(rule).unwrap_or_default();
    if let Some(Value::Array(types)) = value.pointer_mut("/trigger/resource-type") {
        types.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rule_hides_the_whole_page() {
        for bad in [
            "html",
            "body",
            "*",
            ":root",
            "HTML",
            "div > body",
            "body:not(.x)",
            "a.ad, body",
            "html[lang]",
            "[lang]",
            "html.js",
            "body.home",
            "body[class]",
            ":not(.x)",
            "*.x",
            "body:not(.x .y)",
            "body:is(.a .b)",
            "body:not([x=\">\"])",
            "html[x=\"+\"]",
            "a\" /*",
            "div:not(.a",
            "[x=\"y\"",
            "body[class~=x]",
            "html[class~=\"x\"]",
            "body[class~= \"x\"]",
            "[lang~=en]",
            "[*|lang]",
            "html[lang^=e]",
            ".js:root",
            "#x:root",
            ".a:is(html)",
            ".a:where(body)",
            "div .js:root",
            "body/**/",
            "html/**/",
            ".ad/**/",
            "*|body",
            "*|html",
            "*|*",
            "svg|a",
            "[ lang]",
            "[lang ]",
            "[class ]",
            "[ dir = ltr ]",
        ] {
            assert!(!super::valid_selector(bad), "{bad}");
        }
        for ok in [
            ".ad",
            "#banner",
            "body > .ad",
            "div[data-ad]",
            "html.dark .ad",
            "iframe",
            "[data-ad-slot]",
            "a[href*=\"ads\"]",
            "div[lang|=\"en\"]",
            "a[title=\"x y\"]",
            "a[href*=\"x|y\"]",
            "a[href$='/*']",
            ".ad:not(.x .y)",
            "div:not([x=\">\"]) .ad",
            ".ad:not(.x)",
            ".js .ad",
        ] {
            assert!(super::valid_selector(ok), "{ok}");
        }
    }

    #[test]
    fn zaps_allow_only_picker_selectors() {
        for bad in [
            "body",
            "html",
            "head",
            "*",
            ":root",
            "body/**/",
            "*|body",
            "[ lang]",
            "[lang]",
            "div > body",
            "html > .x",
            "body > body",
            "body > body > p",
            "body >",
            "body > ",
            "div > body > p",
            "p > body > div",
            "body > *",
            "body > *|body",
            "div .x",
            "div>p",
            ".a, .b",
            "a[href]",
            "p:hover",
            "p:nth-of-type(0)",
            "p:nth-of-type(n)",
            "p:nth-of-type(12345)",
            "#1a",
            ".-x",
            "DIV",
            "Body",
            "div >",
            "",
            " > p",
            "p > > a",
            ".a:has(.b)",
            "p:nth-child(2)",
            "*.x",
            "svg|a",
            "div\\.x",
        ] {
            assert!(!valid_zap(bad), "{bad}");
        }
        for ok in [
            "div.ad",
            "#banner",
            "main > div:nth-of-type(2) > aside.promo",
            "custom-el",
            ".a_b-c",
            "#x > p:nth-of-type(12)",
            "h1",
            "body > div:nth-of-type(2) > p",
        ] {
            assert!(valid_zap(ok), "{ok}");
        }
    }

    #[test]
    fn zap_css_hides_only_valid_selectors() {
        assert_eq!(
            zap_css([".ad", "body", "#top > p:nth-of-type(2)", "a{}"]).as_deref(),
            Some(".ad { display: none !important; }\n#top > p:nth-of-type(2) { display: none !important; }")
        );
        assert_eq!(zap_css(["html", "p { color: red }"]), None);
        assert_eq!(zap_css(std::iter::repeat_n(".x", MAX_ZAPS_PER_SITE + 1)).unwrap().lines().count(), MAX_ZAPS_PER_SITE);
    }

    #[test]
    fn zaps_hide_on_their_own_host_only() {
        let json = compile_zaps([("a.example", ".ad"), ("b.example", "body"), ("[::1]", ".x"), ("c.example", "#top > p:nth-of-type(2)")]).unwrap();
        let all: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(all.len(), 2, "{json}");
        assert_eq!(all[0]["trigger"]["if-domain"], json!(["a.example"]));
        assert_eq!(all[0]["action"], json!({ "type": "css-display-none", "selector": ".ad" }));
        assert_eq!(all[1]["trigger"]["if-domain"], json!(["c.example"]));
        assert!(compile_zaps([("a.example", "html")]).is_none());
    }

    #[test]
    fn compiler_version_names_the_pinned_adblock() {
        let pin = include_str!("../Cargo.toml").lines().find_map(|l| l.strip_prefix("adblock = { version = \"=")).and_then(|l| l.split('"').next()).unwrap();
        assert!(COMPILER_VERSION.ends_with(format!("adblock-{pin}").as_bytes()));
    }

    const SAMPLE: &str = "[Adblock Plus 2.0]\n! comment\n||tracker.example^\n@@||ok.example^\n##.ad\nexample.com##.banner\n||ads.example^$script\n";

    fn rules(c: &Compiled) -> Vec<Value> {
        c.chunks.iter().flat_map(|s| serde_json::from_str::<Vec<Value>>(s).unwrap()).collect()
    }

    #[test]
    fn converts_network_rules_and_drops_cosmetic() {
        let compiled = compile(&[SAMPLE], &[]);
        assert_eq!(compiled.chunks.len(), 1);
        let all = rules(&compiled);
        let text = Value::Array(all.clone()).to_string();
        assert!(text.contains("tracker\\\\.example"), "{text}");
        assert!(text.contains("ok\\\\.example"), "{text}");
        assert!(!text.contains("css-display-none"), "{text}");
        let block = all.iter().position(|r| r["trigger"]["url-filter"].as_str().unwrap().contains("tracker")).unwrap();
        let allow = all.iter().position(|r| r["trigger"]["url-filter"].as_str().unwrap().contains("ok")).unwrap();
        assert_eq!(all[block]["action"]["type"], "block");
        assert_eq!(all[allow]["action"]["type"], "ignore-previous-rules");
        assert!(block < allow);
    }

    #[test]
    fn output_is_deterministic() {
        let a = compile(&[SAMPLE, "||x.example^$image,script,stylesheet,font,media"], &[]);
        let b = compile(&[SAMPLE, "||x.example^$image,script,stylesheet,font,media"], &[]);
        assert_eq!(a.chunks, b.chunks);
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn fingerprint_tracks_inputs() {
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        assert_ne!(fingerprint(&[SAMPLE]), fingerprint(&[SAMPLE, ""]));
    }

    #[test]
    fn splits_below_limit_and_repeats_exceptions() {
        let mut list: String = (0..MAX_RULES_PER_LIST + 10).map(|i| format!("||t{i}.example^\n")).collect();
        list.push_str("@@||ok.example^\n");
        let compiled = compile(&[&list], &[]);
        assert_eq!(compiled.chunks.len(), 2);
        for chunk in &compiled.chunks {
            let rules: Vec<Value> = serde_json::from_str(chunk).unwrap();
            assert!(rules.len() <= MAX_RULES_PER_LIST);
            assert!(chunk.contains("ok\\\\.example"));
        }
    }

    #[test]
    fn shared_exceptions_reach_other_categories_and_change_the_fingerprint() {
        let own = compile(&["||tracker.example^\n"], &[]);
        let shared = compile(&["||tracker.example^\n"], &["@@||ok.example^\n||other.example^\n"]);
        let text = Value::Array(rules(&shared)).to_string();
        assert!(text.contains("ok\\\\.example") && !text.contains("other"), "{text}");
        assert_ne!(own.fingerprint, shared.fingerprint);
    }

    #[test]
    fn only_block_and_exception_actions_are_emitted() {
        let compiled = compile(&["||a.example^$removeparam=x\n||b.example^$csp=script-src 'none'\n||c.example^\n@@||d.example^\n"], &[]);
        for rule in rules(&compiled) {
            let action = rule["action"]["type"].as_str().unwrap().to_owned();
            assert!(action == "block" || action == "ignore-previous-rules", "{rule}");
        }
    }

    #[test]
    fn cosmetic_rules_keep_plain_selectors_only() {
        let list = "##.ad-banner\nexample.com##.promo\n~good.example##.sponsor\ngood.example#@#.ad-banner\n\
            ##+js(set, x, 1)\n##div:has(> .ad)\nexample.com##.x:-abp-contains(ad)\n##^script\ngoogle.*##.ad\n\
            a.example,~b.example##.mixed\n##.ok\\:x\n! ##.comment\n##.ad-banner\n";
        let long = format!("##.{}", "a".repeat(MAX_SELECTOR));
        let compiled = compile_cosmetic(&[list, &long]);
        let all = rules(&compiled);
        let text = Value::Array(all.clone()).to_string();
        assert_eq!(all.len(), 3, "{text}");
        assert!(all.iter().all(|r| r["action"]["type"] == "css-display-none"));
        assert_eq!(all[0]["action"]["selector"], ".ad-banner");
        assert_eq!(all[0]["trigger"]["unless-domain"], json!(["*good.example"]));
        assert_eq!(all[1]["trigger"]["unless-domain"], json!(["*good.example"]));
        assert_eq!(all[2]["trigger"]["if-domain"], json!(["*example.com"]));
        for absent in ["js(", ":has", "abp", "script", "google", "mixed", "aaaa", "comment"] {
            assert!(!text.contains(absent), "{absent}: {text}");
        }
    }

    #[test]
    fn hide_sheets_follow_the_content_rules_by_host() {
        let list = "##.ad\n##.sponsor\n~good.example##.sponsor\ngood.example#@#.ad\nexample.com##.promo\nexample.com##.promo\n\
            ##div:has(> .ad)\n##@media all\n##body\na.example,~b.example##.mixed\nnews.test,blog.test##.box\n";
        let sheets = HideSheets::new(&[list]);
        let rules = |host: &str| sheets.css(host).lines().map(str::to_owned).collect::<Vec<_>>();
        let hide = |s: &str| format!("{s}{{display:none!important}}");
        assert_eq!(rules("other.test"), [hide(".ad"), hide(".sponsor")]);
        assert_eq!(rules("www.good.example"), Vec::<String>::new());
        assert_eq!(rules("example.com"), [hide(".ad"), hide(".sponsor"), hide(".promo")]);
        assert_eq!(rules("WWW.Example.COM."), rules("example.com"));
        assert_eq!(rules("notexample.com"), rules("other.test"));
        assert_eq!(rules("blog.test"), [hide(".ad"), hide(".sponsor"), hide(".box")]);
        assert!(HideSheets::default().css("example.com").is_empty());
    }

    #[test]
    fn generic_hiding_is_capped() {
        let list: String = (0..MAX_GENERIC_HIDE + 5).map(|i| format!("##.ad-{i}\n")).collect();
        assert_eq!(rules(&compile_cosmetic(&[&list])).len(), MAX_GENERIC_HIDE);
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(compile(&["! only a comment\n##.ad"], &[]).chunks.is_empty());
    }
}
