//! Apple's on-device language model (`src/foundation_models.swift`): it runs on this Mac or
//! iPhone, so a request never leaves the device. Needs the macOS or iOS 26 SDK to build.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use ai::{AiError, Availability, LocalModel, Provider, Request};

unsafe extern "C" {
    fn tarn_model_availability() -> i32;
    fn tarn_model_respond(
        instructions: *const c_char,
        prompt: *const c_char,
        context: *mut c_void,
        text: extern "C" fn(*mut c_void, *const c_char) -> bool,
        done: extern "C" fn(*mut c_void, *const c_char),
    );
}

enum Message {
    Text(String),
    Done(Option<String>),
}

struct Context {
    tx: Sender<Message>,
    stop: Arc<AtomicBool>,
}

extern "C" fn text(context: *mut c_void, piece: *const c_char) -> bool {
    // SAFETY: `context` is the box `stream` leaked; Swift calls `text` only before `done` frees it.
    let context = unsafe { &*context.cast::<Context>() };
    // SAFETY: Swift passes a NUL-terminated string that lives for this call.
    let piece = unsafe { CStr::from_ptr(piece) }.to_string_lossy().into_owned();
    context.tx.send(Message::Text(piece)).is_ok() && !context.stop.load(Ordering::Relaxed)
}

extern "C" fn done(context: *mut c_void, error: *const c_char) {
    // SAFETY: `context` is the box `stream` leaked, and Swift calls `done` exactly once.
    let context = unsafe { Box::from_raw(context.cast::<Context>()) };
    // SAFETY: when not null, Swift passes a NUL-terminated string that lives for this call.
    let error = (!error.is_null()).then(|| unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned());
    context.tx.send(Message::Done(error)).ok();
}

fn c_text(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FoundationModel;

impl LocalModel for FoundationModel {
    fn availability(&self) -> Availability {
        // SAFETY: takes nothing and only reads the system model's state.
        match unsafe { tarn_model_availability() } {
            0 => Availability::Available,
            2 => Availability::NeedsSetup("Turn on Apple Intelligence in System Settings to use the on-device model.".into()),
            3 => Availability::NeedsSetup("The on-device model is still downloading. Try again later.".into()),
            4 => Availability::NotSupported("The on-device model needs macOS 26 or iOS 26.".into()),
            _ => Availability::NotSupported("This device cannot run Apple's on-device model.".into()),
        }
    }
}

impl Provider for FoundationModel {
    fn stream(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        let (instructions, prompt) = (c_text(&req.system), c_text(&req.user_message()));
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let context = Box::into_raw(Box::new(Context { tx, stop: stop.clone() }));
        // SAFETY: Swift copies both strings before returning; `done` takes back `context` once.
        unsafe { tarn_model_respond(instructions.as_ptr(), prompt.as_ptr(), context.cast(), text, done) };
        loop {
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
                return Err(AiError::Cancelled);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Message::Text(piece)) => on_text(&piece),
                Ok(Message::Done(None)) => return Ok(()),
                Ok(Message::Done(Some(code))) => return Err(error(&code)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(AiError::Protocol("the on-device model stopped".into())),
            }
        }
    }
}

fn error(code: &str) -> AiError {
    match code {
        "refused" => AiError::Refused,
        "too long" => AiError::Unavailable("This is too long for the on-device model. Attach less text.".into()),
        "language" => AiError::Unavailable("The on-device model does not support this language.".into()),
        "unavailable" => AiError::Unavailable("The on-device model needs macOS 26 or iOS 26.".into()),
        _ => AiError::Protocol("the on-device model could not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swift_codes_become_errors_and_nul_never_cuts_the_prompt() {
        assert!(matches!(error("refused"), AiError::Refused));
        assert!(matches!(error("too long"), AiError::Unavailable(_)));
        assert!(matches!(error("anything"), AiError::Protocol(_)));
        assert_eq!(c_text("a\0b").to_str().unwrap(), "ab");
    }
}
