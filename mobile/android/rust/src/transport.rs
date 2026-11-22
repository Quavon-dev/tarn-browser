use std::sync::atomic::{AtomicBool, Ordering};

use ai::AiError;
use ai::transport::{Response, Transport, guard};

const STALL_MS: u64 = 60_000;
const TOTAL_MS: u64 = 600_000;

/// Sends AI requests through the Java host's `Fetch`, so they get its TLS key policy and proxy
/// settings instead of a second TLS stack that would need its own JNI setup.
#[derive(Debug, Default)]
pub struct HostTransport;

impl Transport for HostTransport {
    fn post<'a>(&self, url: &str, headers: &[(&str, &str)], body: &[u8], max: u64, cancel: &'a AtomicBool) -> Result<Response<'a>, AiError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        let (status, stream) = crate::host::post(url, headers, body, STALL_MS, TOTAL_MS).map_err(AiError::Network)?;
        Ok(Response { status, body: guard(stream, max, cancel) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_java_host_requests_fail() {
        let cancel = AtomicBool::new(false);
        assert!(matches!(HostTransport.post("https://api.anthropic.com/v1/messages", &[], b"{}", 1, &cancel), Err(AiError::Network(_))));
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(HostTransport.post("https://api.anthropic.com/v1/messages", &[], b"{}", 1, &cancel), Err(AiError::Cancelled)));
    }
}
