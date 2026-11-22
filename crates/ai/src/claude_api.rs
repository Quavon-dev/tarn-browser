use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::provider::{self, AiError, Provider, Request};
use crate::transport::Transport;

const URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const MAX_TOKENS: u32 = 16_000;

/// Has no `Display`, and its `Debug` never shows the value.
pub struct ApiToken(String);

impl ApiToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

#[derive(Debug)]
pub struct ClaudeApi {
    token: ApiToken,
    model: String,
    url: String,
    transport: Arc<dyn Transport>,
}

impl ClaudeApi {
    pub fn new(token: ApiToken, model: Option<String>, transport: Arc<dyn Transport>) -> Self {
        Self { token, model: model.unwrap_or_else(|| DEFAULT_MODEL.into()), url: URL.into(), transport }
    }
}

impl Provider for ClaudeApi {
    fn stream(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "system": req.system,
            "messages": [{ "role": "user", "content": req.user_message() }],
        });
        let headers = [("x-api-key", self.token.expose()), ("anthropic-version", API_VERSION)];
        let reader = provider::post_json(&*self.transport, &self.url, &headers, &body, cancel)?;
        provider::for_each_line(reader, cancel, |line| {
            let Some(data) = line.strip_prefix("data:") else {
                return Ok(false);
            };
            let v: serde_json::Value = serde_json::from_str(data.trim()).map_err(|e| AiError::Protocol(e.to_string()))?;
            match v["type"].as_str() {
                Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                    if let Some(text) = v["delta"]["text"].as_str() {
                        on_text(text);
                    }
                    Ok(false)
                }
                Some("message_delta") if v["delta"]["stop_reason"] == "refusal" => Err(AiError::Refused),
                Some("message_stop") => Ok(true),
                Some("error") => Err(AiError::Protocol(v["error"]["message"].as_str().unwrap_or("stream error").chars().take(300).collect())),
                _ => Ok(false),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_context::PageContext;
    use crate::provider::test_server::serve;
    use crate::transport::UreqTransport;

    const SECRET: &str = "sk-ant-TESTSECRET-0123456789";

    fn api(url: String) -> ClaudeApi {
        ClaudeApi { url, ..ClaudeApi::new(ApiToken::new(SECRET.into()), None, Arc::new(UreqTransport)) }
    }

    fn req() -> Request {
        Request { system: "sys".into(), page: Some(PageContext::new("https://a.test/x?q=1", "T", "body")), question: "q".into() }
    }

    #[test]
    fn streams_sse_text() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi \"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"there\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let (base, rx) = serve("200 OK", body);
        let mut out = String::new();
        api(format!("{base}/v1/messages")).stream(&req(), &AtomicBool::new(false), &mut |t| out.push_str(t)).unwrap();
        assert_eq!(out, "Hi there");
        let raw = rx.recv().unwrap().to_ascii_lowercase();
        assert!(raw.contains("anthropic-version: 2023-06-01"));
        assert!(raw.contains(&format!("x-api-key: {}", SECRET.to_ascii_lowercase())));
        assert!(raw.contains("\"model\":\"claude-opus-5-5\""));
        assert!(!raw.contains("\"tools\""));
        assert!(!raw.contains("q=1"));
    }

    #[test]
    fn reports_refusal_and_errors() {
        let (base, _) = serve("200 OK", "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n\n");
        let e = api(base).stream(&req(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(e, AiError::Refused));

        let (base, _) = serve("401 Unauthorized", "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}");
        let e = api(base).stream(&req(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(e, AiError::Status(401, ref m) if m == "invalid x-api-key"));
    }

    #[test]
    fn does_not_follow_redirects() {
        let (base, _) = serve("307 Temporary Redirect\r\nLocation: https://evil.test/", "");
        let e = api(base).stream(&req(), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(e, AiError::Status(307, _)));
    }

    #[test]
    fn never_sends_to_a_host_outside_the_allowlist() {
        #[derive(Debug)]
        struct Unreachable;
        impl Transport for Unreachable {
            fn post<'a>(&self, _: &str, _: &[(&str, &str)], _: &[u8], _: u64, _: &'a AtomicBool) -> Result<crate::transport::Response<'a>, AiError> {
                panic!("the token reached the transport");
            }
        }
        let api = ClaudeApi { url: "https://evil.test/v1/messages".into(), ..ClaudeApi::new(ApiToken::new(SECRET.into()), None, Arc::new(Unreachable)) };
        assert!(matches!(api.stream(&req(), &AtomicBool::new(false), &mut |_| {}), Err(AiError::Network(_))));
    }

    #[test]
    fn only_anthropic_host() {
        assert_eq!(ClaudeApi::new(ApiToken::new(String::new()), None, Arc::new(UreqTransport)).url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn token_never_in_debug_output() {
        let token = || ApiToken::new(SECRET.into());
        let outputs = [
            format!("{:?}", token()),
            format!("{:#?}", ClaudeApi::new(token(), Some("m".into()), Arc::new(UreqTransport))),
            format!("{:?}", req()),
            format!("{:?}", crate::provider::AiError::Status(401, "x".into())),
            format!("{:?}", crate::ollama::Ollama::new("m", Arc::new(UreqTransport))),
            format!("{:?}", crate::claude_cli::ClaudeCli::new("/usr/bin/false".into(), None)),
        ];
        for out in outputs {
            assert!(!out.contains(SECRET) && !out.contains("TESTSECRET"), "{out}");
        }
    }
}
