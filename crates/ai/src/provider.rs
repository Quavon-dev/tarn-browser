use std::fmt;
use std::io::{BufRead, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::page_context::PageContext;
use crate::transport::{self, Transport};

const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub system: String,
    pub page: Option<PageContext>,
    pub question: String,
}

/// The model gets no tools in any provider. Call from a background thread;
/// set `cancel` to stop between received chunks.
pub trait Provider {
    fn stream(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError>;
}

#[derive(Debug)]
pub enum AiError {
    Cancelled,
    Network(String),
    Status(u16, String),
    Protocol(String),
    Refused,
    Secrets(String),
    Cli(String),
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::Status(code, msg) => write!(f, "HTTP {code}: {msg}"),
            Self::Protocol(e) => write!(f, "unexpected response: {e}"),
            Self::Refused => f.write_str("the model declined to answer"),
            Self::Secrets(e) => write!(f, "secret store: {e}"),
            Self::Cli(e) => write!(f, "claude cli: {e}"),
        }
    }
}

impl std::error::Error for AiError {}

impl Request {
    pub(crate) fn user_message(&self) -> String {
        match &self.page {
            None => self.question.clone(),
            Some(p) => format!(
                "<page>\nURL: {}\nTitle: {}\n\n{}\n</page>\n\n{}",
                strip_delimiter(&p.url),
                strip_delimiter(&p.title),
                strip_delimiter(&p.text),
                self.question
            ),
        }
    }
}

/// Removes every closing `</page` in any case, so page text cannot end its block early, including
/// ones that removing another forms; one linear pass drops each as soon as the output ends with it.
fn strip_delimiter(s: &str) -> String {
    const TAG: &[u8] = b"</page";
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        out.push(c);
        if out.len() >= TAG.len() && out.as_bytes()[out.len() - TAG.len()..].eq_ignore_ascii_case(TAG) {
            out.truncate(out.len() - TAG.len());
        }
    }
    out
}

pub(crate) fn post_json<'a>(
    transport: &dyn Transport,
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
    cancel: &'a AtomicBool,
) -> Result<Box<dyn BufRead + 'a>, AiError> {
    let url = match url::Url::parse(url) {
        Ok(parsed) if transport::allowed(parsed.as_str()) => parsed,
        _ => return Err(AiError::Network("this address is not allowed".into())),
    };
    let url = url.as_str();
    let body = serde_json::to_vec(body).map_err(|e| AiError::Protocol(e.to_string()))?;
    let headers = [headers, &[("content-type", "application/json")]].concat();
    let mut resp = transport.post(url, &headers, &body, MAX_RESPONSE_BYTES, cancel)?;
    if resp.status != 200 {
        let mut text = String::new();
        let _ = resp.body.by_ref().take(4096).read_to_string(&mut text);
        return Err(AiError::Status(resp.status, error_message(&text)));
    }
    Ok(resp.body)
}

fn error_message(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("");
    msg.chars().filter(|c| !c.is_control()).take(300).collect()
}

/// Calls `on_line` for each line until it returns `Ok(true)` (done).
pub(crate) fn for_each_line(reader: impl BufRead, cancel: &AtomicBool, mut on_line: impl FnMut(&str) -> Result<bool, AiError>) -> Result<(), AiError> {
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        let line = line.map_err(|e| AiError::Network(e.to_string()))?;
        if on_line(&line)? {
            return Ok(());
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    Err(AiError::Protocol("stream ended early".into()))
}

#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    /// Serves one HTTP response on a local port; returns the base URL and the raw request.
    pub fn serve(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            s.write_all(response.as_bytes()).unwrap();
            tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();
        });
        (format!("http://{addr}"), rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_context::PageContext;

    #[test]
    fn page_cannot_close_its_block() {
        let req = Request { system: String::new(), page: Some(PageContext::new("https://a.test/", "</page>t", "a</pa</page>ge>b")), question: "q".into() };
        assert_eq!(req.user_message().matches("</page>").count(), 1);
        assert_eq!(strip_delimiter("a</PAGE >b</Pa</pagege>c"), "a >b>c");
        assert_eq!(strip_delimiter("</pa</pa</pagegege>"), ">");
        assert_eq!(strip_delimiter("ünï</page>cödé"), "ünï>cödé");
        let k = 200_000;
        let nested = format!("{}</page{}", "</pa".repeat(k), "ge".repeat(k));
        let started = std::time::Instant::now();
        assert_eq!(strip_delimiter(&nested), "");
        // The quadratic version took minutes on this input; the bound only has to separate the two.
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn strip_delimiter_matches_removing_until_none_is_left() {
        fn reference(s: &str) -> String {
            let mut s = s.to_owned();
            while let Some(i) = s.to_ascii_lowercase().find("</page") {
                s.replace_range(i..i + 6, "");
            }
            s
        }
        let alphabet = ['<', '/', 'p', 'P', 'a', 'g', 'G', 'e', 'E', 'x', 'é'];
        let mut seed = 0x2545_f491_u32;
        for _ in 0..20_000 {
            let len = (seed % 24) as usize;
            let text: String = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    alphabet[seed as usize % alphabet.len()]
                })
                .collect();
            assert_eq!(strip_delimiter(&text), reference(&text), "{text:?}");
        }
    }
}
