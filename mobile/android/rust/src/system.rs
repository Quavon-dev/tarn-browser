pub fn share(url: &str) -> bool {
    crate::host::call_bool("share", url)
}

pub fn speak(text: &str) -> bool {
    crate::host::call_string("speak", text)
}

pub fn speech_started() -> Option<bool> {
    match crate::host::call_int("speechState") {
        Some(0) => None,
        state => Some(state == Some(1)),
    }
}

pub fn stop_speaking() {
    crate::host::call_string("stopSpeaking", "");
}

pub fn fetch(url: &str, max: u64) -> crate::host::Fetched {
    use browser_core::github::{CONNECT_TIMEOUT_SECS, TIMEOUT_SECS, USER_AGENT};
    crate::host::fetch(url, max, USER_AGENT, CONNECT_TIMEOUT_SECS * 1000, TIMEOUT_SECS * 1000)
}
