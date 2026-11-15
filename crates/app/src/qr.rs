use gpui::{
    AppContext, Bounds, Context, Div, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, Subscription,
    Window, actions, canvas, div, fill, point, px, size,
};
use ui::controls::surface;
use ui::motion::fade_in;
use ui::theme::{TEXT_SMALL, Theme};
use url::Url;

use crate::browser::Browser;
use crate::peek::popup_options;
use crate::prefs::Pref;

actions!(tarn, [ShareLink, ShowQrCode]);

const WIDTH: f32 = 240.;
const HEIGHT: f32 = 270.;
const CODE: f32 = 204.;
/// CoreImage leaves one light module around the code; scanners want four.
const QUIET: usize = 3;
const GAP: f32 = 8.;

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &ShareLink, _, cx| b.share_link(cx))).on_action(cx.listener(|b, _: &ShowQrCode, w, cx| b.show_qr_code(w, cx)))
}

impl Browser {
    fn active_url(&self) -> Option<String> {
        self.model.active.and_then(|key| self.tab_url(key)).filter(|u| !u.is_empty())
    }

    /// The system share menu for the active page, below the address field.
    fn share_link(&mut self, cx: &mut Context<Self>) {
        let (Some(url), Some(host)) = (self.active_url(), self.host.clone()) else { return };
        let field = self.address.field.get();
        let (x, y, w, h) = if field.size.width > px(0.) {
            (f32::from(field.origin.x), f32::from(field.origin.y), f32::from(field.size.width), f32::from(field.size.height))
        } else {
            let slot = self.address.slot.get();
            (f32::from(slot.origin.x), 0., f32::from(slot.size.width).max(1.), ui::theme::TOOLBAR_HEIGHT)
        };
        cx.spawn(async move |_, _| host.share(&url, x.into(), y.into(), w.into(), h.into())).detach();
    }

    /// A card with the active page's QR code below the address field; Esc or a click elsewhere closes it.
    fn show_qr_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self.active_url() else { return };
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
        let x = (center - px(WIDTH / 2.)).clamp(px(0.), (frame.size.width - px(WIDTH)).max(px(0.)));
        let bounds = Bounds::new(point(frame.origin.x + x, frame.origin.y + top), size(px(WIDTH), px(HEIGHT)));
        let display = window.display(cx).map(|d| d.id());
        let reduce = self.pref(Pref::ReduceMotion);
        cx.defer(move |cx| {
            cx.open_window(popup_options(bounds, display, true), |window, cx| {
                if let Some(host) = engine::Host::from_window(window) {
                    host.strip_frame();
                    host.round_corners(0., 0., 0., 0., f64::from(ui::theme::Theme::radius_panel()));
                }
                cx.new(|cx| QrWindow::new(code, label.into(), reduce, window, cx))
            })
            .ok();
        });
    }
}

pub struct QrWindow {
    code: Vec<Vec<bool>>,
    label: SharedString,
    reduce: bool,
    focus: FocusHandle,
    _activation: Subscription,
}

impl QrWindow {
    fn new(code: Vec<Vec<bool>>, label: SharedString, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let activation = cx.observe_window_activation(window, |_, window, _| {
            if !window.is_window_active() {
                window.remove_window();
            }
        });
        QrWindow { code, label, reduce, focus, _activation: activation }
    }
}

impl Render for QrWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
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
        let card = surface(Theme::radius_panel())
            .track_focus(&self.focus)
            .on_key_down(|e: &KeyDownEvent, window, _| {
                if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(div().size(px(CODE)).rounded(px(6.)).bg(gpui::white()).flex().items_center().justify_center().child(modules))
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
        fade_in(card, "qr-code", self.reduce)
    }
}
