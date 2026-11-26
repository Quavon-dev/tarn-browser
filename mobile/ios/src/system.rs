use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};

use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSError, NSString, NSURL};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}

const BOUNDARY_IMMEDIATE: isize = 0;
const DEVICE_OWNER_AUTHENTICATION: isize = 2;
const OWNER_ASKING: u8 = 0;
const OWNER_CONFIRMED: u8 = 1;
const OWNER_REFUSED: u8 = 2;

static OWNER: AtomicU8 = AtomicU8::new(OWNER_REFUSED);

thread_local! {
    static SYNTHESIZER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

fn top_controller() -> Option<Retained<AnyObject>> {
    let app_class = AnyClass::get(c"UIApplication")?;
    // SAFETY: main thread (checked by the callers); UIApplication's shared instance and its key window's
    // controllers are plain UIKit getters that return retained objects or nil.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![app_class, sharedApplication];
        let window: Option<Retained<AnyObject>> = msg_send![&*app?, keyWindow];
        let mut controller: Retained<AnyObject> = msg_send![&*window?, rootViewController];
        loop {
            let presented: Option<Retained<AnyObject>> = msg_send![&*controller, presentedViewController];
            match presented {
                Some(next) => controller = next,
                None => return Some(controller),
            }
        }
    }
}

pub fn share(url: &str) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let (Some(link), Some(class), Some(controller)) =
        (NSURL::URLWithString(&NSString::from_str(url)), AnyClass::get(c"UIActivityViewController"), top_controller())
    else {
        return false;
    };
    let item: &AnyObject = &link;
    let items = NSArray::from_slice(&[item]);
    // SAFETY: main thread (marker); UIActivityViewController takes an array of activity items and no
    // custom activities; on iPad its popover is anchored to the presenting view before presenting.
    unsafe {
        let alloc: Allocated<AnyObject> = msg_send![class, alloc];
        let sheet: Retained<AnyObject> = msg_send![alloc, initWithActivityItems: &*items, applicationActivities: std::ptr::null::<AnyObject>()];
        let popover: Option<Retained<AnyObject>> = msg_send![&*sheet, popoverPresentationController];
        let view: Option<Retained<AnyObject>> = msg_send![&*controller, view];
        if let (Some(popover), Some(view)) = (popover, view) {
            let bounds: CGRect = msg_send![&*view, bounds];
            let anchor = CGRect { origin: CGPoint { x: bounds.size.width / 2., y: bounds.size.height - 60. }, size: CGSize { width: 1., height: 1. } };
            let _: () = msg_send![&*popover, setSourceView: &*view];
            let _: () = msg_send![&*popover, setSourceRect: anchor];
        }
        let _: () = msg_send![&*controller, presentViewController: &*sheet, animated: true, completion: std::ptr::null::<AnyObject>()];
    }
    true
}

pub fn speak(text: &str) -> bool {
    let (Some(_), Some(synthesizer), Some(utterance)) = (MainThreadMarker::new(), AnyClass::get(c"AVSpeechSynthesizer"), AnyClass::get(c"AVSpeechUtterance"))
    else {
        return false;
    };
    SYNTHESIZER.with(|s| {
        let mut s = s.borrow_mut();
        // SAFETY: main thread (marker); AVSpeechSynthesizer speaks on the device with the system voice,
        // and the utterance is a fresh object made from an owned string.
        unsafe {
            if s.is_none() {
                let made: Option<Retained<AnyObject>> = msg_send![synthesizer, new];
                *s = made;
            }
            let Some(speaker) = s.as_ref() else { return false };
            let _: bool = msg_send![&**speaker, stopSpeakingAtBoundary: BOUNDARY_IMMEDIATE];
            let spoken: Option<Retained<AnyObject>> = msg_send![utterance, speechUtteranceWithString: &*NSString::from_str(text)];
            let Some(spoken) = spoken else { return false };
            let _: () = msg_send![&**speaker, speakUtterance: &*spoken];
            true
        }
    })
}

pub fn speech_started() -> Option<bool> {
    Some(true)
}

pub fn stop_speaking() {
    let _ = SYNTHESIZER.try_with(|s| {
        if let Some(speaker) = s.borrow().as_ref() {
            // SAFETY: the synthesizer was made on this (main) thread and lives in its thread-local.
            let _: bool = unsafe { msg_send![&**speaker, stopSpeakingAtBoundary: BOUNDARY_IMMEDIATE] };
        }
    });
}

/// Asks the device's owner with Face ID, Touch ID or the passcode; `owner_answer` reads the reply.
pub fn confirm_owner(reason: &str) -> bool {
    let (Some(_), Some(class)) = (MainThreadMarker::new(), AnyClass::get(c"LAContext")) else { return false };
    if OWNER.swap(OWNER_ASKING, Ordering::SeqCst) == OWNER_ASKING {
        return false;
    }
    // SAFETY: main thread (marker); a fresh LAContext evaluates the device-owner policy and calls
    // the reply once, on a queue of its own; the block holds the context until then.
    unsafe {
        let context: Option<Retained<AnyObject>> = msg_send![class, new];
        let Some(context) = context else {
            OWNER.store(OWNER_REFUSED, Ordering::SeqCst);
            return false;
        };
        let held = context.clone();
        let reply = RcBlock::new(move |ok: Bool, _: *mut NSError| {
            let _ = &held;
            OWNER.store(if ok.as_bool() { OWNER_CONFIRMED } else { OWNER_REFUSED }, Ordering::SeqCst);
        });
        let _: () = msg_send![&*context, evaluatePolicy: DEVICE_OWNER_AUTHENTICATION, localizedReason: &*NSString::from_str(reason), reply: &*reply];
    }
    true
}

pub fn owner_answer() -> Option<bool> {
    match OWNER.load(Ordering::SeqCst) {
        OWNER_ASKING => None,
        state => Some(state == OWNER_CONFIRMED),
    }
}

pub use crate::fetch::fetch;
