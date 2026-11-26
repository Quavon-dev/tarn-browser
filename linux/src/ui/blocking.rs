use browser_core::cert_error::{CertDetails, CertError, CertPage, Problem};
use browser_core::shield_card::date;
use browser_core::site::{warning_heading, warning_reason, warning_source};
use nsh_core::prelude::*;
use shield::Category;
use tarn_nsh_shared::controls::{secure_button, secure_ghost, secure_surface};

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

/// The certificate error page, drawn by the shell in place of the web view; nothing of the site
/// loaded. Linux offers no way through (`docs/nsh-plan.md`, "Certificate exceptions").
pub(super) fn cert_page(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.cert.is_some()) && ui.panel.get() == super::Panel::None && !ui.editing.get(),
        move || {
            let Some(CertPage { error: cert, viewer, .. }) = ui.snap.with(|s| s.cert.clone()) else { return view().into_element() };
            let color = if cert.problem == Problem::Revoked { Secure::danger() } else { Secure::warning() };
            let actions = view()
                .flex_row()
                .items_center()
                .gap_2()
                .pt_2()
                .child(secure_button("Go Back", move || act(ui, Browser::leave_blocked), true))
                .child(view().flex_1())
                .child(secure_button(if viewer { "Hide Certificate" } else { "Show Certificate" }, move || act(ui, Browser::toggle_cert_viewer), false));
            let card = secure_surface(Theme::radius_card())
                .w(px(560.))
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
                        .child(view().text_color(color).child(icons::icon(icons::TRIANGLE_ALERT, 22.)))
                        .child(text(cert.heading()).text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD)),
                )
                .child(text(cert.site()).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(text(cert.reason("computer")).text_color(Secure::muted()))
                .children((cert.problem != Problem::Revoked).then(|| text(CertError::WARNING).text_size(px(TEXT_SMALL)).text_color(Secure::faint())))
                .children(viewer.then(|| cert_viewer(&cert)))
                .child(actions);
            scroll().flex_1().child(view().flex_col().items_center().pt(px(72.)).pb_6().px_4().child(card)).into_element()
        },
    )
    .into_element()
}

/// The server's certificates, leaf first.
fn cert_viewer(cert: &CertError) -> View {
    let row = |name: &'static str, value: String| {
        view().flex_row().gap_3().child(text(name).w(px(88.)).flex_shrink_0().text_color(Secure::faint())).child(text(value).flex_1().min_w(px(0.)))
    };
    let list = cert.details();
    let heading = match list.len() {
        0 => "The site sent no certificate Tarn could read.".to_owned(),
        1 => "Certificate".to_owned(),
        n => format!("Certificate chain ({n})"),
    };
    let entries = list.into_iter().enumerate().map(|(ix, d): (usize, CertDetails)| {
        let fingerprint = d.fingerprint.clone();
        view()
            .flex_col()
            .gap_1()
            .pt_3()
            .when(ix > 0, |v| v.border_t_1().border_color(Secure::hairline()))
            .text_size(px(TEXT_SMALL))
            .child(text(d.subject).text_size(px(TEXT)).font_weight(FontWeight::SEMIBOLD).truncate())
            .children(d.organization.map(|o| row("Organization", o)))
            .child(row("Issued by", d.issuer))
            .child(row("Valid", format!("{} to {}", date(d.not_before), date(d.not_after))))
            .children((!d.names.is_empty()).then(|| row("Names", d.names.join(", "))))
            .child(row("SHA-256", d.fingerprint))
            .child(view().flex_row().pt_1().child(secure_ghost("Copy SHA-256", move || set_clipboard(fingerprint.clone()))))
    });
    view()
        .flex_col()
        .gap_3()
        .p_3()
        .rounded(px(Theme::radius_card() / 2.))
        .bg(Secure::control())
        .child(text(heading).font_weight(FontWeight::SEMIBOLD))
        .children(entries)
}
