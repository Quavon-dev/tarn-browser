use nsh_core::JsonValue;
use nsh_core::prelude::*;
use serde_json::json;

use super::{BROWSERS, PALETTE_DIM, Ui, act, act_quietly};
use crate::controls::{icon_button, small};
use crate::icons;
use crate::snapshot::{PeekItem, ViewItem};
use crate::theme::{SYSTEM_FONT, Secure, Theme};
use crate::webview;

const MAX_WIDTH: f32 = 1100.;
const MIN_WIDTH: f32 = 320.;
const MARGIN: f32 = 80.;
const HEADER: f32 = 44.;
const PAD: f32 = 8.;

pub(super) fn close(ui: Ui) {
    act(ui, |b| b.close_link_peek());
}

pub(super) fn layer(ui: Ui) -> AnyElement {
    let shown = computed(move || ui.snap.with(|s| s.link_peek.is_some()));
    dynamic(move || shown.get().then(|| card(ui)))
}

fn card(ui: Ui) -> View {
    let peek = computed(move || ui.snap.with(|s| s.link_peek.clone()));
    let width = move || ui.area.with(|(w, _)| (w - 2. * MARGIN).clamp(MIN_WIDTH, MAX_WIDTH));
    let header = view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(HEADER))
        .flex_none()
        .pl_3()
        .pr_2()
        .child(dynamic(move || peek.with(|p| p.as_ref().map(origin))))
        .child(small(move || peek.with(|p| p.as_ref().map(|p| p.title.clone()).unwrap_or_default())).text_color(Theme::muted()).flex_1().truncate())
        .child(tarn_nsh_shared::controls::pill("Open in Tab", move || act(ui, |b| b.link_peek_to_tab())))
        .child(icon_button(icons::CLOSE, "Close (Esc)", move || close(ui)));
    let dim = nsh_base::button().accessibility_label("Close").on_press(move || close(ui)).absolute().inset_0().bg(rgba(PALETTE_DIM));
    let card = view()
        .absolute()
        .top(px(PAD))
        .bottom(px(MARGIN / 2.))
        .left(move || px(ui.area.with(|(w, _)| (w - width()) / 2.)))
        .w(move || px(width()))
        .flex_col()
        .rounded(px(Theme::radius_panel()))
        .bg(Theme::panel)
        .border_1()
        .border_color(Theme::surface_edge)
        .shadow(Theme::elevation(18., 48.))
        .child(header)
        .child(view().flex_1().mx(px(PAD)).mb(px(PAD)).rounded(px(Theme::radius_page())).overflow_hidden().bg(Theme::page).child(web_view(ui, peek)));
    view().absolute().inset_0().child(dim).child(card)
}

fn origin(peek: &PeekItem) -> View {
    let text = if peek.insecure { format!("Not Secure — {}", peek.host) } else { peek.host.clone() };
    view()
        .flex_none()
        .px_2()
        .py(px(2.))
        .rounded(px(Theme::radius_small()))
        .bg(Secure::chip())
        .child(small(text).font_family(SYSTEM_FONT).font_weight(FontWeight::MEDIUM).text_color(if peek.insecure { Secure::warning() } else { Secure::text() }))
}

fn web_view(ui: Ui, peek: Memo<Option<PeekItem>>) -> impl IntoElement {
    let web = element_ref();
    let prop = move |f: fn(&ViewItem) -> JsonValue| move || peek.with(|p| p.as_ref().map_or(JsonValue::Null, |p| f(&p.view)));
    native(webview::NAME)
        .prop("tab", prop(|v| JsonValue::from(format!("{}-{}", v.key.private, v.key.id))))
        .prop("profile", prop(|v| JsonValue::from(v.profile.clone())))
        .prop("level", prop(|v| JsonValue::from(v.level.clone())))
        .prop("levels", prop(|v| v.levels.clone()))
        .prop("lists", prop(|v| JsonValue::from(v.lists.clone())))
        .prop("styles", prop(|v| v.styles.clone()))
        .prop("themeStyles", prop(|v| v.theme_styles.clone()))
        .prop("zaps", prop(|v| v.zaps.clone()))
        .prop("sortDownloads", prop(|v| JsonValue::from(v.sort_downloads)))
        .prop("load", prop(|v| v.load.as_ref().map_or(JsonValue::Null, |(u, s)| json!({ "url": u, "seq": s, "link": v.link }))))
        .prop("active", JsonValue::from(true))
        .on_native_event(move |event: &NativeEvent| {
            if matches!(&*event.name, "snapshot" | "iconData") {
                return;
            }
            act_quietly(ui, |b| b.on_peek_event(&event.name, &event.data));
            let commands = BROWSERS.with_borrow_mut(|w| w.get_mut(&ui.window).map(|b| std::mem::take(&mut b.peek_outbox))).unwrap_or_default();
            for (name, args) in commands {
                web.command(name, args);
            }
        })
        .element_ref(&web)
        .size_full()
}
