//! Read-aloud with the system's on-device speech synthesizer (`AVSpeechSynthesizer`). There is no
//! binding crate in the lockfile, so the few calls are sent directly.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

/// `AVSpeechUtteranceDefaultSpeechRate`; rates run from 0 to 1.
pub const DEFAULT_RATE: f32 = 0.5;

/// Stops speaking when dropped.
pub struct Speech(Retained<AnyObject>);

impl Speech {
    /// `None` when the system has no speech synthesizer.
    pub fn new() -> Option<Self> {
        let class = AnyClass::get(c"AVSpeechSynthesizer")?;
        // SAFETY: `new` on an NSObject subclass returns a retained, initialised instance.
        Some(Speech(unsafe { msg_send![class, new] }))
    }

    /// Queues `text` after what is already queued. `language` is a BCP 47 tag; without one, or
    /// without a voice for it, the system voice speaks.
    pub fn speak(&self, text: &str, rate: f32, language: Option<&str>) {
        let (Some(utterance), Some(voice)) = (AnyClass::get(c"AVSpeechUtterance"), AnyClass::get(c"AVSpeechSynthesisVoice")) else {
            return;
        };
        // SAFETY: documented AVFAudio class methods and setters with the argument types they
        // declare (NSString, float, AVSpeechSynthesisVoice); `voiceWithLanguage:` may return nil.
        unsafe {
            let utterance: Retained<AnyObject> = msg_send![utterance, speechUtteranceWithString: &*NSString::from_str(text)];
            let _: () = msg_send![&*utterance, setRate: rate.clamp(0., 1.)];
            if let Some(language) = language {
                let voice: Option<Retained<AnyObject>> = msg_send![voice, voiceWithLanguage: &*NSString::from_str(language)];
                if let Some(voice) = voice {
                    let _: () = msg_send![&*utterance, setVoice: &*voice];
                }
            }
            let _: () = msg_send![&*self.0, speakUtterance: &*utterance];
        }
    }

    /// True while an utterance is queued, playing or paused.
    pub fn is_speaking(&self) -> bool {
        // SAFETY: a BOOL property of a live synthesizer.
        unsafe { msg_send![&*self.0, isSpeaking] }
    }

    pub fn pause(&self) {
        // SAFETY: `AVSpeechBoundaryImmediate` is 0 in the NSInteger enum the method takes.
        let _: bool = unsafe { msg_send![&*self.0, pauseSpeakingAtBoundary: 0isize] };
    }

    pub fn resume(&self) {
        // SAFETY: a method without arguments returning BOOL on a live synthesizer.
        let _: bool = unsafe { msg_send![&*self.0, continueSpeaking] };
    }

    pub fn stop(&self) {
        // SAFETY: as in `pause`.
        let _: bool = unsafe { msg_send![&*self.0, stopSpeakingAtBoundary: 0isize] };
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        self.stop();
    }
}
