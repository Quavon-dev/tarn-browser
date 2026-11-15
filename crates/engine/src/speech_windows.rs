//! Read-aloud with Windows' on-device speech synthesizer, played through a media player.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Foundation::TypedEventHandler;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::MediaPlayer;
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::core::HSTRING;

/// The middle of the 0 to 1 range `speak` takes, as on macOS; Windows speaks it at normal speed.
pub const DEFAULT_RATE: f32 = 0.5;

/// Stops speaking when dropped.
pub struct Speech {
    synthesizer: SpeechSynthesizer,
    player: MediaPlayer,
    /// From `speak` until the audio ended, failed or was stopped.
    speaking: Arc<AtomicBool>,
    /// Bumped by `stop`, so audio still being made for a stopped utterance never plays.
    generation: Arc<AtomicU64>,
}

impl Speech {
    /// `None` when the system has no speech synthesizer.
    pub fn new() -> Option<Self> {
        let (synthesizer, player) = (SpeechSynthesizer::new().ok()?, MediaPlayer::new().ok()?);
        let speaking = Arc::new(AtomicBool::new(false));
        let ended = speaking.clone();
        player
            .MediaEnded(&TypedEventHandler::new(move |_, _| {
                ended.store(false, Ordering::Release);
                Ok(())
            }))
            .ok()?;
        let failed = speaking.clone();
        player
            .MediaFailed(&TypedEventHandler::new(move |_, _| {
                failed.store(false, Ordering::Release);
                Ok(())
            }))
            .ok()?;
        Some(Speech { synthesizer, player, speaking, generation: Arc::default() })
    }

    /// Speaks `text` in place of what is playing. `rate` runs from 0 to 1; `language` is a BCP 47
    /// tag, and without one, or without a voice for it, the system voice speaks.
    pub fn speak(&self, text: &str, rate: f32, language: Option<&str>) {
        if let Ok(options) = self.synthesizer.Options() {
            let _ = options.SetSpeakingRate(speaking_rate(rate));
        }
        if let Some(voice) = language.and_then(voice_for) {
            let _ = self.synthesizer.SetVoice(&voice);
        }
        let Ok(made) = self.synthesizer.SynthesizeTextToStreamAsync(&HSTRING::from(text)) else { return };
        self.speaking.store(true, Ordering::Release);
        let (player, speaking, generation) = (self.player.clone(), self.speaking.clone(), self.generation.clone());
        let asked = generation.load(Ordering::Acquire);
        // Making the audio takes a moment, so it is waited for off the main thread.
        std::thread::spawn(move || {
            let played = (|| {
                let stream = made.get()?;
                if generation.load(Ordering::Acquire) != asked {
                    return Ok(());
                }
                player.SetSource(&MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?)?;
                player.Play()
            })();
            if played.is_err() {
                speaking.store(false, Ordering::Release);
            }
        });
    }

    /// True while an utterance is being made, playing or paused.
    pub fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        let _ = self.player.Pause();
    }

    pub fn resume(&self) {
        let _ = self.player.Play();
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.player.Pause();
        let _ = self.player.SetSource(None);
        self.speaking.store(false, Ordering::Release);
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        self.stop();
    }
}

/// macOS's 0 to 1 rate, normal at a half, as Windows' multiple of normal speed (0.5 to 3 here).
fn speaking_rate(rate: f32) -> f64 {
    let rate = f64::from(rate.clamp(0., 1.));
    if rate <= 0.5 { 0.5 + rate } else { 1.0 + (rate - 0.5) * 4.0 }
}

/// The first installed voice whose language starts with `language`, ignoring case.
fn voice_for(language: &str) -> Option<VoiceInformation> {
    let language = language.to_ascii_lowercase();
    SpeechSynthesizer::AllVoices().ok()?.into_iter().find(|v| v.Language().is_ok_and(|l| l.to_string().to_ascii_lowercase().starts_with(&language)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rates_map_onto_windows_speeds() {
        assert_eq!(super::speaking_rate(super::DEFAULT_RATE), 1.0);
        assert_eq!(super::speaking_rate(0.), 0.5);
        assert_eq!(super::speaking_rate(1.), 3.0);
    }
}
