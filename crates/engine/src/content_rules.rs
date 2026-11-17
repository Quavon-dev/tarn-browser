use regex::{Regex, RegexBuilder};
use serde_json::Value;
use url::{Host, Url};

const REGEX_SIZE_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    Subdocument,
    Image,
    StyleSheet,
    Script,
    Font,
    Media,
    Fetch,
    WebSocket,
    Ping,
    Other,
}

impl ResourceKind {
    fn matches(self, name: &str) -> bool {
        match name {
            "document" | "svg-document" => matches!(self, Self::Document | Self::Subdocument),
            "image" => self == Self::Image,
            "style-sheet" => self == Self::StyleSheet,
            "script" => self == Self::Script,
            "font" => self == Self::Font,
            "media" => self == Self::Media,
            "fetch" => self == Self::Fetch,
            "websocket" => self == Self::WebSocket,
            "ping" => self == Self::Ping,
            "other" => self == Self::Other,
            "raw" => matches!(self, Self::Fetch | Self::WebSocket | Self::Other),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block,
    MakeHttps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Block,
    MakeHttps,
    IgnorePrevious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadType {
    Any,
    FirstParty,
    ThirdParty,
}

#[derive(Debug)]
struct Rule {
    url: Regex,
    kinds: Vec<String>,
    load: LoadType,
    if_domain: Vec<String>,
    unless_domain: Vec<String>,
    if_top: Vec<Regex>,
    unless_top: Vec<Regex>,
    /// `load-context`, read only on a document rule: whether it is for top-level documents.
    top_frame: Option<bool>,
    action: Action,
}

#[derive(Debug, Default)]
pub struct RuleList {
    rules: Vec<Rule>,
}

pub struct Request<'a> {
    pub url: &'a Url,
    pub top: Option<&'a Url>,
    pub kind: ResourceKind,
}

impl<'a> Request<'a> {
    /// A top-level document is judged as its own page, as in WebKit, never as the page it replaces.
    pub fn new(url: &'a Url, top: Option<&'a Url>, kind: ResourceKind) -> Self {
        Request { url, top: if kind == ResourceKind::Document { Some(url) } else { top }, kind }
    }
}

impl RuleList {
    /// Rules whose action this evaluator cannot apply (cosmetic, cookie) or whose trigger it
    /// does not understand are skipped, so an unknown field never widens what is blocked. An
    /// exception (`ignore-previous-rules`) with such a trigger is skipped too, so it can only
    /// block more than WebKit would, never less.
    pub fn parse(json: &str) -> Result<Self, String> {
        let Value::Array(items) = serde_json::from_str(json).map_err(|e| e.to_string())? else {
            return Err("rule list is not an array".into());
        };
        Ok(RuleList { rules: items.iter().filter_map(rule).collect() })
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn evaluate(&self, context: &Context) -> Verdict {
        let mut verdict = Verdict::Allow;
        for rule in self.rules.iter().filter(|r| r.matches(context)) {
            verdict = match (rule.action, verdict) {
                (Action::IgnorePrevious, _) => Verdict::Allow,
                (Action::Block, _) | (Action::MakeHttps, Verdict::Block) => Verdict::Block,
                (Action::MakeHttps, _) => Verdict::MakeHttps,
            };
        }
        verdict
    }
}

/// Each list is evaluated on its own, as WebKit does, and a block in any list wins.
pub fn evaluate<'a>(lists: impl IntoIterator<Item = &'a RuleList>, request: &Request) -> Verdict {
    let context = Context {
        url: request.url.as_str(),
        kind: request.kind,
        top_url: request.top.map_or("", Url::as_str),
        top_host: request.top.and_then(Url::host_str).map(|h| h.trim_end_matches('.').to_ascii_lowercase()),
        first_party: request.top.is_some_and(|top| same_site(top, request.url)),
    };
    let mut verdict = Verdict::Allow;
    for list in lists {
        match list.evaluate(&context) {
            Verdict::Block => return Verdict::Block,
            Verdict::MakeHttps => verdict = Verdict::MakeHttps,
            Verdict::Allow => {}
        }
    }
    verdict
}

const TRIGGER_FIELDS: &[&str] =
    &["url-filter", "url-filter-is-case-sensitive", "resource-type", "load-type", "if-domain", "unless-domain", "if-top-url", "unless-top-url", "load-context"];

fn rule(item: &Value) -> Option<Rule> {
    let trigger = item.get("trigger")?.as_object()?;
    if trigger.keys().any(|k| !TRIGGER_FIELDS.contains(&k.as_str())) {
        return None;
    }
    let action = match item.get("action")?.get("type")?.as_str()? {
        "block" => Action::Block,
        "make-https" => Action::MakeHttps,
        "ignore-previous-rules" => Action::IgnorePrevious,
        _ => return None,
    };
    let case_sensitive = trigger.get("url-filter-is-case-sensitive").and_then(Value::as_bool).unwrap_or(false);
    let url = pattern(trigger.get("url-filter")?.as_str()?, case_sensitive)?;
    let load = match trigger.get("load-type").map(strings) {
        None => LoadType::Any,
        Some(types) if types.len() == 1 && types[0] == "first-party" => LoadType::FirstParty,
        Some(types) if types.len() == 1 && types[0] == "third-party" => LoadType::ThirdParty,
        Some(types) if types.is_empty() => LoadType::Any,
        Some(_) => return None,
    };
    let kinds = trigger.get("resource-type").map(strings).unwrap_or_default();
    let top_frame = match trigger.get("load-context").map(strings) {
        None => None,
        Some(context) if kinds == ["document"] && context == ["top-frame"] => Some(true),
        Some(context) if kinds == ["document"] && context == ["child-frame"] => Some(false),
        Some(_) => return None,
    };
    let patterns = |key: &str| -> Option<Vec<Regex>> { trigger.get(key).map_or(Some(Vec::new()), |v| strings(v).iter().map(|p| pattern(p, false)).collect()) };
    let domains = |key: &str| trigger.get(key).map(strings).unwrap_or_default().iter().map(|d| d.to_ascii_lowercase()).collect();
    Some(Rule {
        url,
        kinds,
        load,
        if_domain: domains("if-domain"),
        unless_domain: domains("unless-domain"),
        if_top: patterns("if-top-url")?,
        unless_top: patterns("unless-top-url")?,
        top_frame,
        action,
    })
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

fn pattern(source: &str, case_sensitive: bool) -> Option<Regex> {
    RegexBuilder::new(source).case_insensitive(!case_sensitive).size_limit(REGEX_SIZE_LIMIT).build().ok()
}

struct Context<'a> {
    url: &'a str,
    kind: ResourceKind,
    top_url: &'a str,
    top_host: Option<String>,
    first_party: bool,
}

impl Rule {
    fn matches(&self, context: &Context) -> bool {
        if !self.kinds.is_empty() && !self.kinds.iter().any(|k| context.kind.matches(k)) {
            return false;
        }
        if self.top_frame.is_some_and(|top| top != (context.kind == ResourceKind::Document)) {
            return false;
        }
        if !self.url.is_match(context.url) {
            return false;
        }
        if self.load != LoadType::Any && context.first_party != (self.load == LoadType::FirstParty) {
            return false;
        }
        let top_domain_in = |list: &[String]| context.top_host.as_deref().is_some_and(|host| list.iter().any(|d| domain_matches(d, host)));
        if !self.if_domain.is_empty() && !top_domain_in(&self.if_domain) {
            return false;
        }
        if top_domain_in(&self.unless_domain) {
            return false;
        }
        if !self.if_top.is_empty() && !self.if_top.iter().any(|r| r.is_match(context.top_url)) {
            return false;
        }
        !self.unless_top.iter().any(|r| r.is_match(context.top_url))
    }
}

fn domain_matches(entry: &str, host: &str) -> bool {
    match entry.strip_prefix('*') {
        Some(domain) => host.strip_suffix(domain).is_some_and(|rest| rest.is_empty() || rest.ends_with('.')),
        None => host == entry,
    }
}

fn same_site(top: &Url, url: &Url) -> bool {
    match (top.host(), url.host()) {
        (Some(Host::Domain(a)), Some(Host::Domain(b))) => site(a) == site(b),
        (a, b) => a.is_some() && a == b,
    }
}

fn site(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    psl::domain_str(&host).map_or_else(|| host.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shield::{ShieldLevel, rules};

    fn verdict(lists: &[RuleList], url: &str, top: Option<&str>, kind: ResourceKind) -> Verdict {
        let url = Url::parse(url).unwrap();
        let top = top.map(|t| Url::parse(t).unwrap());
        evaluate(lists, &Request { url: &url, top: top.as_ref(), kind })
    }

    fn builtin(level: ShieldLevel) -> RuleList {
        let json = rules::builtin_rules(&level.policy());
        let list = RuleList::parse(&json).unwrap();
        let source: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(list.len(), source.len(), "every built-in rule is understood");
        list
    }

    #[test]
    fn builtin_rules_block_local_network_from_public_pages_only() {
        let lists = [builtin(ShieldLevel::Standard)];
        let public = Some("https://example.com/");
        for target in [
            "http://127.0.0.1:8080/",
            "http://192.168.1.1/",
            "http://[::1]/",
            "http://user@10.0.0.1/",
            "http://localhost./",
            "http://127.1/",
            "http://0x7f.1/",
            "http://2130706433/",
            "http://127.0.0.1./",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert_eq!(verdict(&lists, target, public, ResourceKind::Fetch), Verdict::Block, "{target}");
        }
        assert_eq!(verdict(&lists, "http://127.0.0.1/", Some("http://localhost:3000/"), ResourceKind::Fetch), Verdict::Allow);
        assert_eq!(verdict(&lists, "http://192.168.1.1/", Some("http://10.0.0.5:8080/"), ResourceKind::Fetch), Verdict::Allow);
        assert_eq!(verdict(&lists, "http://127.0.0.1/pixel", None, ResourceKind::Image), Verdict::Block);
        assert_eq!(verdict(&lists, "https://cdn.example.net/a.js", public, ResourceKind::Script), Verdict::Allow);
    }

    #[test]
    fn strict_upgrades_http_and_standard_does_not() {
        let url = "http://example.com/a.css";
        assert_eq!(verdict(&[builtin(ShieldLevel::Strict)], url, None, ResourceKind::StyleSheet), Verdict::MakeHttps);
        assert_eq!(verdict(&[builtin(ShieldLevel::Standard)], url, None, ResourceKind::StyleSheet), Verdict::Allow);
        assert_eq!(verdict(&[builtin(ShieldLevel::Strict)], "http://10.0.0.1/", Some("https://example.com/"), ResourceKind::Image), Verdict::Block);
    }

    #[test]
    fn filter_list_chunks_block_with_types_party_and_exceptions() {
        let chunks = shield::lists::compile(&["||tracker.example^$third-party\n||ads.example^$script\n@@||ads.example/ok.js\n"], &[]).chunks;
        let lists: Vec<RuleList> = chunks.iter().map(|c| RuleList::parse(c).unwrap()).collect();
        let top = Some("https://news.test/");
        assert_eq!(verdict(&lists, "https://tracker.example/p.gif", top, ResourceKind::Image), Verdict::Block);
        assert_eq!(verdict(&lists, "https://tracker.example/p.gif", Some("https://www.tracker.example/"), ResourceKind::Image), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://ads.example/x.js", top, ResourceKind::Script), Verdict::Block);
        assert_eq!(verdict(&lists, "https://ads.example/x.png", top, ResourceKind::Image), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://ads.example/ok.js", top, ResourceKind::Script), Verdict::Allow);
    }

    #[test]
    fn domain_conditions_follow_the_top_document() {
        let json = r#"[{"trigger":{"url-filter":"widget","if-domain":["*shop.test"]},"action":{"type":"block"}},
            {"trigger":{"url-filter":"beacon","unless-domain":["home.test"]},"action":{"type":"block"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        assert_eq!(verdict(&lists, "https://w.test/widget", Some("https://a.shop.test/"), ResourceKind::Script), Verdict::Block);
        assert_eq!(verdict(&lists, "https://w.test/widget", Some("https://other.test/"), ResourceKind::Script), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://w.test/beacon", Some("https://home.test/"), ResourceKind::Ping), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://w.test/beacon", Some("https://away.test/"), ResourceKind::Ping), Verdict::Block);
    }

    #[test]
    fn an_extension_grant_is_the_hosts_whatever_the_scheme() {
        let rules = shield::dnr::compile(&[br#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"ads.js"}}]"#]).unwrap();
        let json = rules.content_rules(&shield::dnr::Scope::Hosts(vec!["shop.test".into()])).unwrap();
        let lists = [RuleList::parse(&json).unwrap()];
        for top in ["https://shop.test/", "http://shop.test/"] {
            assert_eq!(verdict(&lists, "https://cdn.test/ads.js", Some(top), ResourceKind::Script), Verdict::Block, "{top}");
        }
        assert_eq!(verdict(&lists, "https://cdn.test/ads.js", Some("http://other.test/"), ResourceKind::Script), Verdict::Allow);
    }

    #[test]
    fn a_top_level_document_is_judged_as_its_own_page() {
        let json = r#"[{"trigger":{"url-filter":"x","if-top-url":["^https?://shop\\.test[:/]"]},"action":{"type":"block"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        let (shop, away) = (Url::parse("https://shop.test/x").unwrap(), Url::parse("https://away.test/x").unwrap());
        let judge = |url: &Url, top: &Url, kind| evaluate(&lists, &Request::new(url, Some(top), kind));
        assert_eq!(judge(&shop, &away, ResourceKind::Document), Verdict::Block);
        assert_eq!(judge(&away, &shop, ResourceKind::Document), Verdict::Allow);
        assert_eq!(judge(&away, &shop, ResourceKind::Subdocument), Verdict::Block);
    }

    #[test]
    fn ip_hosts_are_their_own_site() {
        let json = r#"[{"trigger":{"url-filter":"track","load-type":["third-party"]},"action":{"type":"block"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        assert_eq!(verdict(&lists, "http://192.168.0.1/track", Some("http://10.0.0.1/"), ResourceKind::Image), Verdict::Block);
        assert_eq!(verdict(&lists, "http://10.0.0.1/track", Some("http://10.0.0.1:8080/"), ResourceKind::Image), Verdict::Allow);
        assert_eq!(verdict(&lists, "http://[::1]/track", Some("http://[::2]/"), ResourceKind::Image), Verdict::Block);
        assert_eq!(verdict(&lists, "https://cdn.news.co.uk/track", Some("https://www.news.co.uk/"), ResourceKind::Image), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://other.co.uk/track", Some("https://www.news.co.uk/"), ResourceKind::Image), Verdict::Block);
    }

    #[test]
    fn a_request_without_a_top_page_is_third_party() {
        let json = r#"[{"trigger":{"url-filter":"track","load-type":["third-party"]},"action":{"type":"block"}},
            {"trigger":{"url-filter":"own","load-type":["first-party"]},"action":{"type":"block"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        assert_eq!(verdict(&lists, "https://tracker.test/track", None, ResourceKind::Fetch), Verdict::Block);
        assert_eq!(verdict(&lists, "https://site.test/own", None, ResourceKind::Fetch), Verdict::Allow);
    }

    #[test]
    fn document_rules_tell_top_level_documents_from_frames() {
        let json = r#"[{"trigger":{"url-filter":"ads","resource-type":["document"],"load-context":["child-frame"]},"action":{"type":"block"}},
            {"trigger":{"url-filter":"bad","resource-type":["document"],"load-context":["top-frame"]},"action":{"type":"block"}},
            {"trigger":{"url-filter":"any","resource-type":["document","script"],"load-context":["top-frame"]},"action":{"type":"block"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        assert_eq!(lists[0].len(), 2, "load-context is read on document rules only");
        assert_eq!(verdict(&lists, "https://ads.test/", None, ResourceKind::Subdocument), Verdict::Block);
        assert_eq!(verdict(&lists, "https://ads.test/", None, ResourceKind::Document), Verdict::Allow);
        assert_eq!(verdict(&lists, "https://bad.test/", None, ResourceKind::Document), Verdict::Block);
        assert_eq!(verdict(&lists, "https://bad.test/", None, ResourceKind::Subdocument), Verdict::Allow);
    }

    #[test]
    fn unsupported_rules_are_skipped_not_widened() {
        let json = r#"[{"trigger":{"url-filter":".*"},"action":{"type":"css-display-none","selector":".ad"}},
            {"trigger":{"url-filter":".*","load-context":["child-frame"]},"action":{"type":"block"}},
            {"trigger":{"url-filter":"(unclosed"},"action":{"type":"block"}}]"#;
        let list = RuleList::parse(json).unwrap();
        assert!(list.is_empty());
        let json = r#"[{"trigger":{"url-filter":"ads"},"action":{"type":"block"}},
            {"trigger":{"url-filter":"ads/ok","load-context":["child-frame"]},"action":{"type":"ignore-previous-rules"}}]"#;
        let lists = [RuleList::parse(json).unwrap()];
        assert_eq!(verdict(&lists, "https://a.test/ads/ok", None, ResourceKind::Script), Verdict::Block, "an exception it cannot read keeps the block");
        assert!(RuleList::parse("{}").is_err());
    }
}
