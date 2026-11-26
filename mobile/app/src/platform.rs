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

    pub fn confirm_owner(_: &str) -> bool {
        false
    }

    pub fn owner_answer() -> Option<bool> {
        Some(false)
    }

    pub fn fetch(_: &str, _: u64) -> Result<(u16, Option<String>, Vec<u8>), String> {
        Err("This build has no network transport.".to_owned())
    }
}

#[cfg(all(target_os = "ios", feature = "foundation-models"))]
const ON_DEVICE: Option<(&dyn ai::LocalModel, &str)> = Some((&engine::foundation_models::FoundationModel, browser_core::assistant::settings::ON_DEVICE));

#[cfg(all(target_os = "ios", not(feature = "foundation-models")))]
const ON_DEVICE: Option<(&dyn ai::LocalModel, &str)> = None;

#[cfg(target_os = "ios")]
pub static ASSISTANT: tarn_nsh_shared::assistant::Platform = tarn_nsh_shared::assistant::Platform {
    secrets: &ai::secrets::Keyring,
    transport: || std::sync::Arc::new(ai::UreqTransport),
    keyring: "the Keychain on this device",
    ollama_gap: Some("Ollama does not run on iOS."),
    on_device: ON_DEVICE,
};

#[cfg(target_os = "android")]
pub static ASSISTANT: tarn_nsh_shared::assistant::Platform = tarn_nsh_shared::assistant::Platform {
    secrets: &tarn_mobile_android::Keystore,
    transport: || std::sync::Arc::new(tarn_mobile_android::HostTransport),
    keyring: "the Android Keystore",
    ollama_gap: Some("Android sends assistant requests over HTTPS only, and Ollama answers over plain HTTP."),
    on_device: None,
};

/// A build for neither phone keeps no key and sends nothing.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub static ASSISTANT: tarn_nsh_shared::assistant::Platform = tarn_nsh_shared::assistant::Platform {
    secrets: &Unavailable,
    transport: || std::sync::Arc::new(Unavailable),
    keyring: "this build's secret store",
    ollama_gap: Some("This build has no network transport."),
    on_device: None,
};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
#[derive(Debug)]
struct Unavailable;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
impl ai::Secrets for Unavailable {
    fn get(&self, _: &str, _: &str) -> Result<Option<String>, ai::AiError> {
        Err(ai::AiError::Secrets("this build has none".into()))
    }

    fn set(&self, _: &str, _: &str, _: &str) -> Result<(), ai::AiError> {
        Err(ai::AiError::Secrets("this build has none".into()))
    }

    fn delete(&self, _: &str, _: &str) -> Result<(), ai::AiError> {
        Ok(())
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
impl ai::Transport for Unavailable {
    fn post<'a>(
        &self,
        _: &str,
        _: &[(&str, &str)],
        _: &[u8],
        _: u64,
        _: &'a std::sync::atomic::AtomicBool,
    ) -> Result<ai::transport::Response<'a>, ai::AiError> {
        Err(ai::AiError::Network("This build has no network transport.".into()))
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

#[cfg(target_os = "android")]
pub async fn share_file(path: std::path::PathBuf) -> bool {
    run_on_main_thread(move || system::share_file(&path)).await.unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
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

/// Asks the device's owner to confirm with biometrics or the device passcode; `reason` completes
/// "Tarn is trying to …" where the system shows it.
pub async fn confirm_owner(reason: String) -> bool {
    if !run_on_main_thread(move || system::confirm_owner(&reason)).await.unwrap_or(false) {
        return false;
    }
    loop {
        match run_on_main_thread(system::owner_answer).await.unwrap_or(Some(false)) {
            Some(ok) => return ok,
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
