use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::local::{Availability, Checked, LocalModel};
use crate::provider::{self, AiError, Request};
use crate::transport::Transport;

const BASE: &str = "http://127.0.0.1:11434";
const MAX_SHOW_BYTES: u64 = 1024 * 1024;
pub const NOT_RUNNING: &str = "Ollama is not running on this device. Start it, then try again.";
pub const CLOUD: &str = "This Ollama model runs in Ollama's cloud. Pick a model that runs on this device.";

/// Ollama on loopback, run only through `local::stream`: it answers when Ollama itself answers and
/// the model runs on this machine, never one of Ollama's cloud models.
#[derive(Debug, Clone)]
pub struct Ollama {
    model: String,
    base: String,
    transport: Arc<dyn Transport>,
}

/// Ollama names its cloud models with a `-cloud` or `:cloud` tag.
pub fn is_cloud_name(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.ends_with("-cloud") || model.ends_with(":cloud")
}

impl Ollama {
    pub fn new(model: impl Into<String>, transport: Arc<dyn Transport>) -> Self {
        Self { model: model.into(), base: BASE.into(), transport }
    }

    #[cfg(test)]
    pub(crate) fn at(base: &str, model: &str, transport: Arc<dyn Transport>) -> Self {
        Self { base: base.into(), ..Self::new(model, transport) }
    }
}

impl LocalModel for Ollama {
    /// Asks Ollama's `/api/show` about the model: a model with a remote host runs in the cloud.
    fn availability(&self) -> Availability {
        if is_cloud_name(&self.model) {
            return Availability::NotSupported(CLOUD.into());
        }
        let cancel = AtomicBool::new(false);
        let url = format!("{}/api/show", self.base);
        let mut body = match provider::post_json(&*self.transport, &url, &[], &json!({ "model": self.model }), &cancel) {
            Ok(body) => body,
            Err(AiError::Status(404, _)) => return Availability::NeedsSetup(format!("Download {} in Ollama first.", self.model)),
            Err(_) => return Availability::NeedsSetup(NOT_RUNNING.into()),
        };
        let mut text = String::new();
        if body.by_ref().take(MAX_SHOW_BYTES).read_to_string(&mut text).is_err() {
            return Availability::NeedsSetup(NOT_RUNNING.into());
        }
        let Ok(show) = serde_json::from_str::<serde_json::Value>(&text) else { return Availability::NeedsSetup(NOT_RUNNING.into()) };
        let remote = |key: &str| show[key].as_str().is_some_and(|v| !v.is_empty());
        if remote("remote_host") || remote("remote_model") { Availability::NotSupported(CLOUD.into()) } else { Availability::Available }
    }

    fn respond(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str), _: Checked) -> Result<(), AiError> {
        if is_cloud_name(&self.model) {
            return Err(AiError::Unavailable(CLOUD.into()));
        }
        let body = json!({
            "model": self.model,
            "stream": true,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user_message() },
            ],
        });
        let reader = provider::post_json(&*self.transport, &format!("{}/api/chat", self.base), &[], &body, cancel)?;
        provider::for_each_line(reader, cancel, |line| {
            if line.trim().is_empty() {
                return Ok(false);
            }
            let v: serde_json::Value = serde_json::from_str(line).map_err(|e| AiError::Protocol(e.to_string()))?;
            if let Some(err) = v["error"].as_str() {
                return Err(AiError::Protocol(err.chars().take(300).collect()));
            }
            if let Some(text) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) {
                on_text(text);
            }
            Ok(v["done"].as_bool() == Some(true))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::test_server::serve;
    use crate::transport::UreqTransport;

    fn req() -> Request {
        Request { system: "sys".into(), page: None, question: "q".into() }
    }

    fn respond(o: &Ollama, cancel: bool, out: &mut String) -> Result<(), AiError> {
        o.respond(&req(), &AtomicBool::new(cancel), &mut |t| out.push_str(t), Checked(()))
    }

    #[test]
    fn streams_ndjson() {
        let body = "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n{\"message\":{\"content\":\"lo\"},\"done\":false}\n{\"done\":true}\n";
        let (base, rx) = serve("200 OK", body);
        let mut out = String::new();
        respond(&Ollama::at(&base, "m", Arc::new(UreqTransport)), false, &mut out).unwrap();
        assert_eq!(out, "Hello");
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("POST /api/chat "));
        assert!(!raw.contains("\"tools\""));
    }

    #[test]
    fn does_not_follow_redirects() {
        let (base, _rx) = serve("302 Found\r\nLocation: http://example.com/", "");
        let err = respond(&Ollama::at(&base, "m", Arc::new(UreqTransport)), false, &mut String::new()).unwrap_err();
        assert!(matches!(err, AiError::Status(302, _)));
    }

    #[test]
    fn cancel_stops_stream() {
        let (base, _rx) = serve("200 OK", "{\"message\":{\"content\":\"x\"},\"done\":false}\n");
        let err = respond(&Ollama::at(&base, "m", Arc::new(UreqTransport)), true, &mut String::new()).unwrap_err();
        assert!(matches!(err, AiError::Cancelled));
    }

    #[test]
    fn host_is_loopback() {
        assert_eq!(Ollama::new("m", Arc::new(UreqTransport)).base, "http://127.0.0.1:11434");
    }

    #[test]
    fn cloud_names_are_recognised() {
        for name in ["gpt-oss:120b-cloud", "qwen3-coder:480b-cloud", "x:cloud", " GLM-4.6:CLOUD "] {
            assert!(is_cloud_name(name), "{name}");
        }
        for name in ["llama3.2", "cloudy:7b", "mistral:7b-instruct"] {
            assert!(!is_cloud_name(name), "{name}");
        }
    }
}
