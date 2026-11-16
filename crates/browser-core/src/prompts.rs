//! A web view's `prompt` event and what the shell may remember of its answer.

use serde_json::{Value, json};

/// Permission kinds a profile can remember per origin.
pub const PERMISSIONS: [&str; 3] = ["camera", "microphone", "location"];

/// One line under the title saying what allowing does, for the kinds where that is not obvious.
pub fn meaning(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "camera" => "The site sees your camera while this tab uses it.",
        "microphone" => "The site hears your microphone while this tab uses it.",
        "camera and microphone" | "camera+microphone" => "The site sees and hears you while this tab uses them.",
        "location" | "geolocation" => "The site sees where you are while this tab is open.",
        "notifications" => "The site can show notifications while Tarn runs.",
        "credentials" => "Your name and password go to this site only.",
        "extension" => "The extension can read and change pages on this site.",
        "download" | "malicious download" => "The file comes from a site on the malware list.",
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

    fn prompt(data: Value) -> Prompt {
        Prompt::from_event(&data).unwrap()
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
    }
}
