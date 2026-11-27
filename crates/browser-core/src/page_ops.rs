//! The shell's one-shot reads of and commands to a page: its selection, its media and snapshot
//! areas. Every answer comes from the page, so it is bounded and cleaned here before use.

use engine::page_scripts::{MEDIA_SEEK_MAX, MediaCommand};
use engine::view_policy::SnapshotArea;
use serde_json::Value;

use crate::text;

pub const SELECTION_MAX_BYTES: usize = 64 * 1024;

/// The page's selected text without surrounding white space, at most `SELECTION_MAX_BYTES`;
/// `None` when nothing is selected.
pub fn selection(raw: &str) -> Option<String> {
    let text = raw.trim();
    let mut end = text.len().min(SELECTION_MAX_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let text = text[..end].trim_end();
    (!text.is_empty()).then(|| text.to_owned())
}

/// What a component hands on from `SELECTED_TEXT`'s answer: `selection`, empty for none.
pub fn selection_answer(raw: &str) -> String {
    selection(raw).unwrap_or_default()
}

/// What a component hands on from `media_state_script`'s answer: `MediaState::parse` written back
/// as JSON, empty for none or a malformed answer.
pub fn media_answer(raw: &str) -> String {
    MediaState::parse(raw)
        .map(|m| serde_json::json!({ "playing": m.playing, "title": m.title, "duration": m.duration, "position": m.position, "pip": m.pip }).to_string())
        .unwrap_or_default()
}

/// The page's first audible media element, as `media_state_script` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaState {
    pub playing: bool,
    pub title: String,
    pub duration: Option<f64>,
    pub position: Option<f64>,
    /// Shown in Picture in Picture.
    pub pip: bool,
}

impl MediaState {
    /// `None` for an empty or malformed answer; the title is cleaned and cut for display, and
    /// times are finite, not negative and the position no later than the duration.
    pub fn parse(raw: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(raw).ok()?;
        let time = |key: &str| value[key].as_f64().filter(|t| t.is_finite() && *t >= 0.);
        let duration = time("duration");
        let position = time("position").map(|p| duration.map_or(p, |d| p.min(d)));
        let pip = value["pip"].as_bool().unwrap_or(false);
        Some(MediaState {
            playing: value["playing"].as_bool()?,
            title: text::bounded_title(value["title"].as_str().unwrap_or_default()),
            duration,
            position,
            pip,
        })
    }
}

/// A shell's media command `{action: "play" | "pause" | "seekBy", seconds}`; a seek moves at
/// most `MEDIA_SEEK_MAX` seconds either way and at least some.
pub fn media_command(args: &Value) -> Option<MediaCommand> {
    match args["action"].as_str()? {
        "play" => Some(MediaCommand::Play),
        "pause" => Some(MediaCommand::Pause),
        "seekBy" => {
            let by = args["seconds"].as_f64().filter(|s| s.is_finite() && *s != 0.)?;
            Some(MediaCommand::SeekBy(by.clamp(-MEDIA_SEEK_MAX, MEDIA_SEEK_MAX)))
        }
        _ => None,
    }
}

/// A shell's snapshot request: `{fullPage: true}` or `{x, y, width, height}` in CSS pixels of the
/// visible page, for an engine drawing `scale` device pixels per CSS pixel.
pub fn snapshot_area(args: &Value, scale: f64) -> Option<SnapshotArea> {
    if args["fullPage"].as_bool() == Some(true) {
        return Some(SnapshotArea::FullPage);
    }
    let number = |key: &str| args[key].as_f64();
    SnapshotArea::rect(number("x")?, number("y")?, number("width")?, number("height")?, scale)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_selection_is_trimmed_capped_on_a_char_boundary_and_never_blank() {
        assert_eq!(selection("  the reef \n"), Some("the reef".into()));
        assert_eq!(selection(" \n\t "), None);
        assert_eq!(selection(""), None);
        let long = "é".repeat(SELECTION_MAX_BYTES);
        let cut = selection(&long).unwrap();
        assert!(cut.len() <= SELECTION_MAX_BYTES && cut.chars().all(|c| c == 'é'));
        assert_eq!(cut.len(), SELECTION_MAX_BYTES);
    }

    #[test]
    fn media_state_is_cleaned_and_bounded() {
        let state = MediaState::parse(r#"{"playing":true,"title":" Song\u202e\n  one ","duration":200.5,"position":12}"#).unwrap();
        assert_eq!(state, MediaState { playing: true, title: "Song one".into(), duration: Some(200.5), position: Some(12.), pip: false });
        assert!(MediaState::parse(r#"{"playing":true,"pip":true}"#).unwrap().pip);
        let live = MediaState::parse(r#"{"playing":false,"title":"","duration":null,"position":-3}"#).unwrap();
        assert_eq!((live.duration, live.position), (None, None), "a live stream has no duration");
        assert_eq!(MediaState::parse(r#"{"playing":true,"duration":10,"position":99}"#).unwrap().position, Some(10.));
        assert_eq!(MediaState::parse(&format!(r#"{{"playing":true,"title":"{}"}}"#, "x".repeat(5000))).unwrap().title.chars().count(), text::MAX_TITLE_CHARS);
        assert_eq!(MediaState::parse(""), None, "no audible media, or Lockdown");
        assert_eq!(MediaState::parse(r#"{"playing":"yes"}"#), None);
    }

    #[test]
    fn components_hand_on_only_checked_answers() {
        assert_eq!(selection_answer("  the reef \n"), "the reef");
        assert_eq!(selection_answer(&"a".repeat(SELECTION_MAX_BYTES * 4)).len(), SELECTION_MAX_BYTES);
        let forged = format!(r#"{{"playing":true,"title":"{}\u202e","duration":5,"position":99,"extra":"{}"}}"#, "x".repeat(5000), "y".repeat(5000));
        let answer = media_answer(&forged);
        assert!(!answer.contains("extra") && !answer.contains('\u{202e}'), "{answer}");
        assert_eq!(MediaState::parse(&answer), MediaState::parse(&forged), "the answer parses as what it came from");
        assert_eq!(MediaState::parse(&answer).unwrap().position, Some(5.));
        assert_eq!(media_answer("not json"), "");
        let bare = MediaState { playing: true, title: String::new(), duration: None, position: None, pip: false };
        assert_eq!(MediaState::parse(&media_answer(r#"{"playing":true}"#)), Some(bare));
    }

    #[test]
    fn media_commands_parse_and_seeks_are_bounded() {
        assert_eq!(media_command(&json!({ "action": "play" })), Some(MediaCommand::Play));
        assert_eq!(media_command(&json!({ "action": "pause" })), Some(MediaCommand::Pause));
        assert_eq!(media_command(&json!({ "action": "seekBy", "seconds": -10 })), Some(MediaCommand::SeekBy(-10.)));
        assert_eq!(media_command(&json!({ "action": "seekBy", "seconds": 1e9 })), Some(MediaCommand::SeekBy(MEDIA_SEEK_MAX)));
        assert_eq!(media_command(&json!({ "action": "seekBy", "seconds": 0 })), None);
        assert_eq!(media_command(&json!({ "action": "seekBy" })), None);
        assert_eq!(media_command(&json!({ "action": "stop" })), None);
        assert_eq!(media_command(&json!(null)), None);
    }

    #[test]
    fn snapshot_areas_are_a_full_page_or_a_sane_rectangle() {
        assert_eq!(snapshot_area(&json!({ "fullPage": true }), 1.), Some(SnapshotArea::FullPage));
        assert_eq!(
            snapshot_area(&json!({ "x": 0, "y": 10.5, "width": 300, "height": 200 }), 1.),
            Some(SnapshotArea::Rect { x: 0., y: 10.5, width: 300., height: 200. })
        );
        assert_eq!(
            snapshot_area(&json!({ "x": 0, "y": 0, "width": 1e9, "height": 20 }), 1.),
            Some(SnapshotArea::Rect { x: 0., y: 0., width: engine::view_policy::SNAPSHOT_MAX_SIDE, height: 20. })
        );
        assert_eq!(snapshot_area(&json!({ "x": -1, "y": 0, "width": 10, "height": 10 }), 1.), None);
        assert_eq!(snapshot_area(&json!({ "x": 0, "y": 0, "width": 0, "height": 10 }), 1.), None);
        assert_eq!(snapshot_area(&json!({ "x": 0, "y": 0, "width": 10 }), 1.), None);
        assert_eq!(snapshot_area(&json!({ "fullPage": false }), 1.), None);
    }
}
