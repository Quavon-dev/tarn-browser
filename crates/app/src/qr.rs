use gpui::{
    Bounds, Context, Div, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, Subscription, Window,
    WindowHandle, actions, canvas, div, fill, point, px, size,
};
use ui::controls::{POPOVER_INSET, open_popup, popover};
use ui::motion::popover_motion;
use ui::theme::{TEXT_SMALL, Theme};
use url::Url;

use crate::browser::Browser;
use crate::peek::remove_after_exit;
use crate::prefs::Pref;

actions!(tarn, [ShareLink, ShowQrCode]);

const WIDTH: f32 = 240.;
const HEIGHT: f32 = 270.;
const CODE: f32 = 204.;
/// CoreImage leaves one light module around the code; scanners want four.
const QUIET: usize = 3;
const GAP: f32 = 8.;

/// Only a web address means something on another device.
pub(crate) fn web_link(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ShareLink, w, cx| b.toggle_share_menu(w, cx))).on_action(cx.listener(|b, _: &ShowQrCode, w, cx| b.show_qr_code(w, cx)))
}

impl Browser {
    fn active_url(&self) -> Option<String> {
        self.model.active.and_then(|key| self.tab_url(key)).filter(|u| web_link(u))
    }

    /// A card with the active page's QR code below the address field; Esc or a click elsewhere closes it.
    pub(crate) fn show_qr_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.qr.take()
            && handle.update(cx, |_, window, _| window.remove_window()).is_ok()
        {
            return;
        }
        let Some(url) = self.active_url() else {
            return self.show_toast("Only web pages have a QR code", None, cx);
        };
        let Some(code) = engine::share::qr_code(&url) else {
            return self.show_toast("No QR code for this link", None, cx);
        };
        let label = Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|h| crate::toolbar::host_tail(h.trim_start_matches("www."), 40)))
            .unwrap_or_else(|| ui::text::bounded_title(&url));
        let (frame, field) = (window.bounds(), self.address.field.get());
        let center = if field.size.width > px(0.) { field.center().x } else { frame.size.width / 2. };
        let top = if field.size.height > px(0.) { field.bottom() } else { px(ui::theme::TOOLBAR_HEIGHT) } + px(GAP);
        let x = (center - px(WIDTH / 2.)).clamp(px(0.), (frame.size.width - px(WIDTH)).max(px(0.))) - px(POPOVER_INSET);
        let bounds = Bounds::new(point(frame.origin.x + x, frame.origin.y + top), size(px(WIDTH + 2. * POPOVER_INSET), px(HEIGHT + POPOVER_INSET)));
        let display = window.display(cx).map(|d| d.id());
        let reduce = self.pref(Pref::ReduceMotion);
        let main = window.window_handle().downcast::<Browser>().or(self.peek.main);
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            let inset = [0., POPOVER_INSET, POPOVER_INSET, POPOVER_INSET];
            let (handle, _) =
                open_popup(cx, bounds, display, true, inset, Theme::radius_panel(), |window, cx| QrWindow::new(code, label.into(), reduce, main, window, cx));
            browser.update(cx, |b, _| b.qr = handle).ok();
        });
    }
}

pub struct QrWindow {
    code: Vec<Vec<bool>>,
    label: SharedString,
    reduce: bool,
    main: Option<WindowHandle<Browser>>,
    focus: FocusHandle,
    closing: bool,
    _activation: Subscription,
}

impl QrWindow {
    fn new(code: Vec<Vec<bool>>, label: SharedString, reduce: bool, main: Option<WindowHandle<Browser>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(false, window, cx);
            }
        });
        QrWindow { code, label, reduce, main, focus, closing: false, _activation: activation }
    }

    /// Esc and the menu command hand the focus back to the page the card was opened from.
    fn close(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.closing, true) {
            return;
        }
        remove_after_exit(self.reduce, window, cx);
        if let Some(main) = self.main.filter(|_| refocus) {
            main.update(cx, |_, window, _| window.activate_window()).ok();
        }
        cx.notify();
    }
}

impl Render for QrWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let code = self.code.clone();
        let module = (CODE / (code.len() + 2 * QUIET) as f32).floor();
        let side = module * code.len() as f32;
        let modules = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                for (y, row) in code.iter().enumerate() {
                    for (x, _) in row.iter().enumerate().filter(|(_, dark)| **dark) {
                        let at = point(bounds.origin.x + px(module * x as f32), bounds.origin.y + px(module * y as f32));
                        window.paint_quad(fill(Bounds::new(at, size(px(module), px(module))), gpui::black()));
                    }
                }
            },
        )
        .size(px(side));
        let card = popover(Theme::radius_panel(), 0.)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(div().size(px(CODE)).rounded(px(Theme::radius_small())).bg(gpui::white()).flex().items_center().justify_center().child(modules))
            .child(
                div()
                    .max_w(px(CODE))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(TEXT_SMALL))
                    .text_color(Theme::muted())
                    .child(self.label.clone()),
            );
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ShowQrCode, window, cx| this.close(true, window, cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    this.close(true, window, cx);
                }
            }))
            .size_full()
            .child(popover_motion(div().relative().size_full().child(card), "qr-code", self.closing, self.reduce))
    }
}
