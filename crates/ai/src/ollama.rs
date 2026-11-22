use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::json;

use crate::local::{Availability, LocalModel};
use crate::provider::{self, AiError, Provider, Request};
use crate::transport::Transport;

const URL: &str = "http://127.0.0.1:11434/api/chat";
pub const NOT_RUNNING: &str = "Ollama is not running on this device. Start it, then try again.";

#[derive(Debug, Clone)]
pub struct Ollama {
    model: String,
    url: String,
    transport: Arc<dyn Transport>,
}

impl Ollama {
    pub fn new(model: impl Into<String>, transport: Arc<dyn Transport>) -> Self {
        Self { model: model.into(), url: URL.into(), transport }
    }

    #[cfg(test)]
    pub(crate) fn at_port(port: u16, model: &str, transport: Arc<dyn Transport>) -> Self {
        Self { url: format!("http://127.0.0.1:{port}/api/chat"), ..Self::new(model, transport) }
    }

    fn listening(&self) -> bool {
        let addr = url::Url::parse(&self.url).ok().and_then(|u| match u.host() {
            Some(url::Host::Ipv4(ip)) if ip.is_loopback() => Some(SocketAddr::from((ip, u.port_or_known_default()?))),
            _ => None,
        });
        addr.is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok())
    }
}

/// Ollama runs on this machine, so it counts as local once something listens on its loopback port.
impl LocalModel for Ollama {
    fn availability(&self) -> Availability {
        if self.listening() { Availability::Available } else { Availability::NeedsSetup(NOT_RUNNING.into()) }
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
        let reader = provider::post_json(&*self.transport, &self.url, &[], &body, cancel)?;
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

    #[test]
    fn streams_ndjson() {
        let body = "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n{\"message\":{\"content\":\"lo\"},\"done\":false}\n{\"done\":true}\n";
        let (base, rx) = serve("200 OK", body);
        let o = Ollama { url: format!("{base}/api/chat"), ..Ollama::new("m", Arc::new(UreqTransport)) };
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
        let o = Ollama { url: format!("{base}/api/chat"), ..Ollama::new("m", Arc::new(UreqTransport)) };
        let err = o.stream(&req(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(err, AiError::Status(302, _)));
    }

    #[test]
    fn cancel_stops_stream() {
        let (base, _rx) = serve("200 OK", "{\"message\":{\"content\":\"x\"},\"done\":false}\n");
        let o = Ollama { url: format!("{base}/api/chat"), ..Ollama::new("m", Arc::new(UreqTransport)) };
        let err = o.stream(&req(), &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert!(matches!(err, AiError::Cancelled));
    }

    #[test]
    fn host_is_loopback() {
        assert!(Ollama::new("m", Arc::new(UreqTransport)).url.starts_with("http://127.0.0.1:11434/"));
    }
}
