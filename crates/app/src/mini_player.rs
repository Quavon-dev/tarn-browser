//! The sidebar's mini-player for media playing in a tab that is not on screen.

use std::rc::Rc;

use browser_core::media::{Control, POLL};
use browser_core::page_ops::MediaState;
use engine::page_scripts::PipMode;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px};
use ui::controls::{icon_button, surface, tab_stop, tooltip};
use ui::icon::{ICON, Icon, icon};
use ui::motion::pop_in;
use ui::text::OneLine;
use ui::theme::{TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};

#[derive(Default)]
pub struct MiniPlayer {
    pub(crate) player: browser_core::media::Player,
    auto_pip: browser_core::media::AutoPip,
    poll: Option<Task<()>>,
    /// A tab whose video came back from Picture in Picture by its button, to show on the next frame.
    returned: Option<TabKey>,
}

impl Browser {
    /// Asks `key`'s page for its media once and shows the answer.
    pub(crate) fn ask_media(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
        view.media_state(move |raw| drop(tx.unbounded_send(raw)));
        cx.spawn(async move |this, cx| {
            let Some(raw) = rx.next().await else { return };
            this.update(cx, |b, cx| {
                if b.same_view(key, &weak) {
                    let state = MediaState::parse(&raw);
                    if b.mini_player.auto_pip.returned(key, state.as_ref()) {
                        b.mini_player.returned = Some(key);
                        cx.notify();
                    }
                    if b.mini_player.player.answer(key, state) {
                        cx.notify();
                    }
                    b.poll_media(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// One timer while the shown media plays; none otherwise.
    fn poll_media(&mut self, cx: &mut Context<Self>) {
        if !self.mini_player.player.polls() || self.mini_player.poll.is_some() {
            return;
        }
        self.mini_player.poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POLL).await;
            this.update(cx, |b, cx| {
                b.mini_player.poll = None;
                if let Some(key) = b.mini_player.player.tab().filter(|_| b.mini_player.player.polls()) {
                    b.ask_media(key, cx);
                }
            })
            .ok();
        }));
    }

    /// Drops `key` from the mini-player once its view is gone; a video Tarn moved into Picture in
    /// Picture comes back out, since the tab closes or shows in another window.
    pub(crate) fn forget_media(&mut self, key: TabKey) {
        self.mini_player.player.forget(key);
        if self.mini_player.auto_pip.take(key) {
            self.exit_pip(key);
        }
    }

    /// The tabs on screen: the active tab, or every pane of its split, and a tab still fading out.
    fn screen_tabs(&self) -> Vec<TabKey> {
        let active = self.model.active;
        let mut tabs = active.and_then(|k| self.splits.of(k)).map_or_else(|| active.into_iter().collect(), |s| s.panes().to_vec());
        let fading = self.page_fade.as_ref().and_then(|(view, _)| self.views.iter().find(|(_, v)| Rc::ptr_eq(v, view)));
        tabs.extend(fading.map(|(k, _)| *k).filter(|k| !tabs.contains(k)));
        tabs
    }

    /// Shows the tab whose video its Picture in Picture window sent back, with Tarn in front.
    pub(crate) fn show_returned_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.mini_player.returned.take() {
            cx.defer_in(window, move |b, window, cx| {
                window.activate_window();
                b.open_media_tab(key, window, cx);
            });
        }
    }

    /// Called on every render, after whatever changed the tabs on screen: a playing video of a tab
    /// that left moves into Picture in Picture, and one Tarn moved there comes back with its tab.
    pub(crate) fn settle_screen(&mut self, cx: &mut Context<Self>) {
        let (left, back) = self.mini_player.auto_pip.screen(self.screen_tabs());
        for key in left {
            self.leave_for_pip(key, cx);
        }
        for key in back {
            self.exit_pip(key);
        }
    }

    /// Asks the engine first whether the page plays (on WebKit its own playback state), so a page
    /// that plays nothing gets no script, and with it no gesture, when its tab leaves the screen.
    fn leave_for_pip(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key).filter(|v| self.may_pip(key, v)) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
        view.media_playing(move |playing| drop(tx.unbounded_send(playing)));
        cx.spawn(async move |this, cx| {
            if rx.next().await != Some(true) {
                return;
            }
            this.update(cx, |b, cx| {
                let Some(view) = weak.upgrade().filter(|v| b.same_view(key, &weak) && !b.screen_tabs().contains(&key) && b.may_pip(key, v)) else { return };
                let (tx, mut rx) = mpsc::unbounded();
                view.picture_in_picture(PipMode::Enter, move |result| drop(tx.unbounded_send(result)));
                cx.spawn(async move |this, cx| {
                    let Some(result) = rx.next().await else { return };
                    this.update(cx, |b, _| {
                        if !b.same_view(key, &weak) {
                            // The view moved to another window or was replaced meanwhile.
                            if let Some(view) = weak.upgrade().filter(|_| result == "pip") {
                                view.picture_in_picture(PipMode::Exit, |_| {});
                            }
                        } else if b.mini_player.auto_pip.answered(key, &result, b.screen_tabs().contains(&key)) {
                            b.exit_pip(key);
                        }
                    })
                    .ok();
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }

    /// The switch is on, the tab is not muted in Tarn, and no script of Tarn's may run under Lockdown.
    fn may_pip(&self, key: TabKey, view: &engine::WebView) -> bool {
        let url = view.url().map(|u| u.to_string()).unwrap_or_default();
        self.pref(crate::prefs::Pref::AutoPictureInPicture) && !view.is_muted() && self.level_for_tab(key, &url) != shield::ShieldLevel::Lockdown
    }

    fn exit_pip(&self, key: TabKey) {
        if let Some(view) = self.views.get(&key) {
            view.picture_in_picture(PipMode::Exit, |_| {});
        }
    }

    fn press_media(&mut self, control: Control, cx: &mut Context<Self>) {
        let Some((key, state)) = self.mini_player.player.shown(&[]) else { return };
        let Some(view) = self.views.get(&key) else { return };
        view.media_command(control.command(state.playing));
        self.mini_player.player.pressed(control);
        self.ask_media(key, cx);
        cx.notify();
    }

    fn open_media_tab(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.store(key.private).tab(key.id).ok().map(|t| t.space_id);
        let ix = self.model.spaces.iter().position(|s| s.private == key.private && Some(s.space.id) == space);
        if let Some(ix) = ix.filter(|ix| *ix != self.model.space_ix) {
            self.switch_space(ix, window, cx);
        }
        self.switch_to(key, window, cx);
    }
}

pub(crate) fn render(b: &Browser, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let visible: Vec<TabKey> = b.model.active.into_iter().chain(b.mini_player.player.tab().filter(|k| b.in_split(*k))).collect();
    let (key, state) = b.mini_player.player.shown(&visible)?;
    let playing = state.playing;
    let title = SharedString::from(ui::text::bounded_title(&state.title));
    let buttons = Control::ALL.into_iter().enumerate().map(|(ix, control)| {
        let glyph = Icon::named(control.glyph(playing)).unwrap_or(Icon::Info);
        tab_stop(icon_button(("media-control", ix), glyph, true))
            .tooltip(tooltip(control.label(playing), None))
            .on_click(cx.listener(move |b, _, _, cx| b.press_media(control, cx)))
    });
    let open = tab_stop(div().id("media-title"))
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap_1p5()
        .cursor_pointer()
        .child(match b.tab_url(key) {
            Some(url) => crate::sidebar::tab_site_icon(b, key, &url, ICON),
            None => icon(Icon::Music, ICON, Theme::muted()).into_any_element(),
        })
        .child(div().flex_1().min_w_0().text_size(px(TEXT_SMALL)).one_line(title.clone()))
        .tooltip(tooltip(title, None))
        .on_click(cx.listener(move |b, _, window, cx| b.open_media_tab(key, window, cx)));
    let card = surface(Theme::radius_panel()).mx_2().mb_1p5().pl_2().pr_0p5().py_0p5().flex().items_center().gap_1().child(open).children(buttons);
    Some(pop_in(card, ("mini-player", key.id as u64), b.pref(crate::prefs::Pref::ReduceMotion)))
}
