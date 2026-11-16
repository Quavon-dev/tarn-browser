use std::time::Duration;

use browser_core::github::{FetchError, Hop, Reply};
use nsh_core::prelude::{run_on_main_thread, timer};
#[cfg(target_os = "android")]
use tarn_mobile_android::system;
#[cfg(target_os = "ios")]
use tarn_mobile_ios::system;
use url::Url;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod system {
    use std::cell::RefCell;

    thread_local! {
        pub static SPOKEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub fn share(_: &str) -> bool {
        false
    }

    pub fn speak(text: &str) -> bool {
        SPOKEN.with(|s| s.borrow_mut().push(text.to_owned()));
        true
    }

    pub fn speech_started() -> Option<bool> {
        Some(true)
    }

    pub fn stop_speaking() {
        SPOKEN.with(|s| s.borrow_mut().clear());
    }

    pub fn fetch(_: &str, _: u64) -> Result<(u16, Option<String>, Vec<u8>), String> {
        Err("This build has no network transport.".to_owned())
    }
}

pub struct Transport;

impl Hop for Transport {
    fn request(&self, url: &Url, max: u64) -> Result<Reply, FetchError> {
        if url.scheme() != "https" {
            return Err(FetchError::Refused("only HTTPS is allowed".into()));
        }
        let (status, location, body) = system::fetch(url.as_str(), max).map_err(FetchError::Failed)?;
        if body.len() as u64 > max {
            return Err(FetchError::Failed("The answer is too large.".into()));
        }
        Ok(Reply { status, location, body })
    }
}

const SPEECH_POLL: Duration = Duration::from_millis(100);

pub async fn share(url: String) -> bool {
    run_on_main_thread(move || system::share(&url)).await.unwrap_or(false)
}

pub async fn share_file(path: std::path::PathBuf) -> bool {
    if !cfg!(target_os = "ios") {
        return false;
    }
    match Url::from_file_path(&path) {
        Ok(url) => share(url.to_string()).await,
        Err(()) => false,
    }
}

pub async fn speak(text: String) -> bool {
    if !run_on_main_thread(move || system::speak(&text)).await.unwrap_or(false) {
        return false;
    }
    loop {
        match run_on_main_thread(system::speech_started).await.unwrap_or(Some(false)) {
            Some(started) => return started,
            None => timer(SPEECH_POLL).await,
        }
    }
}

pub fn stop_speaking() {
    drop(run_on_main_thread(system::stop_speaking));
}
