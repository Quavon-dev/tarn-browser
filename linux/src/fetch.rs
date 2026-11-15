use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use browser_core::github::{CONNECT_TIMEOUT_SECS, FetchError, Hop, Reply, TIMEOUT_SECS, USER_AGENT};
use url::Url;
use webkit6::prelude::*;
use webkit6::{gio, soup};

pub struct Soup(soup::Session);

impl Soup {
    pub fn new() -> Soup {
        let session = soup::Session::new();
        session.set_user_agent(USER_AGENT);
        session.set_proxy_resolver(None::<&gio::ProxyResolver>);
        session.set_timeout(CONNECT_TIMEOUT_SECS as u32);
        session.set_idle_timeout(CONNECT_TIMEOUT_SECS as u32);
        session.set_accept_language_auto(false);
        Soup(session)
    }
}

fn failed(e: impl std::fmt::Display) -> FetchError {
    FetchError::Failed(e.to_string())
}

impl Hop for Soup {
    fn request(&self, url: &Url, max: u64) -> Result<Reply, FetchError> {
        if url.scheme() != "https" {
            return Err(FetchError::Refused("only HTTPS is allowed".into()));
        }
        let cancellable = gio::Cancellable::new();
        let (finished, watched) = channel::<()>();
        let watchdog = cancellable.clone();
        std::thread::spawn(move || {
            if watched.recv_timeout(Duration::from_secs(TIMEOUT_SECS)) == Err(RecvTimeoutError::Timeout) {
                watchdog.cancel();
            }
        });
        let message = soup::Message::new("GET", url.as_str()).map_err(failed)?;
        message.set_flags(soup::MessageFlags::NO_REDIRECT | soup::MessageFlags::DO_NOT_USE_AUTH_CACHE);
        let stream = self.0.send(&message, Some(&cancellable)).map_err(failed)?;
        let status = message.status_code() as u16;
        let location = message.response_headers().and_then(|h| h.one("Location")).map(|l| l.to_string());
        let mut body = Vec::new();
        if status == 200 {
            let mut chunk = vec![0u8; 16 * 1024];
            loop {
                let n = stream.read(&mut chunk[..], Some(&cancellable)).map_err(failed)?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
                if body.len() as u64 > max {
                    return Err(FetchError::Failed("The answer is too large.".into()));
                }
            }
        }
        drop(finished);
        Ok(Reply { status, location, body })
    }
}
