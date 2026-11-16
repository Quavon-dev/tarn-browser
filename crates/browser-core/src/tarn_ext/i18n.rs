use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

const MAX_MESSAGES_BYTES: u64 = 1 << 20;
const MAX_MESSAGES: usize = 4096;
const MAX_MESSAGE: usize = 8192;

pub type Messages = BTreeMap<String, String>;

fn refuse<T>(why: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("This Tarn extension cannot be installed: {why}."))
}

/// One `messages.json`, keyed by lowercase name, with named placeholders replaced by their
/// content; `$1`..`$9` and `$$` stay for `i18n.getMessage`.
pub fn parse_messages(json: &Value) -> Result<Messages, String> {
    let Some(entries) = json.as_object().filter(|e| e.len() <= MAX_MESSAGES) else { return refuse("a messages.json is not a short object") };
    let mut out = Messages::new();
    for (name, entry) in entries {
        let valid_name = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'@');
        let Some(message) = entry.get("message").and_then(Value::as_str).filter(|m| valid_name && m.len() <= MAX_MESSAGE) else {
            return refuse(format!("the message \"{name}\" is invalid"));
        };
        let placeholders: BTreeMap<String, String> = entry
            .get("placeholders")
            .and_then(Value::as_object)
            .map(|p| p.iter().filter_map(|(k, v)| Some((k.to_ascii_lowercase(), v.get("content")?.as_str()?.to_owned()))).collect())
            .unwrap_or_default();
        out.insert(name.to_ascii_lowercase(), expand(message, &placeholders));
    }
    Ok(out)
}

fn expand(message: &str, placeholders: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let named = after.find('$').map(|end| &after[..end]).filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        match named.and_then(|n| placeholders.get(&n.to_ascii_lowercase()).map(|c| (n.len(), c))) {
            Some((len, content)) => {
                out.push_str(content);
                rest = &after[len + 1..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn candidates(ui: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for locale in ui {
        let full = locale.replace('-', "_");
        let short = full.split('_').next().unwrap_or_default().to_owned();
        for l in [full, short] {
            if !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !out.contains(&l) {
                out.push(l);
            }
        }
    }
    out
}

fn read(dir: &Path, locale: &str) -> Result<Option<Messages>, String> {
    let path = dir.join("_locales").join(locale).join("messages.json");
    let Ok(meta) = std::fs::symlink_metadata(&path) else { return Ok(None) };
    if !meta.is_file() || meta.len() > MAX_MESSAGES_BYTES {
        return refuse(format!("_locales/{locale}/messages.json is too large"));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let json: Value =
        serde_json::from_slice(&bytes).map_err(|_| format!("This Tarn extension cannot be installed: _locales/{locale}/messages.json is not JSON."))?;
    parse_messages(&json).map(Some)
}

/// The messages for the first of `ui` the package has, over its default locale's, and that locale.
pub fn load(dir: &Path, default_locale: Option<&str>, ui: &[String]) -> Result<(String, Messages), String> {
    let has_locales = dir.join("_locales").is_dir();
    let Some(default) = default_locale else {
        return if has_locales { refuse("_locales needs \"default_locale\"") } else { Ok(("en".to_owned(), Messages::new())) };
    };
    if candidates(&[default.to_owned()]).first().map(String::as_str) != Some(&default.replace('-', "_")) {
        return refuse("\"default_locale\" is invalid");
    }
    let Some(mut messages) = read(dir, default)? else { return refuse(format!("_locales/{default}/messages.json is missing")) };
    for locale in candidates(ui) {
        if let Some(chosen) = read(dir, &locale)? {
            messages.extend(chosen);
            return Ok((locale, messages));
        }
    }
    Ok((default.to_owned(), messages))
}

pub fn localize(text: &str, messages: &Messages) -> String {
    match text.strip_prefix("__MSG_").and_then(|t| t.strip_suffix("__")) {
        Some(name) => messages.get(&name.to_ascii_lowercase()).cloned().unwrap_or_default(),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placeholders_expand_and_positional_ones_stay() {
        let m = parse_messages(&json!({
            "Greeting": {"message": "Hello $USER$, $1 costs $$5", "placeholders": {"user": {"content": "$1 ($2)"}}},
            "plain": {"message": "x $unknown$ y"}
        }))
        .unwrap();
        assert_eq!(m["greeting"], "Hello $1 ($2), $1 costs $$5");
        assert_eq!(m["plain"], "x $unknown$ y");
        assert!(parse_messages(&json!({"bad name": {"message": "x"}})).is_err());
        assert!(parse_messages(&json!({"n": {"text": "x"}})).is_err());
    }

    #[test]
    fn the_ui_locale_wins_over_the_default() {
        let dir = tempfile::tempdir().unwrap();
        for (locale, text) in [("en", r#"{"a":{"message":"A"},"b":{"message":"B"}}"#), ("de", r#"{"a":{"message":"Ä"}}"#)] {
            std::fs::create_dir_all(dir.path().join("_locales").join(locale)).unwrap();
            std::fs::write(dir.path().join("_locales").join(locale).join("messages.json"), text).unwrap();
        }
        let (locale, m) = load(dir.path(), Some("en"), &["de-AT".to_owned()]).unwrap();
        assert_eq!((locale.as_str(), m["a"].as_str(), m["b"].as_str()), ("de", "Ä", "B"));
        assert_eq!(load(dir.path(), Some("en"), &["fr".to_owned()]).unwrap().0, "en");
        assert!(load(dir.path(), None, &[]).is_err());
        assert!(load(dir.path(), Some("fr"), &[]).is_err());
        assert_eq!(localize("__MSG_A__", &m), "Ä");
        assert_eq!(localize("Name", &m), "Name");
    }
}
