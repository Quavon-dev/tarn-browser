//! The window's mini-player: which tab's media it shows and the commands its buttons send. Shells
//! ask `page_ops::MediaState` of a tab once the engine says it plays, and again every `POLL` only
//! while the shown media plays, so nothing ticks while nothing plays.

use std::collections::HashSet;
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

/// The tabs whose video Tarn moved into Picture in Picture when they left the screen, so only
/// those come back out when the tab returns; one the user moved there stays.
#[derive(Clone, Debug, Default)]
pub struct AutoPip {
    entered: HashSet<TabKey>,
    screen: Vec<TabKey>,
}

impl AutoPip {
    /// The tabs on screen are now `now`: answers those that left it, whose playing video should
    /// move into Picture in Picture, and those Tarn moved there that are back, whose video
    /// should come out.
    pub fn screen(&mut self, now: Vec<TabKey>) -> (Vec<TabKey>, Vec<TabKey>) {
        let left = self.screen.iter().copied().filter(|k| !now.contains(k)).collect();
        let back = now.iter().copied().filter(|k| self.entered.remove(k)).collect();
        self.screen = now;
        (left, back)
    }

    /// `key`'s page answered `result` to `PipMode::Enter`; true when its video should come back out
    /// at once, because the tab is `visible` again before the answer arrived.
    pub fn answered(&mut self, key: TabKey, result: &str, visible: bool) -> bool {
        let entered = result == "pip";
        if entered && !visible {
            self.entered.insert(key);
        }
        entered && visible
    }

    /// True once for a tab Tarn moved into Picture in Picture: on its return, or when it goes.
    pub fn take(&mut self, key: TabKey) -> bool {
        self.entered.remove(&key)
    }

    /// `key`'s media answered `state`: true when a video Tarn moved into Picture in Picture came
    /// back still playing, as the Picture in Picture window's button to return does, so its tab
    /// should show. Closing that window pauses the video, which shows nothing.
    pub fn returned(&mut self, key: TabKey, state: Option<&MediaState>) -> bool {
        match state {
            Some(state) if !state.pip && self.entered.contains(&key) => {
                self.entered.remove(&key);
                state.playing
            }
            _ => false,
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
        Some(MediaState { playing, title: title.into(), duration: Some(100.), position: Some(1.), pip: false })
    }

    #[test]
    fn a_video_returned_from_picture_in_picture_brings_its_tab_back() {
        let mut pip = AutoPip::default();
        pip.screen(vec![key(1)]);
        pip.screen(vec![key(2)]);
        assert!(!pip.answered(key(1), "pip", false));
        let shown = MediaState { pip: true, ..state(true, "Film").unwrap() };
        assert!(!pip.returned(key(1), Some(&shown)), "still in Picture in Picture");
        assert!(pip.returned(key(1), state(true, "Film").as_ref()), "back inline, still playing");
        assert!(!pip.take(key(1)), "answered once");
        assert!(!pip.answered(key(3), "pip", false));
        assert!(!pip.returned(key(3), state(false, "Film").as_ref()), "closed: the video paused");
        assert!(!pip.returned(key(2), state(true, "Song").as_ref()), "a tab Tarn did not move");
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
    fn tabs_leave_and_return_once_each() {
        let mut pip = AutoPip::default();
        assert_eq!(pip.screen(vec![key(1), key(2)]), (vec![], vec![]));
        assert_eq!(pip.screen(vec![key(1), key(2)]), (vec![], vec![]), "nothing changed");
        assert_eq!(pip.screen(vec![key(3)]), (vec![key(1), key(2)], vec![]), "a whole split leaves");
        pip.answered(key(1), "pip", false);
        assert_eq!(pip.screen(vec![key(1)]), (vec![key(3)], vec![key(1)]));
        assert_eq!(pip.screen(vec![key(1), key(2)]), (vec![], vec![]), "back out only once");
    }

    #[test]
    fn only_video_tarn_moved_comes_back_out() {
        let mut pip = AutoPip::default();
        assert!(!pip.answered(key(1), "pip", false));
        assert!(!pip.answered(key(2), "open", false), "the user's own window is not Tarn's");
        assert!(!pip.answered(key(3), "refused", false));
        assert!(pip.take(key(1)) && !pip.take(key(1)), "back out once");
        assert!(!pip.take(key(2)) && !pip.take(key(3)));
        assert!(pip.answered(key(4), "pip", true), "back on screen before the answer: out at once");
        assert!(!pip.take(key(4)));
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
