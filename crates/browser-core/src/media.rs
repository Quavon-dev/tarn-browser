//! The window's mini-player: which tab's media it shows and the commands its buttons send. Shells
//! ask `page_ops::MediaState` of a tab once the engine says it plays, and again every `POLL` only
//! while the shown media plays, so nothing ticks while nothing plays.

use std::time::Duration;

use engine::page_scripts::MediaCommand;

use crate::page_ops::MediaState;
use crate::tabs::TabKey;

/// How often a shell asks the shown tab for its media while it plays.
pub const POLL: Duration = Duration::from_secs(2);
/// What the mini-player's back and forward buttons skip, in seconds.
pub const SKIP: f64 = 15.;

/// The mini-player's buttons, in their order, with their accessible names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Back,
    PlayPause,
    Forward,
}

impl Control {
    pub const ALL: [Control; 3] = [Control::Back, Control::PlayPause, Control::Forward];

    pub fn label(self, playing: bool) -> &'static str {
        match self {
            Control::Back => "Back 15 Seconds",
            Control::PlayPause if playing => "Pause",
            Control::PlayPause => "Play",
            Control::Forward => "Forward 15 Seconds",
        }
    }

    pub fn glyph(self, playing: bool) -> &'static str {
        match self {
            Control::Back => "chevron-left",
            Control::PlayPause if playing => "pause",
            Control::PlayPause => "play",
            Control::Forward => "chevron-right",
        }
    }

    pub fn command(self, playing: bool) -> MediaCommand {
        match self {
            Control::Back => MediaCommand::SeekBy(-SKIP),
            Control::PlayPause if playing => MediaCommand::Pause,
            Control::PlayPause => MediaCommand::Play,
            Control::Forward => MediaCommand::SeekBy(SKIP),
        }
    }

    /// `command` as the NSH component's `media` command arguments.
    pub fn args(self, playing: bool) -> serde_json::Value {
        match self.command(playing) {
            MediaCommand::Play => serde_json::json!({ "action": "play" }),
            MediaCommand::Pause => serde_json::json!({ "action": "pause" }),
            MediaCommand::SeekBy(seconds) => serde_json::json!({ "action": "seekBy", "seconds": seconds }),
        }
    }
}

/// The tab whose media the mini-player shows. It takes a tab when its media plays, keeps it while
/// paused, so the user can resume, and lets it go once the tab has no media or closes. Another
/// tab that starts playing takes over from a paused one, not from one still playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    shown: Option<(TabKey, MediaState)>,
}

impl Player {
    /// Takes a tab's answer (`None`: no audible media); true when what shows changed.
    pub fn answer(&mut self, key: TabKey, state: Option<MediaState>) -> bool {
        let before = self.shown.clone();
        match (&self.shown, state) {
            (Some((shown, _)), None) if *shown == key => self.shown = None,
            (_, None) => {}
            (Some((shown, current)), Some(state)) if *shown != key && (current.playing || !state.playing) => {}
            (_, Some(state)) => self.shown = Some((key, state)),
        }
        self.shown != before
    }

    /// Drops `key` once its tab closed, slept or left the window; true when it was shown.
    pub fn forget(&mut self, key: TabKey) -> bool {
        self.shown.as_ref().is_some_and(|(k, _)| *k == key) && self.shown.take().is_some()
    }

    pub fn tab(&self) -> Option<TabKey> {
        self.shown.as_ref().map(|(k, _)| *k)
    }

    /// What the mini-player shows; nothing while `visible`, the tab on screen, is the media's own,
    /// whose page has its controls.
    pub fn shown(&self, visible: &[TabKey]) -> Option<(TabKey, &MediaState)> {
        self.shown.as_ref().filter(|(k, _)| !visible.contains(k)).map(|(k, s)| (*k, s))
    }

    /// Whether the shell should ask the shown tab again after `POLL`: only while its media plays.
    pub fn polls(&self) -> bool {
        self.shown.as_ref().is_some_and(|(_, s)| s.playing)
    }

    /// The state after the user pressed `control`, shown until the page's next answer.
    pub fn pressed(&mut self, control: Control) {
        if let (Some((_, state)), Control::PlayPause) = (&mut self.shown, control) {
            state.playing = !state.playing;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: store::Id) -> TabKey {
        TabKey { private: false, id }
    }

    fn state(playing: bool, title: &str) -> Option<MediaState> {
        Some(MediaState { playing, title: title.into(), duration: Some(100.), position: Some(1.) })
    }

    #[test]
    fn a_playing_tab_shows_stays_while_paused_and_goes_without_media() {
        let mut player = Player::default();
        assert!(!player.polls() && player.tab().is_none());
        assert!(player.answer(key(1), state(true, "Song")));
        assert!(player.polls());
        assert!(player.answer(key(1), state(false, "Song")));
        assert_eq!(player.tab(), Some(key(1)), "paused media stays to be resumed");
        assert!(!player.polls(), "nothing polls while paused");
        assert!(!player.answer(key(2), None));
        assert!(player.answer(key(1), None));
        assert_eq!(player.tab(), None);
    }

    #[test]
    fn another_tab_takes_over_only_from_paused_media() {
        let mut player = Player::default();
        player.answer(key(1), state(true, "One"));
        assert!(!player.answer(key(2), state(true, "Two")), "the first stays while it plays");
        player.answer(key(1), state(false, "One"));
        assert!(!player.answer(key(2), state(false, "Two")), "a paused tab does not take over");
        assert!(player.answer(key(2), state(true, "Two")));
        assert_eq!(player.tab(), Some(key(2)));
        assert!(!player.forget(key(1)));
        assert!(player.forget(key(2)) && player.tab().is_none());
    }

    #[test]
    fn the_visible_tab_shows_no_mini_player() {
        let mut player = Player::default();
        player.answer(key(1), state(true, "Song"));
        assert_eq!(player.shown(&[key(1)]), None);
        assert_eq!(player.shown(&[key(2)]).map(|(k, s)| (k, s.title.as_str())), Some((key(1), "Song")));
    }

    #[test]
    fn controls_skip_fifteen_seconds_and_toggle_play() {
        assert_eq!(Control::Back.command(true), MediaCommand::SeekBy(-SKIP));
        assert_eq!(Control::Forward.command(false), MediaCommand::SeekBy(SKIP));
        assert_eq!(Control::PlayPause.command(true), MediaCommand::Pause);
        assert_eq!((Control::PlayPause.label(false), Control::PlayPause.glyph(true)), ("Play", "pause"));
        for control in Control::ALL {
            for playing in [true, false] {
                assert_eq!(crate::page_ops::media_command(&control.args(playing)), Some(control.command(playing)));
            }
        }
        let mut player = Player::default();
        player.answer(key(1), state(true, "Song"));
        player.pressed(Control::PlayPause);
        assert!(!player.polls());
        player.pressed(Control::Forward);
        assert_eq!(player.shown(&[]).map(|(_, s)| s.playing), Some(false));
    }
}
