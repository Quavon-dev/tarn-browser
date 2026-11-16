//! Static `declarative_net_request` rules of Tarn extensions, compiled into one content rule list
//! per extension. Block and upgrade rules take effect only on top-level sites the extension holds
//! a grant for (`Scope`); allow rules only lift the same extension's rules, since every list is
//! evaluated on its own and a block in any list wins.

use adblock::content_blocking::CbType;
use adblock::lists::{FilterSet, ParseOptions, RuleTypes};
use regex::RegexBuilder;
use serde_json::{Map, Value, json};

use crate::lists::MAX_RULES_PER_LIST;

pub const MAX_RULES: usize = 30_000;
pub const MAX_FILES: usize = 5;
pub const MAX_FILE_BYTES: usize = 4 << 20;
const MAX_FILTER: usize = 2048;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
const RULE_KEYS: &[&str] = &["id", "priority", "action", "condition"];
const CONDITION_KEYS: &[&str] = &[
    "urlFilter",
    "regexFilter",
    "isUrlFilterCaseSensitive",
    "domains",
    "initiatorDomains",
    "excludedInitiatorDomains",
    "requestDomains",
    "resourceTypes",
    "excludedResourceTypes",
    "domainType",
];
const RESOURCE_TYPES: &[&str] = &[
    "main_frame",
    "sub_frame",
    "stylesheet",
    "script",
    "image",
    "font",
    "object",
    "xmlhttprequest",
    "ping",
    "csp_report",
    "media",
    "websocket",
    "webtransport",
    "webbundle",
    "other",
];

/// The top-level sites an extension's block and upgrade rules may act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Every,
    Hosts(Vec<String>),
}

#[derive(Clone, Debug, Default)]
pub struct Rules {
    gated: Vec<Gated>,
    exceptions: Vec<Value>,
    count: usize,
}

#[derive(Clone, Debug)]
struct Gated {
    rule: Value,
    include: Vec<String>,
    exclude: Vec<String>,
}

/// Resource types sharing one trigger, with the trigger's `load-context` if it needs one.
type TypeGroup = (Vec<&'static str>, Option<&'static str>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Block,
    Allow,
    Upgrade,
    AllowAll,
}

fn refuse<T>(id: i64, why: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("declarative_net_request rule {id}: {why}"))
}

/// Parses and checks every enabled rule file; any rule outside Tarn's subset refuses the whole
/// extension, naming the rule and the key.
pub fn compile(files: &[&[u8]]) -> Result<Rules, String> {
    if files.len() > MAX_FILES {
        return Err(format!("declarative_net_request: at most {MAX_FILES} rule files"));
    }
    let mut out = Rules::default();
    for file in files {
        if file.len() > MAX_FILE_BYTES {
            return Err("declarative_net_request: a rule file is larger than 4 MiB".to_owned());
        }
        let Ok(Value::Array(rules)) = serde_json::from_slice::<Value>(file) else {
            return Err("declarative_net_request: a rule file is not a JSON array".to_owned());
        };
        out.count += rules.len();
        if out.count > MAX_RULES {
            return Err(format!("declarative_net_request: more than {MAX_RULES} rules"));
        }
        for rule in &rules {
            add_rule(&mut out, rule)?;
        }
    }
    if out.gated.len() + out.exceptions.len() > MAX_RULES_PER_LIST / 2 {
        return Err("declarative_net_request: the rules expand to too many entries".to_owned());
    }
    Ok(out)
}

fn add_rule(out: &mut Rules, rule: &Value) -> Result<(), String> {
    let Some(object) = rule.as_object() else { return Err("declarative_net_request: a rule is not an object".to_owned()) };
    let id = object.get("id").and_then(Value::as_i64).filter(|id| *id >= 1).ok_or("declarative_net_request: a rule has no valid \"id\"")?;
    if let Some(key) = object.keys().find(|k| !RULE_KEYS.contains(&k.as_str())) {
        return refuse(id, format!("\"{key}\" is not supported"));
    }
    if object.get("priority").is_some_and(|p| p.as_i64().is_none_or(|p| p < 1)) {
        return refuse(id, "\"priority\" must be a positive integer");
    }
    let action = object.get("action").and_then(Value::as_object).ok_or_else(|| format!("declarative_net_request rule {id}: no \"action\""))?;
    let kind = match action.get("type").and_then(Value::as_str) {
        Some("block") => Kind::Block,
        Some("allow") => Kind::Allow,
        Some("upgradeScheme") => Kind::Upgrade,
        Some("allowAllRequests") => Kind::AllowAll,
        Some(other) => return refuse(id, format!("action type \"{other}\" is not supported")),
        None => return refuse(id, "no action \"type\""),
    };
    if let Some(key) = action.keys().find(|k| *k != "type") {
        return refuse(id, format!("action \"{key}\" is not supported"));
    }
    let empty = Map::new();
    let condition = match object.get("condition") {
        None => &empty,
        Some(Value::Object(c)) => c,
        Some(_) => return refuse(id, "\"condition\" is not an object"),
    };
    if let Some(key) = condition.keys().find(|k| !CONDITION_KEYS.contains(&k.as_str())) {
        return refuse(id, format!("condition \"{key}\" is not supported"));
    }
    let c = Condition::read(id, condition)?;
    let groups = c.types(id, kind)?;
    if kind == Kind::AllowAll {
        let Some(filter) = c.url_filters(id)?.into_iter().next() else { return refuse(id, "allowAllRequests needs a urlFilter or regexFilter") };
        out.exceptions.push(json!({"trigger": {"url-filter": ".*", "if-top-url": [filter.0]}, "action": {"type": "ignore-previous-rules"}}));
        return Ok(());
    }
    for (url_filter, case_sensitive) in c.url_filters(id)? {
        for (types, context) in &groups {
            let mut trigger = Map::new();
            trigger.insert("url-filter".into(), url_filter.clone().into());
            if case_sensitive {
                trigger.insert("url-filter-is-case-sensitive".into(), true.into());
            }
            trigger.insert("resource-type".into(), types.iter().map(|t| Value::from(*t)).collect());
            if let Some(context) = context {
                trigger.insert("load-context".into(), json!([context]));
            }
            if let Some(party) = c.party {
                trigger.insert("load-type".into(), json!([party]));
            }
            match kind {
                Kind::Allow => {
                    domains_into(&mut trigger, &c.include, &c.exclude);
                    out.exceptions.push(json!({"trigger": trigger, "action": {"type": "ignore-previous-rules"}}));
                }
                _ => {
                    let action = if kind == Kind::Upgrade { "make-https" } else { "block" };
                    out.gated.push(Gated {
                        rule: json!({"trigger": trigger, "action": {"type": action}}),
                        include: c.include.clone(),
                        exclude: c.exclude.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn domains_into(trigger: &mut Map<String, Value>, include: &[String], exclude: &[String]) {
    let wild = |list: &[String]| list.iter().map(|d| Value::from(format!("*{d}"))).collect::<Value>();
    if !include.is_empty() {
        trigger.insert("if-domain".into(), wild(include));
    } else if !exclude.is_empty() {
        trigger.insert("unless-domain".into(), wild(exclude));
    }
}

struct Condition {
    url_filter: Option<String>,
    regex_filter: Option<String>,
    case_sensitive: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    requests: Vec<String>,
    types: Option<Vec<String>>,
    excluded_types: Option<Vec<String>>,
    party: Option<&'static str>,
}

impl Condition {
    fn read(id: i64, c: &Map<String, Value>) -> Result<Self, String> {
        let text = |key: &str| -> Result<Option<String>, String> {
            match c.get(key) {
                None => Ok(None),
                Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_FILTER && s.bytes().all(|b| b.is_ascii_graphic()) => Ok(Some(s.clone())),
                Some(_) => refuse(id, format!("\"{key}\" must be printable ASCII without spaces")),
            }
        };
        let domains = |key: &str| -> Result<Vec<String>, String> {
            let Some(value) = c.get(key) else { return Ok(Vec::new()) };
            let list: Option<Vec<String>> =
                value.as_array().and_then(|a| a.iter().map(|d| d.as_str().filter(|d| valid_domain(d)).map(str::to_owned)).collect());
            list.filter(|l| !l.is_empty()).map_or_else(|| refuse(id, format!("\"{key}\" must list lowercase domain names")), Ok)
        };
        let names = |key: &str| -> Result<Option<Vec<String>>, String> {
            let Some(value) = c.get(key) else { return Ok(None) };
            let list: Option<Vec<String>> =
                value.as_array().and_then(|a| a.iter().map(|t| t.as_str().filter(|t| RESOURCE_TYPES.contains(t)).map(str::to_owned)).collect());
            list.map_or_else(|| refuse(id, format!("\"{key}\" lists an unknown resource type")), |l| Ok(Some(l)))
        };
        let mut include = domains("domains")?;
        include.extend(domains("initiatorDomains")?);
        let party = match c.get("domainType").map(|v| v.as_str()) {
            None => None,
            Some(Some("firstParty")) => Some("first-party"),
            Some(Some("thirdParty")) => Some("third-party"),
            Some(_) => return refuse(id, "\"domainType\" must be firstParty or thirdParty"),
        };
        let case_sensitive = match c.get("isUrlFilterCaseSensitive") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return refuse(id, "\"isUrlFilterCaseSensitive\" must be true or false"),
        };
        let condition = Condition {
            url_filter: text("urlFilter")?,
            regex_filter: text("regexFilter")?,
            case_sensitive,
            include,
            exclude: domains("excludedInitiatorDomains")?,
            requests: domains("requestDomains")?,
            types: names("resourceTypes")?,
            excluded_types: names("excludedResourceTypes")?,
            party,
        };
        if condition.url_filter.is_some() && condition.regex_filter.is_some() {
            return refuse(id, "\"urlFilter\" and \"regexFilter\" together are not supported");
        }
        if !condition.requests.is_empty() && (condition.url_filter.is_some() || condition.regex_filter.is_some()) {
            return refuse(id, "\"requestDomains\" with a urlFilter or regexFilter is not supported");
        }
        if condition.types.is_some() && condition.excluded_types.is_some() {
            return refuse(id, "\"resourceTypes\" and \"excludedResourceTypes\" together are not supported");
        }
        if !condition.include.is_empty() && !condition.exclude.is_empty() {
            return refuse(id, "\"excludedInitiatorDomains\" with \"initiatorDomains\" or \"domains\" is not supported");
        }
        Ok(condition)
    }

    /// The rule's resource types in the triggers they need: WebKit names main and sub frames alike
    /// `document`, so a rule for only one of them gets a document trigger of its own with that
    /// `load-context`. Without `resourceTypes` a rule leaves out top-level documents, as in Chrome.
    fn types(&self, id: i64, kind: Kind) -> Result<Vec<TypeGroup>, String> {
        let chosen: Vec<&str> = match (&self.types, &self.excluded_types) {
            (Some(t), _) => t.iter().map(String::as_str).collect(),
            (None, Some(x)) => RESOURCE_TYPES.iter().copied().filter(|t| !x.iter().any(|e| e == t)).collect(),
            (None, None) => RESOURCE_TYPES.iter().copied().filter(|t| *t != "main_frame").collect(),
        };
        if kind == Kind::AllowAll && (chosen.is_empty() || chosen.iter().any(|t| !matches!(*t, "main_frame" | "sub_frame"))) {
            return refuse(id, "allowAllRequests applies to main_frame and sub_frame only");
        }
        let (main, sub) = (chosen.contains(&"main_frame"), chosen.contains(&"sub_frame"));
        let mut out: Vec<&'static str> = chosen
            .into_iter()
            .filter(|t| !matches!(*t, "main_frame" | "sub_frame"))
            .map(|t| match t {
                "stylesheet" => "style-sheet",
                "script" => "script",
                "image" => "image",
                "font" => "font",
                "media" => "media",
                "xmlhttprequest" => "fetch",
                "websocket" => "websocket",
                "ping" | "csp_report" => "ping",
                _ => "other",
            })
            .collect();
        if main && sub {
            out.push("document");
        }
        out.sort_unstable();
        out.dedup();
        let mut groups: Vec<TypeGroup> = Vec::new();
        if !out.is_empty() {
            groups.push((out, None));
        }
        match (main, sub) {
            (true, false) => groups.push((vec!["document"], Some("top-frame"))),
            (false, true) => groups.push((vec!["document"], Some("child-frame"))),
            _ => {}
        }
        if groups.is_empty() {
            return refuse(id, "the rule matches no resource type");
        }
        Ok(groups)
    }

    fn url_filters(&self, id: i64) -> Result<Vec<(String, bool)>, String> {
        if let Some(regex) = &self.regex_filter {
            return webkit_regex(regex)
                .map_or_else(|| refuse(id, "\"regexFilter\" cannot be compiled by Shield"), |()| Ok(vec![(regex.clone(), self.case_sensitive)]));
        }
        if let Some(filter) = &self.url_filter {
            if filter.contains('$') || filter.starts_with("@@") {
                return refuse(id, "\"urlFilter\" cannot be compiled by Shield");
            }
            let line = if filter.len() > 1 && filter.starts_with('/') && filter.ends_with('/') { format!("{filter}*") } else { filter.clone() };
            let line = if self.case_sensitive { format!("{line}$match-case") } else { line };
            return Ok(vec![convert(id, &line, "urlFilter")?]);
        }
        if !self.requests.is_empty() {
            return self.requests.iter().map(|d| convert(id, &format!("||{d}^"), "requestDomains")).collect();
        }
        Ok(vec![(".*".to_owned(), false)])
    }
}

fn convert(id: i64, line: &str, key: &str) -> Result<(String, bool), String> {
    let mut set = FilterSet::new(true);
    set.add_filter_list(line.to_owned(), ParseOptions { rule_types: RuleTypes::NetworkOnly, ..ParseOptions::default() });
    let (rules, _) = set.into_content_blocking().unwrap_or_default();
    let blocks: Vec<_> = rules.into_iter().filter(|r| r.action.typ == CbType::Block).collect();
    match blocks.as_slice() {
        [rule] => Ok((rule.trigger.url_filter.clone(), rule.trigger.url_filter_is_case_sensitive.unwrap_or(false))),
        _ => refuse(id, format!("\"{key}\" cannot be compiled by Shield")),
    }
}

/// WebKit's content blockers know no alternation, counted repetition, character-class escapes or
/// group modifiers.
fn webkit_regex(source: &str) -> Option<()> {
    let unsupported = ["|", "{", "}", "(?", "\\d", "\\D", "\\w", "\\W", "\\s", "\\S", "\\b", "\\B"];
    if unsupported.iter().any(|s| source.contains(s)) {
        return None;
    }
    RegexBuilder::new(source).size_limit(REGEX_SIZE_LIMIT).build().ok().map(|_| ())
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with('.')
        && domain.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn covers(domain: &str, host: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.'))
}

impl Rules {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The content rule list for `scope`, `None` when no block or upgrade rule applies anywhere.
    pub fn content_rules(&self, scope: &Scope) -> Option<String> {
        let mut rules = Vec::new();
        for g in &self.gated {
            let mut rule = g.rule.clone();
            let Some(trigger) = rule.get_mut("trigger").and_then(Value::as_object_mut) else { continue };
            match scope {
                Scope::Every => domains_into(trigger, &g.include, &g.exclude),
                Scope::Hosts(hosts) => {
                    let kept: Vec<Value> = hosts
                        .iter()
                        .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
                        .filter(|h| (g.include.is_empty() || g.include.iter().any(|d| covers(d, h))) && !g.exclude.iter().any(|d| covers(d, h)))
                        .map(Value::from)
                        .collect();
                    if kept.is_empty() {
                        continue;
                    }
                    trigger.insert("if-domain".into(), Value::Array(kept));
                }
            }
            rules.push(rule);
        }
        if rules.is_empty() {
            return None;
        }
        rules.extend(self.exceptions.iter().cloned());
        Some(Value::Array(rules).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(json: &str) -> Result<Rules, String> {
        compile(&[json.as_bytes()])
    }

    fn list(rules: &Rules, scope: &Scope) -> Vec<Value> {
        rules.content_rules(scope).map(|s| serde_json::from_str::<Vec<Value>>(&s).unwrap()).unwrap_or_default()
    }

    #[test]
    fn redirect_modify_headers_and_unknown_keys_are_refused_by_name() {
        let cases = [
            (r#"[{"id":1,"action":{"type":"redirect","redirect":{"url":"https://x.test/"}}}]"#, "\"redirect\""),
            (r#"[{"id":2,"action":{"type":"modifyHeaders"}}]"#, "\"modifyHeaders\""),
            (r#"[{"id":3,"action":{"type":"block"},"condition":{"tabIds":[1]}}]"#, "\"tabIds\""),
            (r#"[{"id":4,"action":{"type":"block"},"extra":1}]"#, "\"extra\""),
            (r#"[{"id":5,"action":{"type":"block"},"condition":{"urlFilter":"a","regexFilter":"b"}}]"#, "\"urlFilter\""),
            (r#"[{"id":6,"action":{"type":"block"},"condition":{"regexFilter":"a|b"}}]"#, "\"regexFilter\""),
            (r#"[{"id":7,"action":{"type":"block"},"condition":{"urlFilter":"a$script"}}]"#, "\"urlFilter\""),
            (r#"[{"id":8,"action":{"type":"block"},"condition":{"domains":["Bad.Test"]}}]"#, "\"domains\""),
            (r#"[{"id":9,"action":{"type":"block"},"condition":{"resourceTypes":["beacon"]}}]"#, "\"resourceTypes\""),
            (r#"[{"id":10,"action":{"type":"allowAllRequests"},"condition":{"urlFilter":"a","resourceTypes":["script"]}}]"#, "allowAllRequests"),
            (r#"[{"id":11,"action":{"type":"block"},"condition":{"excludedRequestDomains":["a.test"]}}]"#, "\"excludedRequestDomains\""),
            (
                r#"[{"id":12,"action":{"type":"block"},"condition":{"initiatorDomains":["shop.test"],"excludedInitiatorDomains":["cart.shop.test"]}}]"#,
                "\"excludedInitiatorDomains\"",
            ),
        ];
        for (json, named) in cases {
            let why = rules(json).unwrap_err();
            assert!(why.contains(named), "{json}: {why}");
        }
    }

    #[test]
    fn caps_on_files_size_and_count() {
        let one: &[u8] = br#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||a.test^"}}]"#;
        assert!(compile(&[one; MAX_FILES]).is_ok());
        assert!(compile(&[one; MAX_FILES + 1]).is_err());
        assert!(compile(&[vec![b' '; MAX_FILE_BYTES + 1].as_slice()]).is_err());
        let many: Vec<Value> = (1..=MAX_RULES + 1).map(|i| json!({"id": i, "action": {"type": "allow"}})).collect();
        assert!(rules(&Value::Array(many).to_string()).unwrap_err().contains("30000"));
    }

    #[test]
    fn block_rules_reach_granted_top_sites_only() {
        let r = rules(r#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.test^","resourceTypes":["script"]}}]"#).unwrap();
        assert_eq!(r.content_rules(&Scope::Hosts(Vec::new())), None);
        let granted = list(&r, &Scope::Hosts(vec!["news.test".into()]));
        assert_eq!(granted[0]["trigger"]["if-domain"], json!(["news.test"]));
        assert_eq!(granted[0]["trigger"]["resource-type"], json!(["script"]));
        assert_eq!(granted[0]["action"]["type"], "block");
        let every = list(&r, &Scope::Every);
        assert!(every[0]["trigger"].get("if-domain").is_none());
    }

    #[test]
    fn initiator_domains_narrow_the_grants() {
        let r = rules(
            r#"[{"id":1,"action":{"type":"upgradeScheme"},"condition":{"initiatorDomains":["shop.test"]}},
               {"id":2,"action":{"type":"block"},"condition":{"urlFilter":"ads","excludedInitiatorDomains":["cart.shop.test"]}}]"#,
        )
        .unwrap();
        let hosts = Scope::Hosts(vec!["shop.test".into(), "cart.shop.test".into(), "www.shop.test".into(), "other.test".into()]);
        let out = list(&r, &hosts);
        assert_eq!(out[0]["trigger"]["if-domain"], json!(["shop.test", "cart.shop.test", "www.shop.test"]));
        assert_eq!(out[0]["action"]["type"], "make-https");
        let block = |l: &[Value]| l.iter().find(|r| r["action"]["type"] == "block").unwrap().clone();
        assert_eq!(block(&out)["trigger"]["if-domain"], json!(["shop.test", "www.shop.test", "other.test"]));
        let every = list(&r, &Scope::Every);
        assert_eq!(every[0]["trigger"]["if-domain"], json!(["*shop.test"]));
        assert_eq!(block(&every)["trigger"]["unless-domain"], json!(["*cart.shop.test"]));
        assert!(every.iter().all(|r| r["action"]["type"] != "ignore-previous-rules"));
    }

    #[test]
    fn allow_rules_come_last_and_are_not_gated() {
        let r = rules(
            r#"[{"id":2,"action":{"type":"allow"},"condition":{"urlFilter":"||ads.test/ok"}},
               {"id":1,"action":{"type":"block"},"condition":{"requestDomains":["ads.test","t.test"]}},
               {"id":3,"action":{"type":"allowAllRequests"},"condition":{"urlFilter":"||safe.test^","resourceTypes":["main_frame"]}}]"#,
        )
        .unwrap();
        let out = list(&r, &Scope::Hosts(vec!["a.test".into()]));
        let actions: Vec<&str> = out.iter().map(|r| r["action"]["type"].as_str().unwrap()).collect();
        assert_eq!(actions, ["block", "block", "block", "block", "ignore-previous-rules", "ignore-previous-rules", "ignore-previous-rules"]);
        assert!(out[6]["trigger"]["if-top-url"].is_array());
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn frame_types_keep_top_level_documents_and_frames_apart() {
        let r = rules(
            r#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.test^","resourceTypes":["sub_frame","script"]}},
               {"id":2,"action":{"type":"block"},"condition":{"urlFilter":"||bad.test^","resourceTypes":["main_frame"]}},
               {"id":3,"action":{"type":"block"},"condition":{"urlFilter":"||all.test^","resourceTypes":["main_frame","sub_frame"]}}]"#,
        )
        .unwrap();
        let out = list(&r, &Scope::Every);
        assert_eq!(out[0]["trigger"]["resource-type"], json!(["script"]));
        assert_eq!((&out[1]["trigger"]["resource-type"], &out[1]["trigger"]["load-context"]), (&json!(["document"]), &json!(["child-frame"])));
        assert_eq!((&out[2]["trigger"]["resource-type"], &out[2]["trigger"]["load-context"]), (&json!(["document"]), &json!(["top-frame"])));
        assert_eq!(out[3]["trigger"]["resource-type"], json!(["document"]));
        assert!(out[3]["trigger"].get("load-context").is_none());
    }

    #[test]
    fn regexes_are_checked_and_default_types_leave_out_top_level_documents() {
        let r = rules(r#"[{"id":1,"action":{"type":"block"},"condition":{"regexFilter":"^https://x\\.test/a+$","isUrlFilterCaseSensitive":true}}]"#).unwrap();
        let out = list(&r, &Scope::Every);
        assert_eq!(out[0]["trigger"]["url-filter"], "^https://x\\.test/a+$");
        assert_eq!(out[0]["trigger"]["url-filter-is-case-sensitive"], true);
        let types = out[0]["trigger"]["resource-type"].as_array().unwrap();
        assert!(!types.contains(&json!("document")) && types.contains(&json!("fetch")) && out[0]["trigger"].get("load-context").is_none());
        assert_eq!(out[1]["trigger"]["resource-type"], json!(["document"]));
        assert_eq!(out[1]["trigger"]["load-context"], json!(["child-frame"]));
        assert_eq!(out.len(), 2);
        assert!(rules(r#"[{"id":1,"action":{"type":"block"},"condition":{"regexFilter":"a{2}"}}]"#).is_err());
        assert!(rules(r#"[{"id":1,"action":{"type":"block"},"condition":{"regexFilter":"(a"}}]"#).is_err());
    }
}
