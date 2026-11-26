pub fn share(url: &str) -> bool {
    crate::host::call_bool("share", url)
}

pub fn share_file(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(|path| crate::host::call_bool("shareFile", path))
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

/// Asks the device's owner with biometrics or the screen lock; the check's number for
/// `owner_answer`, or 0 when it could not start.
pub fn confirm_owner(reason: &str) -> u64 {
    crate::host::call_string_int("confirmOwner", reason).and_then(|n| u64::try_from(n).ok()).unwrap_or(0)
}

/// `None` while check `request` runs; a newer check refuses it.
pub fn owner_answer(request: u64) -> Option<bool> {
    match crate::host::call_string_int("ownerState", &request.to_string()) {
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
