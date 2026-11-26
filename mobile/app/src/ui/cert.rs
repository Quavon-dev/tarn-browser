use browser_core::cert_error::{CertDetails, CertError, CertPage, Problem, Step};
use browser_core::shield_card::date;
use browser_core::toast::Tone;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::{group, list_row, pill, row_action, section_with, secure_button, secure_ghost, secure_surface};
use tarn_nsh_shared::icons as glyphs;

use super::{Screen, Ui, framed, label, small};
use crate::theme::{Paint, Secure, TEXT_HEADING, Theme};

const CARD_WIDTH: f32 = 560.;
const FIELD_WIDTH: f32 = 84.;
const NOT_SECURE: &str = "Not secure — you trusted this site's certificate";

type Shown = (CertPage, bool);

/// The certificate error page, drawn by the shell in place of the web view; nothing of the site loaded.
pub fn page(ui: Ui) -> impl IntoElement {
    let shown = computed(move || {
        ui.read(|s| {
            let key = s.model.active?;
            let page = s.pages.get(&key)?;
            Some((page.cert.clone()?, s.cert_may_continue(key)))
        })
    });
    dynamic(move || {
        let browsing = ui.screen.get() == Screen::Browse && !ui.editing.get();
        shown.get().filter(|_| browsing).map(|shown: Shown| untracked(|| card(ui, shown)))
    })
}

fn card(ui: Ui, (page, may_continue): Shown) -> View {
    let (step, viewer, cert) = (page.shown(may_continue), page.viewer, page.error);
    let revoked = cert.problem == Problem::Revoked;
    let tint: fn() -> Rgba = if revoked { Theme::danger } else { Secure::warning };
    let mac = ui.mac.get_untracked();
    let go = move |step: Step| {
        move || {
            ui.act_on_screen(|s| {
                s.set_cert_step(step);
                Vec::new()
            })
        }
    };
    let actions = if mac { view().flex_row().flex_wrap().items_center().gap_2() } else { view().gap_2() };
    let actions = if step == Step::Confirm {
        actions.child(secure_button("Cancel", go(Step::Details), true)).child(secure_button("Trust Certificate", move || trust(ui), false))
    } else {
        let (details, next) = if step == Step::Details { ("Hide Details", Step::Summary) } else { ("Details", Step::Details) };
        actions
            .child(secure_button("Go Back", move || ui.act_on_screen(|s| s.leave_cert_page()), true))
            .child(secure_button(
                if viewer { "Hide Certificate" } else { "Show Certificate" },
                move || {
                    ui.act_on_screen(|s| {
                        s.toggle_cert_viewer();
                        Vec::new()
                    })
                },
                false,
            ))
            .children(may_continue.then(|| secure_button(details, go(next), false)))
    };
    let way_through = match step {
        Step::Summary => None,
        Step::Details => Some(
            view()
                .gap_2()
                .child(label(
                    "If you know this site and its certificate, such as a router, NAS or server on your own network, you can trust this exact certificate in this profile.",
                ).text_color(Secure::muted))
                .child(secure_ghost(cert.continue_label(), go(Step::Confirm))),
        ),
        Step::Confirm => Some(
            view()
                .gap_1()
                .p_3()
                .rounded(px(Theme::radius_row()))
                .border_1()
                .border_color(tint)
                .child(label(format!("Trust the certificate of {}?", cert.site())).font_weight(FontWeight::BOLD).text_color(tint))
                .child(label(cert.confirm_message()))
                .child(small("Tarn asks for your device passcode, Face ID or fingerprint next.").text_color(Secure::faint)),
        ),
    };
    let card = secure_surface(Theme::radius_card())
        .w_full()
        .max_w(px(CARD_WIDTH))
        .p_5()
        .gap_3()
        .test_id("cert-error")
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(view().text_color(tint).child(glyphs::icon(glyphs::TRIANGLE_ALERT, 22.)))
                .child(text(cert.heading()).flex_1().text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD)),
        )
        .child(label(cert.site()).font_weight(FontWeight::BOLD).number_of_lines(2))
        .child(label(cert.reason(if mac { "device" } else { "phone" })).text_color(Secure::muted))
        .children((!revoked).then(|| small(CertError::WARNING).text_color(Secure::faint)))
        .children(way_through)
        .children(viewer.then(|| viewer_list(ui, &cert)))
        .child(actions);
    view().absolute().inset_0().z_index(2).paint(Theme::backdrop()).child(scroll().flex_1().child(view().items_center().px_4().pt_6().pb_6().child(card)))
}

/// The server's certificates, leaf first.
fn viewer_list(ui: Ui, cert: &CertError) -> View {
    let list = cert.details();
    let title = match list.len() {
        0 => "The site sent no certificate Tarn could read.".to_owned(),
        1 => "Certificate".to_owned(),
        n => format!("Certificate chain ({n})"),
    };
    let field = |name: &'static str, value: String| {
        view()
            .flex_row()
            .gap_3()
            .child(small(name).w(px(FIELD_WIDTH)).flex_none().text_color(Secure::faint))
            .child(small(value).flex_1().text_color(Secure::text))
    };
    let entries = list.into_iter().enumerate().map(move |(ix, d): (usize, CertDetails)| {
        let fingerprint = d.fingerprint.clone();
        let copy = secure_ghost("Copy SHA-256", move || {
            set_clipboard(fingerprint.clone());
            ui.act_on_screen(|s| {
                s.toast_as(Tone::Success, "SHA-256 copied".into());
                Vec::new()
            })
        });
        view()
            .gap_1()
            .pt_3()
            .when(ix > 0, |v| v.border_t_1().border_color(Secure::hairline))
            .child(label(d.subject).font_weight(FontWeight::BOLD).number_of_lines(2))
            .children(d.organization.map(|o| field("Organization", o)))
            .child(field("Issued by", d.issuer))
            .child(field("Valid", format!("{} to {}", date(d.not_before), date(d.not_after))))
            .children((!d.names.is_empty()).then(|| field("Names", d.names.join(", "))))
            .child(field("SHA-256", d.fingerprint))
            .child(view().flex_row().child(copy))
    });
    view().gap_3().p_3().rounded(px(Theme::radius_row())).bg(Secure::control).child(label(title).font_weight(FontWeight::BOLD)).children(entries)
}

/// "Trust Certificate": the device's owner confirms, then this exact certificate is kept for the
/// site in this profile and the page loads.
fn trust(ui: Ui) {
    let mut asked = None;
    ui.act_on_screen(|s| {
        asked = s.begin_cert_trust();
        Vec::new()
    });
    let Some((key, cert)) = asked else { return };
    let reason = cert.error.owner_reason();
    ui.root.run(|| {
        drop(spawn(async move {
            let confirmed = crate::platform::confirm_owner(reason).await;
            ui.act(|s| {
                s.end_cert_trust(key, &cert, confirmed);
                Vec::new()
            });
        }))
    });
}

/// Whether the active page shows as not secure for a certificate the user trusted.
pub fn excepted(ui: Ui) -> impl Fn() -> bool + Copy + 'static {
    move || ui.read(|s| s.active_cert_excepted())
}

/// The site popover's line for a page under a certificate exception, with Remove.
pub fn trusted(ui: Ui) -> impl IntoElement {
    let excepted = excepted(ui);
    dynamic(move || {
        excepted().then(|| {
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .test_id("cert-trusted")
                .child(view().text_color(Secure::warning).child(glyphs::icon(glyphs::TRIANGLE_ALERT, 14.)))
                .child(small(NOT_SECURE).flex_1().number_of_lines(0).text_color(Secure::warning))
                .child(pill("Remove", move || {
                    ui.act(|s| {
                        s.remove_active_cert_exception();
                        Vec::new()
                    })
                }))
        })
    })
}

/// Settings › Permissions: every certificate the profile trusts, each with Remove.
pub fn settings(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let list = ui.read_data(|s| s.cert_exceptions());
        (!list.is_empty()).then(|| {
            let rows: Vec<AnyElement> = list
                .into_iter()
                .enumerate()
                .map(|(ix, e)| {
                    let site = browser_core::text::bounded_title(&if e.port == 443 { e.host.clone() } else { format!("{}:{}", e.host, e.port) });
                    let short = format!("SHA-256 {}…", e.fingerprint.get(..16).unwrap_or(&e.fingerprint).to_ascii_uppercase());
                    let remove = move || {
                        ui.act(|s| {
                            s.remove_cert_exception(&e.host, e.port);
                            Vec::new()
                        })
                    };
                    list_row(ix, site.clone(), short).child(row_action(glyphs::TRASH, format!("Remove the certificate of {site}"), remove)).into_element()
                })
                .collect();
            view()
                .gap_2()
                .child(section_with(
                    "Trusted Certificates",
                    "Certificates you trusted on a certificate error page. These sites show as not secure; one you remove shows the error page again.",
                ))
                .child(framed(group().child(rows)))
        })
    })
}
