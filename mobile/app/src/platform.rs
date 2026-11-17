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

#[cfg(target_os = "android")]
pub use tarn_mobile_android::tarn_ext as ext;
#[cfg(target_os = "ios")]
pub use tarn_mobile_ios::tarn_ext as ext;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod ext {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use engine::tarn_ext::{FrameNav, Resources, World, WorldScripts};

    pub type Gate = Rc<dyn Fn(&str, &str, &World, &FrameNav<'_>) -> Vec<usize>>;

    thread_local! {
        pub static GATE: RefCell<Option<Gate>> = RefCell::new(None);
        pub static SCRIPTS: RefCell<HashMap<String, Vec<Rc<WorldScripts>>>> = RefCell::new(HashMap::new());
        pub static RULES: RefCell<HashMap<String, Vec<(String, String)>>> = RefCell::new(HashMap::new());
        pub static SERVED: RefCell<HashMap<String, Resources>> = RefCell::new(HashMap::new());
    }

    pub fn set_gate(gate: Gate) {
        GATE.with(|g| *g.borrow_mut() = Some(gate));
    }

    pub fn set_scripts(store_id: &str, worlds: Vec<Rc<WorldScripts>>) {
        SCRIPTS.with(|s| s.borrow_mut().insert(store_id.to_owned(), worlds));
    }

    pub fn set_rule_lists(store_id: &str, lists: Vec<(String, String)>) {
        RULES.with(|r| r.borrow_mut().insert(store_id.to_owned(), lists));
    }

    pub fn serve(store_id: &str, resources: Resources) {
        SERVED.with(|s| s.borrow_mut().insert(store_id.to_owned(), resources));
    }

    pub fn worlds_supported() -> bool {
        true
    }

    pub fn frame_event(_: &serde_json::Value) {}
}
