use std::rc::Rc;

use engine::favicon;
use engine::page_scripts::{self, PICKER};
use nsh_core::JsonValue;
use serde_json::json;
use shield::ShieldLevel;
use url::Url;
use webkit6::prelude::*;
use webkit6::{UserContentInjectedFrames, UserContentManager, UserScript, UserScriptInjectionTime, gio};

use super::State;

pub const WORLD: &str = "tarn";
const PICK_CANCEL: &str = "if (window.tarnPick) window.tarnPick(); return '';";

pub fn locked(state: &State) -> bool {
    let page = state.page.borrow();
    state.level.get() == ShieldLevel::Lockdown || page.as_ref().is_some_and(|p| state.levels.borrow().level(p) == ShieldLevel::Lockdown)
}

fn run(state: &Rc<State>, body: Option<String>, answer: impl FnOnce(&State, String, String) + 'static) {
    let url = state.page.borrow().as_ref().map(Url::to_string).unwrap_or_default();
    let (Some(web), Some(body)) = (state.web().filter(|_| !locked(state)), body) else {
        return answer(state, url, String::new());
    };
    let weak = Rc::downgrade(state);
    web.call_async_javascript_function(&body, None, Some(WORLD), None, None::<&gio::Cancellable>, move |result| {
        let Some(state) = weak.upgrade() else { return };
        let text = result.ok().filter(|v| v.is_string()).map(|v| v.to_str().to_string()).unwrap_or_default();
        answer(&state, url, text);
    });
}

fn seq(args: &JsonValue) -> JsonValue {
    args["seq"].as_u64().map_or(JsonValue::Null, JsonValue::from)
}

pub fn command(state: &Rc<State>, name: &str, args: &JsonValue) -> bool {
    let seq = seq(args);
    let page = state.page.borrow().clone();
    let origin = page.as_ref().map(|p| p.origin().ascii_serialization()).filter(|o| o != "null");
    match name {
        "readArticle" => {
            let probe = args["probe"].as_bool().unwrap_or(false);
            let body = origin.map(|o| engine::reader::script(&o, probe));
            run(state, body, move |s, url, text| s.emit("article", json!({ "seq": seq, "url": url, "text": text })));
        }
        "readText" => {
            let max = args["maxChars"].as_u64().unwrap_or(0).min(store::MAX_HISTORY_TEXT as u64) as usize;
            let body = page.as_ref().filter(|_| max > 0).and_then(|p| page_scripts::page_text_script(p, max));
            run(state, body, move |s, url, text| s.emit("pageText", json!({ "seq": seq, "url": url, "text": text })));
        }
        "findText" => {
            let query = args["query"].as_str().unwrap_or_default();
            let body = (!query.is_empty()).then(|| page_scripts::find_text_script(query));
            run(state, body, move |s, url, text| s.emit("textFound", json!({ "seq": seq, "url": url, "snippets": page_scripts::find_snippets(&text) })));
        }
        "readIconLinks" => {
            let body = page.as_ref().and_then(page_scripts::icon_links_script);
            run(state, body, move |s, url, text| s.emit("iconLinks", json!({ "seq": seq, "url": url, "text": text })));
        }
        "fetchIcon" => {
            let urls = favicon::fetch_urls(args["urls"].as_array().into_iter().flatten().filter_map(|u| u.as_str()));
            let asked = args["page"].as_str().and_then(|p| Url::parse(p).ok());
            fetch_icon(state, seq, asked, &urls);
        }
        "readJson" => {
            let max = args["maxBytes"].as_u64().unwrap_or(0).min(devkit::json_view::MAX_BYTES as u64) as usize;
            let body = page.as_ref().filter(|_| max > 0).and_then(|p| page_scripts::json_text_script(p, max));
            run(state, body, move |s, url, text| s.emit("jsonText", json!({ "seq": seq, "url": url, "text": text })));
        }
        "pick" => run(state, Some(PICKER.to_owned()), move |s, url, selector| s.emit("picked", json!({ "seq": seq, "url": url, "selector": selector }))),
        "pickCancel" => run(state, Some(PICK_CANCEL.to_owned()), |_, _, _| {}),
        _ => return false,
    }
    true
}

fn shows(state: &State, asked: Option<&Url>) -> bool {
    asked.is_some_and(|a| state.page.borrow().as_ref().is_some_and(|p| p.origin() == a.origin()))
}

fn fetch_icon(state: &Rc<State>, seq: JsonValue, asked: Option<Url>, urls: &[Url]) {
    let (Some((first, rest)), true) = (urls.split_first(), shows(state, asked.as_ref())) else {
        return run(state, None, move |s, url, _| s.emit("iconData", json!({ "seq": seq, "url": url, "text": "" })));
    };
    let body = asked.as_ref().and_then(|p| page_scripts::fetch_icon_script(p, std::slice::from_ref(first), favicon::MAX_BYTES));
    let (weak, rest) = (Rc::downgrade(state), rest.to_vec());
    run(state, body, move |s, url, text| match weak.upgrade() {
        Some(state) if text.is_empty() && shows(s, asked.as_ref()) => fetch_icon(&state, seq, asked, &rest),
        _ => s.emit("iconData", json!({ "seq": seq, "url": url, "text": if shows(s, asked.as_ref()) { text } else { String::new() } })),
    });
}

pub fn styles(value: &JsonValue) -> Vec<(String, String)> {
    let mut styles: Vec<(String, String)> = value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(host, css)| Some((host.clone(), css.as_str()?.to_owned())))
        .filter(|(host, css)| !host.is_empty() && css.len() <= store::MAX_SITE_CSS + page_scripts::DARK_CSS.len() + 1)
        .collect();
    styles.sort();
    styles
}

pub fn theme_styles(value: &JsonValue) -> Vec<(String, String)> {
    page_scripts::theme_sheets(value)
}

pub fn zaps(value: &JsonValue) -> Vec<(String, String)> {
    let mut zaps: Vec<(String, String)> = value
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(host, list)| {
            list.as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str())
                .filter(|s| shield::lists::valid_zap(s))
                .map(|s| (host.clone(), s.to_owned()))
                .take(shield::lists::MAX_ZAPS_PER_SITE)
        })
        .collect();
    zaps.sort();
    zaps
}

pub fn unlocked<'a>(state: &State, entries: &'a [(String, String)]) -> impl Iterator<Item = &'a (String, String)> {
    let levels = state.levels.borrow().clone();
    let open = state.level.get() != ShieldLevel::Lockdown;
    entries.iter().filter(move |(host, _)| open && Url::parse(&format!("https://{host}/")).is_ok_and(|u| levels.level(&u) != ShieldLevel::Lockdown))
}

pub fn apply_styles(state: &State, content: &UserContentManager) {
    if let Some(old) = state.style_script.take() {
        content.remove_script(&old);
    }
    let themed = state.theme_styles.borrow().clone().into_iter().filter(|_| state.level.get() != ShieldLevel::Lockdown);
    let styles: Vec<(String, String)> = themed.chain(unlocked(state, &state.styles.borrow()).cloned()).collect();
    if let Some(source) = page_scripts::site_styles_script(&styles).map(|s| page_scripts::outside_lockdown(s, &state.levels.borrow())) {
        let script = UserScript::for_world(&source, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, WORLD, &[], &[]);
        content.add_script(&script);
        *state.style_script.borrow_mut() = Some(script);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_and_zaps_keep_only_valid_entries() {
        let s = styles(&json!({ "b.test": "p{}", "a.test": "q{}", "": "x", "c.test": 3 }));
        assert_eq!(s, [("a.test".to_owned(), "q{}".to_owned()), ("b.test".to_owned(), "p{}".to_owned())]);
        let z = zaps(&json!({ "a.test": ["div.ad", "</style>", 5], "b.test": "x" }));
        assert_eq!(z, [("a.test".to_owned(), "div.ad".to_owned())]);
        let t = theme_styles(&json!([["*", "a{}"], ["b.test,*.c.test", "b{}"], ["", "x"], ["a b", "y"], [1, "z"], ["d.test"]]));
        assert_eq!(t, [("*".to_owned(), "a{}".to_owned()), ("b.test,*.c.test".to_owned(), "b{}".to_owned())], "theme sheets keep their order");
        let big = "p".repeat(store::MAX_SITE_CSS);
        assert_eq!(theme_styles(&json!([["*", big], ["e.test", "x"]])).len(), 1);
    }
}
