//! Models that answer on this device: the platform's on-device model, or Ollama on loopback with a
//! model that runs on this machine. A local model that cannot answer says why; nothing here ever
//! hands a request to a cloud provider.

use std::sync::atomic::AtomicBool;

use crate::provider::{AiError, Request};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// This device, system or model cannot answer on the device.
    NotSupported(String),
    /// The model can run once the user does what the reason says.
    NeedsSetup(String),
}

/// Proof that `stream` checked the model's availability; only `stream` can make one, so a local
/// model runs through it alone.
pub struct Checked(pub(crate) ());

/// Both methods may block (Ollama asks its server), so call `stream` off the UI thread.
pub trait LocalModel: Send + Sync {
    fn availability(&self) -> Availability;
    fn respond(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str), checked: Checked) -> Result<(), AiError>;
}

/// Streams from `model` when it can answer, else fails with its reason; never asks another model.
pub fn stream(model: &dyn LocalModel, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
    match model.availability() {
        Availability::Available => model.respond(req, cancel, on_text, Checked(())),
        Availability::NotSupported(why) | Availability::NeedsSetup(why) => Err(AiError::Unavailable(why)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Local,
    Cloud,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Local,
    Cloud,
    Unavailable(String),
}

/// Who answers: the chosen model, except that a selection command or a summary (`on_device_task`)
/// stays on the device whenever the local model is available. A local choice never reaches the cloud.
pub fn route(choice: Choice, local: &Availability, on_device_task: bool) -> Route {
    match (choice, local) {
        (Choice::Local, Availability::Available) => Route::Local,
        (Choice::Local, Availability::NotSupported(why) | Availability::NeedsSetup(why)) => Route::Unavailable(why.clone()),
        (Choice::Cloud, Availability::Available) if on_device_task => Route::Local,
        (Choice::Cloud, _) => Route::Cloud,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::Ollama;
    use crate::transport::{Response, Transport};

    /// Answers `/api/show` with `show` (none: Ollama is not running) and `/api/chat` with one word,
    /// and records every address it was asked for.
    #[derive(Debug)]
    struct Fake {
        show: Option<(u16, &'static str)>,
        asked: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn post<'a>(&self, url: &str, _: &[(&str, &str)], _: &[u8], _: u64, _: &'a AtomicBool) -> Result<Response<'a>, AiError> {
            self.asked.lock().unwrap().push(url.rsplit('/').next().unwrap_or_default().to_owned());
            let (status, body): (u16, &'static str) = match url.ends_with("/api/chat") {
                true => (200, "{\"message\":{\"content\":\"hi\"},\"done\":true}\n"),
                false => self.show.ok_or_else(|| AiError::Network("connection refused".into()))?,
            };
            Ok(Response { status, body: Box::new(body.as_bytes()) })
        }
    }

    fn run(model: &str, show: Option<(u16, &'static str)>) -> (Availability, Result<String, AiError>, Vec<String>) {
        let fake = Arc::new(Fake { show, asked: Mutex::default() });
        let ollama = Ollama::new(model, fake.clone());
        let availability = ollama.availability();
        let mut out = String::new();
        let result = stream(&ollama, &Request { system: "s".into(), page: None, question: "q".into() }, &AtomicBool::new(false), &mut |t| out.push_str(t));
        let asked = fake.asked.lock().unwrap().clone();
        (availability, result.map(|()| out), asked)
    }

    #[test]
    fn ollama_answers_only_when_it_answers_and_the_model_is_on_this_device() {
        let (availability, out, asked) = run("llama3.2", Some((200, "{\"details\":{\"family\":\"llama\"}}")));
        assert_eq!(availability, Availability::Available);
        assert_eq!(out.unwrap(), "hi");
        assert_eq!(asked, ["show", "show", "chat"]);
    }

    #[test]
    fn an_ollama_that_does_not_answer_needs_setup_and_gets_no_question() {
        for show in [None, Some((500, "{}")), Some((200, "<html>")), Some((404, "{\"error\":\"model not found\"}"))] {
            let (availability, out, asked) = run("llama3.2", show);
            assert!(matches!(availability, Availability::NeedsSetup(_)), "{show:?}");
            assert!(matches!(out, Err(AiError::Unavailable(_))), "{show:?}");
            assert!(!asked.contains(&"chat".to_owned()), "{show:?}");
        }
    }

    #[test]
    fn cloud_models_are_never_local() {
        let (availability, out, asked) = run("gpt-oss:120b-cloud", Some((200, "{}")));
        assert!(matches!(availability, Availability::NotSupported(_)));
        assert!(matches!(out, Err(AiError::Unavailable(_))));
        assert!(asked.is_empty(), "a cloud name is refused before asking Ollama");
        for show in ["{\"remote_host\":\"https://ollama.com:443\"}", "{\"remote_model\":\"gpt-oss:120b\"}"] {
            let (availability, out, asked) = run("my-alias", Some((200, show)));
            assert!(matches!(availability, Availability::NotSupported(_)), "{show}");
            assert!(matches!(out, Err(AiError::Unavailable(_))), "{show}");
            assert!(!asked.contains(&"chat".to_owned()), "{show}");
        }
    }

    #[test]
    fn a_local_choice_never_routes_to_the_cloud() {
        let off = Availability::NeedsSetup("Turn it on".into());
        let gone = Availability::NotSupported("No".into());
        for task in [false, true] {
            assert_eq!(route(Choice::Local, &Availability::Available, task), Route::Local);
            assert_eq!(route(Choice::Local, &off, task), Route::Unavailable("Turn it on".into()));
            assert_eq!(route(Choice::Local, &gone, task), Route::Unavailable("No".into()));
            assert_eq!(route(Choice::Cloud, &off, task), Route::Cloud);
        }
        assert_eq!(route(Choice::Cloud, &Availability::Available, true), Route::Local, "selections and summaries stay on the device");
        assert_eq!(route(Choice::Cloud, &Availability::Available, false), Route::Cloud);
    }
}
