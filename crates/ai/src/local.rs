//! Models that answer on this device: the platform's on-device model, or Ollama on loopback. A
//! local model that cannot answer says why; nothing here ever hands a request to a cloud provider.

use std::sync::atomic::AtomicBool;

use crate::provider::{AiError, Provider, Request};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// This device or system cannot run the model.
    NotSupported(String),
    /// The model can run once the user does what the reason says.
    NeedsSetup(String),
}

pub trait LocalModel: Provider + Send + Sync {
    fn availability(&self) -> Availability;
}

/// Streams from `model` when it can answer, else fails with its reason; never asks another model.
pub fn stream(model: &dyn LocalModel, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
    match model.availability() {
        Availability::Available => model.stream(req, cancel, on_text),
        Availability::NotSupported(why) | Availability::NeedsSetup(why) => Err(AiError::Unavailable(why)),
    }
}

/// The model the user chose for the assistant.
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
    use std::net::TcpListener;
    use std::sync::Arc;

    use super::*;
    use crate::Ollama;
    use crate::transport::{Response, Transport};

    #[derive(Debug)]
    struct NoNetwork;

    impl Transport for NoNetwork {
        fn post<'a>(&self, url: &str, _: &[(&str, &str)], _: &[u8], _: u64, _: &'a AtomicBool) -> Result<Response<'a>, AiError> {
            panic!("a local model sent a request to {url}");
        }
    }

    fn req() -> Request {
        Request { system: "s".into(), page: None, question: "q".into() }
    }

    #[test]
    fn an_unreachable_ollama_needs_setup_and_sends_nothing() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let model = Ollama::at_port(port, "m", Arc::new(NoNetwork));
        assert!(matches!(model.availability(), Availability::NeedsSetup(_)));
        let err = stream(&model, &req(), &AtomicBool::new(false), &mut |_| panic!("no text")).unwrap_err();
        assert!(matches!(err, AiError::Unavailable(_)), "{err}");
    }

    #[test]
    fn a_listening_ollama_is_available() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let model = Ollama::at_port(listener.local_addr().unwrap().port(), "m", Arc::new(NoNetwork));
        assert_eq!(model.availability(), Availability::Available);
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
