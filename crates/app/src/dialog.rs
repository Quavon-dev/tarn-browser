use futures::channel::oneshot;
use gpui::{
    App, AppContext, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, ParentElement,
    PromptButton, PromptHandle, PromptLevel, PromptResponse, Render, RenderablePromptHandle, SharedString, StatefulInteractiveElement, Styled, Window, div,
    hsla, px,
};
use ui::controls::{button, surface};
use ui::theme::{Elevation, TEXT, TEXT_TITLE, Theme};

/// Asks in the window itself, in the app's theme. Only for windows without a web view, which
/// would cover it. An AppKit sheet stays behind when its window closes under it, as Settings and
/// the Library do with their browser window, and later shows as a stray window.
pub(crate) fn ask(level: PromptLevel, message: &str, detail: Option<&str>, answers: &[&str], window: &mut Window, cx: &mut App) -> oneshot::Receiver<usize> {
    cx.set_prompt_builder(build);
    let answer = window.prompt(level, message, detail, answers, cx);
    cx.reset_prompt_builder();
    answer
}

fn build(
    _: PromptLevel,
    message: &str,
    detail: Option<&str>,
    answers: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let dialog = cx.new(|cx| Dialog {
        message: message.to_owned().into(),
        detail: detail.map(|d| d.to_owned().into()),
        answers: answers
            .iter()
            .map(|a| (if matches!(a, PromptButton::Ok(_)) { "OK".into() } else { a.label().clone() }, matches!(a, PromptButton::Cancel(_))))
            .collect(),
        focus: cx.focus_handle(),
    });
    handle.with_view(dialog, window, cx)
}

struct Dialog {
    message: SharedString,
    detail: Option<SharedString>,
    answers: Vec<(SharedString, bool)>,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for Dialog {}

impl Focusable for Dialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Dialog {
    /// GPUI drops the answered prompt without redrawing the window.
    fn answer(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PromptResponse(ix));
        window.refresh();
    }
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cancel = self.answers.iter().position(|(_, cancel)| *cancel);
        let buttons = self.answers.iter().enumerate().rev().map(|(ix, (label, is_cancel))| {
            button(("dialog-answer", ix), label.clone(), ix == 0 && !is_cancel).on_click(cx.listener(move |this, _, window, cx| this.answer(ix, window, cx)))
        });
        let card = surface(Theme::radius_card())
            .w(px(380.))
            .max_w_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_2()
            .shadow(Elevation::Modal.shadow())
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .child(div().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).child(self.message.clone()))
            .children(self.detail.clone().map(|d| div().text_color(Theme::muted()).child(d)))
            .child(div().pt_3().flex().flex_wrap().justify_end().gap_2().children(buttons));
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    this.answer(cancel.unwrap_or(0), window, cx);
                    cx.stop_propagation();
                }
            }))
            // On release: answering on the press hands focus back to the button that asked, whose
            // click GPUI fires on that same Enter's release and so asks again. A focused answer
            // button takes its own Enter.
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                if event.keystroke.key == "enter" && !event.keystroke.modifiers.modified() && this.focus.is_focused(window) {
                    this.answer(0, window, cx);
                    cx.stop_propagation();
                }
            }))
            .occlude()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(hsla(0., 0., 0., 0.3))
            .child(card)
    }
}
