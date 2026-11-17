use browser_core::site::{warning_heading, warning_reason, warning_source};
use nsh_core::prelude::*;
use shield::Category;
use tarn_nsh_shared::controls::{secure_button, secure_surface};

use super::{Ui, act};
use crate::browser::Browser;
use crate::icons;
use crate::theme::{Secure, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

/// The warning page, drawn by the shell in place of the web view; nothing of the site is loaded.
pub(super) fn page(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.blocked.is_some()) && ui.panel.get() == super::Panel::None && !ui.editing.get(),
        move || {
            let Some(blocked) = ui.snap.with(|s| s.blocked.clone()) else { return view().into_element() };
            let origin = url::Url::parse(&blocked.url).map(|u| browser_core::text::bounded_origin(&u.origin().ascii_serialization())).unwrap_or_default();
            let malware = blocked.category == Category::Malware;
            let color = if malware { Secure::danger() } else { Secure::warning() };
            let mut buttons =
                view().flex_row().items_center().pt_2().child(secure_button("Go Back", move || act(ui, Browser::leave_blocked), true)).child(view().flex_1());
            if malware {
                buttons = buttons.child(secure_button("Visit Anyway", move || act(ui, Browser::visit_anyway), false));
            }
            let card = secure_surface(Theme::radius_card())
                .w(px(520.))
                .p_6()
                .flex_col()
                .gap_3()
                .text_size(px(TEXT))
                .shadow(Theme::elevation(18., 48.))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(view().text_color(color).child(icons::icon(icons::SHIELD, 22.)))
                        .child(text(warning_heading(blocked.category)).text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD)),
                )
                .child(text(origin).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(text(warning_reason(blocked.category, "computer")).text_color(Secure::muted()))
                .child(text(warning_source(blocked.category)).text_size(px(TEXT_SMALL)).text_color(Secure::faint()))
                .child(buttons);
            view().flex_col().flex_1().items_center().pt(px(96.)).px_4().child(card).into_element()
        },
    )
    .into_element()
}
