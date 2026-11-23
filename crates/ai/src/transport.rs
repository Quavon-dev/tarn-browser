use std::fmt;
use std::io::{self, BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use url::{Host, Url};

use crate::provider::AiError;

pub struct Response<'a> {
    pub status: u16,
    pub body: Box<dyn BufRead + 'a>,
}

/// How a client sends a provider's request. Implementations never follow redirects, and their body
/// fails once it passes `max` bytes or `cancel` is set (`guard` does both).
pub trait Transport: Send + Sync + fmt::Debug {
    fn post<'a>(&self, url: &str, headers: &[(&str, &str)], body: &[u8], max: u64, cancel: &'a AtomicBool) -> Result<Response<'a>, AiError>;
}

/// The Anthropic API over HTTPS, or Ollama on literal loopback over HTTP.
pub fn allowed(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else { return false };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match (url.scheme(), url.host()) {
        ("https", Some(Host::Domain("api.anthropic.com"))) => url.port().is_none(),
        ("http", Some(Host::Ipv4(ip))) => ip == std::net::Ipv4Addr::LOCALHOST,
        _ => false,
    }
}

pub fn guard<'a>(body: impl Read + 'a, max: u64, cancel: &'a AtomicBool) -> Box<dyn BufRead + 'a> {
    Box::new(BufReader::new(Guarded { body, left: max, cancel }))
}

struct Guarded<'a, R> {
    body: R,
    left: u64,
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for Guarded<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelled"));
        }
        let n = self.body.read(buf)?;
        self.left = self.left.checked_sub(n as u64).ok_or_else(|| io::Error::other("response too large"))?;
        Ok(n)
    }
}

#[cfg(not(target_os = "android"))]
pub use self::ureq_transport::UreqTransport;

#[cfg(not(target_os = "android"))]
mod ureq_transport {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use ureq::tls::{RootCerts, TlsConfig};

    use super::{Response, Transport, guard};
    use crate::provider::AiError;

    /// Talks TLS itself with the platform's certificate verifier; Android's needs JNI setup, so
    /// the Android host brings its own transport.
    #[derive(Debug, Default)]
    pub struct UreqTransport;

    impl Transport for UreqTransport {
        fn post<'a>(&self, url: &str, headers: &[(&str, &str)], body: &[u8], max: u64, cancel: &'a AtomicBool) -> Result<Response<'a>, AiError> {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            let mut req = agent(Duration::from_secs(600)).post(url);
            for (name, value) in headers {
                req = req.header(*name, *value);
            }
            let resp = req.send(body).map_err(|e| AiError::Network(e.to_string()))?;
            let status = resp.status().as_u16();
            Ok(Response { status, body: guard(resp.into_body().into_reader(), max, cancel) })
        }
    }

    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder()
            .max_redirects(0)
            .proxy(None)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
                    .build(),
            )
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(timeout))
            .build()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_anthropic_over_https_and_loopback_over_http() {
        assert!(allowed("https://api.anthropic.com/v1/messages"));
        assert!(allowed("http://127.0.0.1:11434/api/chat"));
        for url in [
            "http://api.anthropic.com/v1/messages",
            "https://api.anthropic.com:8443/v1/messages",
            "https://user:pw@api.anthropic.com/v1/messages",
            "https://api.anthropic.com.evil.test/",
            "https://evil.test/",
            "https://127.0.0.1:11434/api/chat",
            "http://localhost:11434/api/chat",
            "http://127.0.0.2:11434/api/chat",
            "http://[::1]:11434/api/chat",
            "http://0.0.0.0:11434/api/chat",
            "http://192.168.1.2:11434/api/chat",
            "http://x@127.0.0.1:11434/api/chat",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn guard_caps_and_cancels() {
        let cancel = AtomicBool::new(false);
        let mut out = String::new();
        guard(&b"12345"[..], 5, &cancel).read_to_string(&mut out).unwrap();
        assert_eq!(out, "12345");
        assert!(guard(&b"123456"[..], 5, &cancel).read_to_string(&mut String::new()).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(guard(&b"1"[..], 5, &cancel).read_to_string(&mut String::new()).is_err());
    }
}
