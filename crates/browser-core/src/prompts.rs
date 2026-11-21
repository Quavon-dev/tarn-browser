//! A web view's `prompt` event and what the shell may remember of its answer.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use shield::ShieldLevel;

/// "Visit Anyway", Keep on a flagged download, a notice's offer, "Open" in another app and a
/// permission prompt's Allow ignore clicks this long after any of them appears or moves, so a page
/// cannot time a click to land on them.
pub const BYPASS_DELAY: Duration = Duration::from_millis(700);

pub fn stable(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= BYPASS_DELAY
}

/// Restarts a guard's wait from `rest`, when what moved the guarded controls comes to rest. A wait
/// that already runs past it is kept, so a change during a slide does not end the slide's wait.
pub fn restart(since: &mut Instant, rest: Instant) {
    *since = (*since).max(rest);
}

/// Permission kinds a profile can remember per origin.
pub const PERMISSIONS: [&str; 3] = ["camera", "microphone", "location"];

pub fn meaning(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "camera" => "The site sees your camera while this tab uses it.",
        "microphone" => "The site hears your microphone while this tab uses it.",
        "camera and microphone" | "camera+microphone" => "The site sees and hears you while this tab uses them.",
        "location" | "geolocation" => "The site sees where you are while this tab is open.",
        "notifications" => "The site can show notifications while Tarn runs.",
        "credentials" => "Your name and password go to this site only.",
        "extension" => "The extension can read and change pages on this site.",
        "malicious download" => "The file comes from a site on the malware list.",
        "external" => "Tarn hands the link to the app that opens it.",
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub id: u64,
    pub kind: String,
    pub origin: String,
    pub message: Option<String>,
    pub default: Option<String>,
    pub realm: Option<String>,
    pub insecure: bool,
    pub top_origin: Option<String>,
}

impl Prompt {
    /// What the site asks, after its origin in bold; a sign-in names its realm, which the server
    /// sets, bounded.
    pub fn ask(&self) -> String {
        match self.kind.as_str() {
            "alert" | "confirm" | "text" => "says".to_owned(),
            "credentials" => match self.realm.as_deref().map(crate::text::bounded_title).filter(|r| !r.is_empty()) {
                Some(realm) => format!("asks you to sign in ({realm})"),
                None => "asks you to sign in".to_owned(),
            },
            "camera+microphone" | "camera and microphone" => "wants to use your camera and microphone".to_owned(),
            kind => format!("wants to use your {kind}"),
        }
    }

    pub fn from_event(data: &Value) -> Option<Prompt> {
        let text = |field: &str| data.get(field).and_then(Value::as_str).map(str::to_owned);
        Some(Prompt {
            id: data.get("id").and_then(Value::as_u64)?,
            kind: text("kind").unwrap_or_default(),
            origin: text("origin").unwrap_or_default(),
            message: text("message"),
            default: text("default"),
            realm: text("realm"),
            insecure: data.get("insecure").and_then(Value::as_bool).unwrap_or(false),
            top_origin: text("topOrigin"),
        })
    }

    pub fn is_permission(&self) -> bool {
        PERMISSIONS.contains(&self.kind.as_str())
    }

    /// Only the top page's own requests are remembered, never an embedded frame's.
    pub fn rememberable(&self) -> bool {
        self.is_permission() && !self.origin.is_empty() && self.top_origin.as_deref().is_none_or(|top| top == self.origin)
    }

    pub fn remembered_at(&self, level: ShieldLevel) -> bool {
        self.rememberable() && level == ShieldLevel::Standard
    }

    pub fn silence(&self) -> Value {
        json!({ "id": self.id, "accept": false, "silence": true })
    }

    /// The `answer` command's arguments.
    pub fn answer(&self, accept: bool, text: Option<String>, user: Option<String>, password: Option<String>) -> Value {
        let mut args = json!({ "id": self.id, "accept": accept });
        for (field, value) in [("text", text), ("user", user), ("password", password)] {
            if let Some(value) = value {
                args[field] = json!(value);
            }
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_clicks_wait_for_a_stable_layout() {
        let since = Instant::now();
        assert!(!stable(since, since));
        assert!(!stable(since, since + BYPASS_DELAY - Duration::from_millis(1)));
        assert!(stable(since, since + BYPASS_DELAY));
        assert!(!stable(since + BYPASS_DELAY, since), "a change after `now` is not stable");
    }

    fn prompt(data: Value) -> Prompt {
        Prompt::from_event(&data).unwrap()
    }

    #[test]
    fn a_restart_never_ends_a_longer_wait() {
        let now = Instant::now();
        let mut since = now;
        restart(&mut since, now + Duration::from_millis(280));
        restart(&mut since, now + Duration::from_millis(10));
        assert_eq!(since, now + Duration::from_millis(280), "a change mid-slide keeps the slide's wait");
        restart(&mut since, now + Duration::from_secs(1));
        assert_eq!(since, now + Duration::from_secs(1));
    }

    #[test]
    fn prompts_need_an_id() {
        assert!(Prompt::from_event(&json!({ "kind": "alert" })).is_none());
        let alert = prompt(json!({ "id": 3, "kind": "alert", "origin": "https://a.test", "message": "hi" }));
        assert_eq!((alert.id, alert.message.as_deref(), alert.realm), (3, Some("hi"), None));
    }

    #[test]
    fn only_the_top_pages_permissions_are_remembered() {
        assert!(prompt(json!({ "id": 1, "kind": "camera", "origin": "https://a.test" })).rememberable());
        assert!(prompt(json!({ "id": 1, "kind": "camera", "origin": "https://a.test", "topOrigin": "https://a.test" })).rememberable());
        assert!(!prompt(json!({ "id": 1, "kind": "camera", "origin": "https://ads.test", "topOrigin": "https://a.test" })).rememberable());
        assert!(!prompt(json!({ "id": 1, "kind": "location", "origin": "" })).rememberable());
        assert!(!prompt(json!({ "id": 1, "kind": "confirm", "origin": "https://a.test" })).rememberable());
    }

    #[test]
    fn answers_are_remembered_only_at_standard() {
        let top = prompt(json!({ "id": 1, "kind": "camera", "origin": "https://a.test", "topOrigin": "https://a.test" }));
        assert!(top.remembered_at(ShieldLevel::Standard));
        assert!(!top.remembered_at(ShieldLevel::Strict));
        assert!(!top.remembered_at(ShieldLevel::Lockdown));
        let unknown_frame = prompt(json!({ "id": 1, "kind": "camera", "origin": "https://a.test", "topOrigin": "" }));
        assert!(!unknown_frame.remembered_at(ShieldLevel::Standard));
    }

    #[test]
    fn silencing_declines_and_names_the_page() {
        assert_eq!(prompt(json!({ "id": 4, "kind": "alert" })).silence(), json!({ "id": 4, "accept": false, "silence": true }));
    }

    #[test]
    fn answers_carry_only_the_fields_given() {
        let p = prompt(json!({ "id": 7, "kind": "credentials", "origin": "https://a.test" }));
        assert_eq!(p.answer(true, None, Some("u".into()), Some("p".into())), json!({ "id": 7, "accept": true, "user": "u", "password": "p" }));
        assert_eq!(p.answer(false, None, None, None), json!({ "id": 7, "accept": false }));
    }

    #[test]
    fn permissions_say_what_allowing_means() {
        assert!(meaning("location").is_some_and(|m| m.contains("where you are")));
        assert!(PERMISSIONS.iter().all(|kind| meaning(kind).is_some()));
        assert_eq!(meaning("alert"), None);
        assert_eq!(meaning("download"), None, "only a listed download names the malware list");
    }

    #[test]
    fn a_prompt_says_what_the_site_asks() {
        let ask =
            |kind: &str, realm: Option<&str>| Prompt::from_event(&json!({ "id": 1, "kind": kind, "origin": "https://a.test", "realm": realm })).unwrap().ask();
        assert_eq!(ask("confirm", None), "says");
        assert_eq!(ask("camera+microphone", None), "wants to use your camera and microphone");
        assert_eq!(ask("credentials", Some("")), "asks you to sign in");
        assert_eq!(ask("credentials", Some("Admin")), "asks you to sign in (Admin)");
        assert!(ask("credentials", Some(&"r".repeat(5000))).len() < 600);
    }
}
