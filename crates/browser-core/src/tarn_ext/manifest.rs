use serde_json::{Map, Value};

use super::patterns::Pattern;
use engine::tarn_ext::RunAt;

const TOP_KEYS: &[&str] = &[
    "manifest_version",
    "name",
    "version",
    "browser_specific_settings",
    "content_scripts",
    "host_permissions",
    "permissions",
    "optional_permissions",
    "declarative_net_request",
    "action",
    "background",
    "description",
    "author",
    "homepage_url",
    "short_name",
    "icons",
    "default_locale",
];
const SCRIPT_KEYS: &[&str] = &["matches", "exclude_matches", "js", "css", "run_at", "all_frames"];
const ACTION_KEYS: &[&str] = &["default_popup", "default_icon", "default_title"];
const BACKGROUND_KEYS: &[&str] = &["service_worker", "type"];
const PERMISSIONS: &[&str] = &["storage", "activeTab", "declarativeNetRequest"];
const MAX_CONTENT_SCRIPTS: usize = 64;
const MAX_LIST: usize = 256;

/// Whether `manifest` asks for Tarn's runtime: `manifest_version` 3 and
/// `browser_specific_settings.tarn.runtime` 1.
pub fn is_tarn(manifest: &Value) -> bool {
    manifest.get("manifest_version").and_then(Value::as_u64) == Some(3)
        && manifest.pointer("/browser_specific_settings/tarn/runtime").and_then(Value::as_u64) == Some(1)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentScript {
    pub matches: Vec<Pattern>,
    pub exclude: Vec<Pattern>,
    pub js: Vec<String>,
    pub css: Vec<String>,
    pub run_at: RunAt,
    pub all_frames: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Action {
    pub popup: Option<String>,
    pub title: Option<String>,
    pub icons: Vec<(u32, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub script: String,
    pub module: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_locale: Option<String>,
    pub content_scripts: Vec<ContentScript>,
    pub host_permissions: Vec<Pattern>,
    pub storage: bool,
    pub active_tab: bool,
    pub rule_files: Vec<String>,
    pub action: Option<Action>,
    pub background: Option<Background>,
    pub icons: Vec<(u32, String)>,
}

fn refuse<T>(why: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("This Tarn extension cannot be installed: {why}."))
}

fn unknown<T>(place: &str, key: &str) -> Result<T, String> {
    refuse(format!("{place}\"{key}\" is not supported"))
}

/// A path inside the package: relative, no `.` or `..` part, no characters a URL or file system
/// treats specially.
pub fn package_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start_matches("./").trim_start_matches('/');
    let ok = !trimmed.is_empty()
        && trimmed.len() <= 1024
        && trimmed.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && part.len() <= 255)
        && trimmed.bytes().all(|b| b.is_ascii_graphic() && !b"\\:?#%\"<>|*".contains(&b));
    ok.then(|| trimmed.to_owned())
}

fn string(v: &Value, what: &str) -> Result<String, String> {
    v.as_str().map(str::to_owned).map_or_else(|| refuse(format!("\"{what}\" must be text")), Ok)
}

fn strings(v: &Value, what: &str) -> Result<Vec<String>, String> {
    let list: Option<Vec<String>> = v.as_array().filter(|a| a.len() <= MAX_LIST).and_then(|a| a.iter().map(|s| s.as_str().map(str::to_owned)).collect());
    list.map_or_else(|| refuse(format!("\"{what}\" must be a list of text")), Ok)
}

fn paths(v: &Value, what: &str) -> Result<Vec<String>, String> {
    strings(v, what)?.iter().map(|p| package_path(p).map_or_else(|| refuse(format!("\"{what}\" names an unsafe path")), Ok)).collect()
}

fn patterns(v: &Value, what: &str) -> Result<Vec<Pattern>, String> {
    strings(v, what)?.iter().map(|p| Pattern::parse(p).map_err(|e| format!("This Tarn extension cannot be installed: {e}."))).collect()
}

fn icons(v: &Value, what: &str) -> Result<Vec<(u32, String)>, String> {
    match v {
        Value::String(s) => Ok(vec![(0, package_path(s).ok_or_else(|| format!("This Tarn extension cannot be installed: \"{what}\" names an unsafe path."))?)]),
        Value::Object(map) => {
            let mut out = Vec::new();
            for (size, path) in map.iter().take(MAX_LIST) {
                let size = size.parse::<u32>().ok().filter(|s| *s > 0 && *s <= 1024);
                let path = path.as_str().and_then(package_path);
                match (size, path) {
                    (Some(size), Some(path)) => out.push((size, path)),
                    _ => return refuse(format!("\"{what}\" lists an invalid icon")),
                }
            }
            out.sort();
            Ok(out)
        }
        _ => refuse(format!("\"{what}\" must name icons")),
    }
}

fn object<'a>(v: &'a Value, what: &str, allowed: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let Some(map) = v.as_object() else { return refuse(format!("\"{what}\" must be an object")) };
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return unknown(&format!("{what}."), key);
    }
    Ok(map)
}

/// The smallest icon at least `size` pixels wide, else the largest one.
pub fn select_icon(icons: &[(u32, String)], size: u32) -> Option<&str> {
    icons.iter().find(|(s, _)| *s >= size).or(icons.last()).map(|(_, p)| p.as_str())
}

/// Reads the subset Tarn runs; any other key, permission or value refuses the package and is named.
pub fn parse(manifest: &Value) -> Result<Manifest, String> {
    if !is_tarn(manifest) {
        return refuse("it does not ask for Tarn's runtime");
    }
    let Some(top) = manifest.as_object() else { return refuse("manifest.json is not an object") };
    if let Some(key) = top.keys().find(|k| !TOP_KEYS.contains(&k.as_str())) {
        return unknown("", key);
    }
    object(&manifest["browser_specific_settings"]["tarn"], "browser_specific_settings.tarn", &["runtime"])?;
    let field = |key: &str| top.get(key);
    let text = |key: &str| field(key).map(|v| string(v, key)).transpose();
    let name = text("name")?.filter(|n| !n.trim().is_empty()).ok_or("This Tarn extension cannot be installed: it has no \"name\".")?;
    let version = text("version")?
        .filter(|v| !v.is_empty() && v.len() <= 64 && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
        .ok_or("This Tarn extension cannot be installed: \"version\" must be dotted numbers.")?;
    let mut storage = false;
    let mut active_tab = false;
    let mut dnr = false;
    for key in ["permissions", "optional_permissions"] {
        for p in field(key).map(|v| strings(v, key)).transpose()?.unwrap_or_default() {
            match p.as_str() {
                "storage" => storage = true,
                "activeTab" => active_tab = true,
                "declarativeNetRequest" => dnr = true,
                other if !PERMISSIONS.contains(&other) => return refuse(format!("the permission \"{other}\" is not supported")),
                _ => {}
            }
        }
    }
    let mut content_scripts = Vec::new();
    if let Some(list) = field("content_scripts") {
        let Some(list) = list.as_array().filter(|l| l.len() <= MAX_CONTENT_SCRIPTS) else { return refuse("\"content_scripts\" must be a short list") };
        for entry in list {
            let entry = object(entry, "content_scripts", SCRIPT_KEYS)?;
            let get = |key: &str| entry.get(key);
            let script = ContentScript {
                matches: get("matches")
                    .map(|v| patterns(v, "matches"))
                    .transpose()?
                    .filter(|m| !m.is_empty())
                    .ok_or("This Tarn extension cannot be installed: every content script needs \"matches\".")?,
                exclude: get("exclude_matches").map(|v| patterns(v, "exclude_matches")).transpose()?.unwrap_or_default(),
                js: get("js").map(|v| paths(v, "js")).transpose()?.unwrap_or_default(),
                css: get("css").map(|v| paths(v, "css")).transpose()?.unwrap_or_default(),
                run_at: match get("run_at").map(Value::as_str) {
                    None | Some(Some("document_idle")) => RunAt::DocumentIdle,
                    Some(Some("document_start")) => RunAt::DocumentStart,
                    Some(Some("document_end")) => RunAt::DocumentEnd,
                    Some(_) => return refuse("\"run_at\" must be document_start, document_end or document_idle"),
                },
                all_frames: match get("all_frames") {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return refuse("\"all_frames\" must be true or false"),
                },
            };
            if script.js.is_empty() && script.css.is_empty() {
                return refuse("a content script names no \"js\" or \"css\"");
            }
            content_scripts.push(script);
        }
    }
    let mut rule_files = Vec::new();
    if let Some(v) = field("declarative_net_request") {
        let dnr_object = object(v, "declarative_net_request", &["rule_resources"])?;
        if !dnr {
            return refuse("\"declarative_net_request\" needs the \"declarativeNetRequest\" permission");
        }
        let Some(resources) = dnr_object.get("rule_resources").and_then(Value::as_array) else {
            return refuse("\"declarative_net_request.rule_resources\" must be a list");
        };
        for resource in resources {
            let resource = object(resource, "declarative_net_request.rule_resources", &["id", "enabled", "path"])?;
            let path = resource.get("path").and_then(Value::as_str).and_then(package_path);
            let Some(path) = path.filter(|_| resource.get("id").is_some_and(Value::is_string)) else {
                return refuse("a rule resource needs an \"id\" and a safe \"path\"");
            };
            match resource.get("enabled") {
                Some(Value::Bool(true)) => rule_files.push(path),
                Some(Value::Bool(false)) => {}
                _ => return refuse("a rule resource's \"enabled\" must be true or false"),
            }
        }
        if rule_files.len() > shield::dnr::MAX_FILES {
            return refuse(format!("at most {} rule files may be enabled", shield::dnr::MAX_FILES));
        }
    }
    let action = field("action")
        .map(|v| -> Result<Action, String> {
            let a = object(v, "action", ACTION_KEYS)?;
            Ok(Action {
                popup: a
                    .get("default_popup")
                    .map(|p| p.as_str().and_then(package_path).ok_or("This Tarn extension cannot be installed: \"default_popup\" names an unsafe path."))
                    .transpose()?,
                title: a.get("default_title").map(|t| string(t, "default_title")).transpose()?,
                icons: a.get("default_icon").map(|i| icons(i, "default_icon")).transpose()?.unwrap_or_default(),
            })
        })
        .transpose()?;
    let background = field("background")
        .map(|v| -> Result<Background, String> {
            let b = object(v, "background", BACKGROUND_KEYS)?;
            let script = b.get("service_worker").and_then(Value::as_str).and_then(package_path);
            let script = script.filter(|s| s.bytes().all(|c| c.is_ascii_alphanumeric() || b"._/-".contains(&c)));
            let Some(script) = script else { return refuse("\"background.service_worker\" must name a script by a plain path") };
            let module = match b.get("type").map(Value::as_str) {
                None | Some(Some("classic")) => false,
                Some(Some("module")) => true,
                Some(_) => return refuse("\"background.type\" must be classic or module"),
            };
            Ok(Background { script, module })
        })
        .transpose()?;
    Ok(Manifest {
        name,
        version,
        description: text("description")?.unwrap_or_default(),
        default_locale: text("default_locale")?,
        content_scripts,
        host_permissions: field("host_permissions").map(|v| patterns(v, "host_permissions")).transpose()?.unwrap_or_default(),
        storage,
        active_tab,
        rule_files,
        action,
        background,
        icons: field("icons").map(|v| icons(v, "icons")).transpose()?.unwrap_or_default(),
    })
}

impl Manifest {
    /// Every site pattern the extension wants: `host_permissions` and content-script `matches`.
    pub fn wanted(&self) -> Vec<&Pattern> {
        let mut out: Vec<&Pattern> = self.host_permissions.iter().chain(self.content_scripts.iter().flat_map(|c| &c.matches)).collect();
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out.dedup_by(|a, b| a.as_str() == b.as_str());
        out
    }

    pub fn wants_host(&self, host: &str) -> bool {
        self.wanted().iter().any(|p| p.covers_host(host))
    }

    /// What the consent prompt lists and the install row confirms: API permissions, then site patterns.
    pub fn consent(&self) -> (Vec<String>, Vec<String>) {
        let apis = [(self.storage, "storage"), (self.active_tab, "activeTab"), (!self.rule_files.is_empty(), "declarativeNetRequest")];
        let apis = apis.into_iter().filter(|(on, _)| *on).map(|(_, name)| name.to_owned()).collect();
        (apis, self.wanted().iter().map(|p| p.as_str().to_owned()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({"manifest_version": 3, "name": "X", "version": "1.2", "browser_specific_settings": {"tarn": {"runtime": 1}, "gecko": {"id": "x@y"}}})
    }

    fn with(key: &str, value: Value) -> Value {
        let mut m = base();
        m[key] = value;
        m
    }

    #[test]
    fn detection_needs_mv3_and_the_tarn_key() {
        assert!(is_tarn(&base()));
        assert!(!is_tarn(&json!({"manifest_version": 3, "name": "X"})));
        assert!(!is_tarn(&json!({"manifest_version": 2, "browser_specific_settings": {"tarn": {"runtime": 1}}})));
        assert!(!is_tarn(&json!({"manifest_version": 3, "browser_specific_settings": {"tarn": {"runtime": 2}}})));
    }

    #[test]
    fn refusals_name_the_offending_key() {
        let cases = [
            (with("web_accessible_resources", json!([])), "\"web_accessible_resources\""),
            (with("permissions", json!(["storage", "tabs"])), "\"tabs\""),
            (with("optional_permissions", json!(["scripting"])), "\"scripting\""),
            (with("content_scripts", json!([{"matches": ["<all_urls>"], "js": ["a.js"], "world": "MAIN"}])), "\"world\""),
            (with("content_scripts", json!([{"matches": ["file:///*"], "js": ["a.js"]}])), "file:///*"),
            (with("content_scripts", json!([{"matches": ["<all_urls>"], "js": ["../a.js"]}])), "\"js\""),
            (with("action", json!({"default_popup": "p.html", "browser_style": true})), "\"browser_style\""),
            (with("background", json!({"scripts": ["bg.js"]})), "\"scripts\""),
            (with("declarative_net_request", json!({"rule_resources": []})), "\"declarativeNetRequest\""),
        ];
        for (manifest, named) in cases {
            let why = parse(&manifest).unwrap_err();
            assert!(why.contains(named), "{named}: {why}");
        }
        let mut odd = base();
        odd["browser_specific_settings"]["tarn"]["extra"] = json!(1);
        assert!(parse(&odd).unwrap_err().contains("\"extra\""));
    }

    #[test]
    fn the_subset_parses_with_defaults() {
        let mut m = with("content_scripts", json!([{"matches": ["*://*.a.test/*"], "js": ["/c.js"], "css": ["./c.css"]}]));
        m["permissions"] = json!(["storage", "declarativeNetRequest"]);
        m["host_permissions"] = json!(["https://b.test/*"]);
        m["declarative_net_request"] =
            json!({"rule_resources": [{"id": "r", "enabled": true, "path": "rules.json"}, {"id": "s", "enabled": false, "path": "off.json"}]});
        m["background"] = json!({"service_worker": "bg.js", "type": "module"});
        m["action"] = json!({"default_popup": "popup.html", "default_icon": {"16": "i16.png", "48": "i48.png"}});
        m["description"] = json!("about");
        let parsed = parse(&m).unwrap();
        let script = &parsed.content_scripts[0];
        assert_eq!((script.run_at, script.all_frames), (RunAt::DocumentIdle, false));
        assert_eq!(script.js, ["c.js"]);
        assert_eq!(parsed.rule_files, ["rules.json"]);
        assert!(parsed.background.as_ref().unwrap().module && parsed.storage && !parsed.active_tab);
        assert!(parsed.wants_host("x.a.test") && parsed.wants_host("b.test") && !parsed.wants_host("c.test"));
        assert_eq!(
            parsed.consent(),
            (vec!["storage".to_owned(), "declarativeNetRequest".to_owned()], vec!["*://*.a.test/*".to_owned(), "https://b.test/*".to_owned()])
        );
        let icons = &parsed.action.as_ref().unwrap().icons;
        assert_eq!(select_icon(icons, 32), Some("i48.png"));
        assert_eq!(select_icon(icons, 16), Some("i16.png"));
        assert_eq!(select_icon(icons, 128), Some("i48.png"));
    }
}
