use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::json;

use crate::provider::{self, AiError, Provider, Request};

const URL: &str = "http://127.0.0.1:11434/api/chat";

#[derive(Debug, Clone)]
pub struct Ollama {
    model: String,
    url: String,
}

impl Ollama {
    pub fn new(model: impl Into<String>) -> Self {
        Self { model: model.into(), url: URL.into() }
    }
}

impl Provider for Ollama {
    fn stream(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        let body = json!({
            "model": self.model,
            "stream": true,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user_message() },
            ],
        });
        let agent = provider::agent(Duration::from_secs(600));
        let reader = provider::post_json(agent.post(&self.url), &body)?;
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

    fn req() -> Request {
        Request { system: "sys".into(), page: None, question: "q".into() }
    }

    #[test]
    fn streams_ndjson() {
        let body = "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n{\"message\":{\"content\":\"lo\"},\"done\":false}\n{\"done\":true}\n";
        let (base, rx) = serve("200 OK", body);
        let o = Ollama { model: "m".into(), url: format!("{base}/api/chat") };
        let mut out = String::new();
        o.stream(&req(), &AtomicBool::new(false), &mut |t| out.push_str(t)).unwrap();
        assert_eq!(out, "Hello");
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("POST /api/chat "));
        assert!(!raw.contains("\"tools\""));
    }

    #[test]
    fn does_not_follow_redirects() {
        let (base, _rx) = serve("302 Found\r\nLocation: http://example.com/", "");
        let o = Ollama { model: "m".into(), url: format!("{base}/api/chat") };
        let err = o.stream(&req(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(err, AiError::Status(302, _)));
    }

    #[test]
    fn cancel_stops_stream() {
        let (base, _rx) = serve("200 OK", "{\"message\":{\"content\":\"x\"},\"done\":false}\n");
        let o = Ollama { model: "m".into(), url: format!("{base}/api/chat") };
        let err = o.stream(&req(), &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert!(matches!(err, AiError::Cancelled));
    }

    #[test]
    fn host_is_loopback() {
        assert!(Ollama::new("m").url.starts_with("http://127.0.0.1:11434/"));
    }
}
