//! The sidebar's mini-player for media playing in a tab that is not on screen.

use std::rc::Rc;

use browser_core::media::{Control, POLL};
use browser_core::page_ops::MediaState;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{AnyElement, Context, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px};
use ui::controls::{icon_button, surface, tab_stop, tooltip};
use ui::icon::{ICON, Icon, icon};
use ui::motion::pop_in;
use ui::text::OneLine;
use ui::theme::{TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};

#[derive(Default)]
pub struct MiniPlayer {
    pub(crate) player: browser_core::media::Player,
    poll: Option<Task<()>>,
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
                    if b.mini_player.player.answer(key, MediaState::parse(&raw)) {
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

    /// Drops `key` from the mini-player once its view is gone.
    pub(crate) fn forget_media(&mut self, key: TabKey) {
        self.mini_player.player.forget(key);
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
        .child(icon(Icon::Music, ICON, Theme::muted()))
        .child(div().flex_1().min_w_0().text_size(px(TEXT_SMALL)).one_line(title.clone()))
        .tooltip(tooltip(title, None))
        .on_click(cx.listener(move |b, _, window, cx| b.open_media_tab(key, window, cx)));
    let card = surface(Theme::radius_panel()).mx_2().mb_1p5().pl_2().pr_0p5().py_0p5().flex().items_center().gap_1().child(open).children(buttons);
    Some(pop_in(card, ("mini-player", key.id as u64), b.pref(crate::prefs::Pref::ReduceMotion)))
}
