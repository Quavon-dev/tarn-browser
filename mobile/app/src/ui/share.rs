use std::time::Duration;

use nsh_base::button;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::group;
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::motion::{SCRIM, fade_to, pop_out, slide_up};

use super::{Screen, Ui, caption, label};
use crate::theme::{ICON, SHEET_WIDTH, TEXT_SMALL, TOUCH_TARGET, Theme};

const TARGET: f32 = 52.;
const TILE: f32 = 40.;
const TRAVEL: f32 = 420.;
const COPIED: Duration = Duration::from_millis(1200);

fn glyph(source: &'static str, size: f32) -> View {
    view().items_center().justify_center().child(glyphs::icon(source, size))
}

fn round_target(name: &'static str, source: impl Fn() -> &'static str + 'static, primary: bool, press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .items_center()
        .gap_1()
        .w(px(TARGET + 16.))
        .child(
            button()
                .accessibility_label(name)
                .on_press(press)
                .w(px(TARGET))
                .h(px(TARGET))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(move || if primary { Theme::accent() } else { Theme::control() })
                .text_color(move || if primary { Theme::on_fill(Theme::accent()) } else { Theme::text() })
                .active(|s| s.opacity(0.75))
                .child(dynamic(move || glyph(source(), ICON).bind_motion(&tarn_nsh_shared::motion::fade_in()))),
        )
        .child(text(name).text_size(px(TEXT_SMALL)).text_color(Theme::muted).number_of_lines(1))
}

fn action_row(name: &'static str, source: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .accessibility_label(name)
        .on_press(press)
        .flex_row()
        .items_center()
        .min_h(px(TOUCH_TARGET))
        .px_4()
        .border_b_1()
        .border_color(Theme::separator)
        .active(|s| s.bg(Theme::pressed()))
        .child(label(name).flex_1())
        .child(view().text_color(Theme::muted).child(glyphs::icon(source, ICON)))
}

pub fn share_sheet(ui: Ui) -> impl IntoElement {
    when(move || ui.screen.get() == Screen::Share && !ui.mac.get(), move || sheet(ui))
}

fn sheet(ui: Ui) -> impl IntoElement {
    let scrim = fade_to(SCRIM);
    let panel = slide_up(TRAVEL);
    let close =
        move |next: Screen| {
            pop_out(scrim, SCRIM, move || ui.show(next));
            drop(panel.play(
                MotionSpec::new(KeyframeTiming::new(tarn_nsh_shared::motion::EXIT.as_secs_f32() * 1000.)).values(MotionChannel::TranslateY, [0., TRAVEL]),
            ));
        };
    let copied = signal(false);
    let page = ui.read(|s| (s.share_url(), s.active_page().map(|p| p.title.clone()).unwrap_or_default()));
    let (url, title) = page;
    let host = url.as_deref().map(browser_core::command::display_host).unwrap_or_default();
    let site = url.as_deref().and_then(|u| url::Url::parse(u).ok()).and_then(|u| browser_core::command::page_host(&u)).unwrap_or_else(|| host.clone());
    let icon = url.as_deref().and_then(|u| ui.read(|s| s.site_icon(u)));
    let shown_title = if title.trim().is_empty() { host.clone() } else { title };
    let link = url.clone();
    let shared = url.clone();
    let copy = move || {
        if let Some(url) = link.clone() {
            set_clipboard(url);
            copied.set(true);
            drop(set_timeout(COPIED, move || copied.set(false)));
        }
    };
    let actions = view()
        .child(action_row("Add to Reading List", glyphs::BOOKMARK, move || {
            ui.act(|s| {
                s.save_for_later(browser_core::now());
                Vec::new()
            });
            close(Screen::Browse);
        }))
        .child(dynamic(move || {
            ui.read(|s| s.offers_reader()).then(|| {
                action_row("Open Reader", glyphs::BOOK_OPEN, move || {
                    ui.screen.set(Screen::Browse);
                    ui.act(|s| s.open_reader());
                })
            })
        }))
        .child(action_row("Find on Page", glyphs::SEARCH, move || close(Screen::Find)));
    view()
        .absolute()
        .inset_0()
        .z_index(3)
        .justify_end()
        .items_center()
        .child(button().accessibility_label("Close Share").absolute().inset_0().bg(rgb(0x000000)).bind_motion(&scrim).on_press(move || close(Screen::Browse)))
        .child(
            view()
                .w_full()
                .max_w(px(SHEET_WIDTH))
                .rounded_tl(px(Theme::radius_card()))
                .rounded_tr(px(Theme::radius_card()))
                .bg(Theme::surface)
                .border_1()
                .border_color(Theme::surface_edge)
                .shadow(Theme::elevation(-4., 24.))
                .pb_safe_area_plus(px(12.))
                .bind_motion(&panel)
                .child(view().items_center().pt_2().child(view().w(px(36.)).h(px(5.)).rounded_full().bg(Theme::faint)))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .pl_4()
                        .pr_2()
                        .pt_3()
                        .child(tarn_nsh_shared::controls::site_tile(icon, &site, TILE))
                        .child(
                            view()
                                .flex_1()
                                .child(label(shown_title).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                                .child(caption(host).number_of_lines(1)),
                        )
                        .child(tarn_nsh_shared::controls::icon_button(glyph(glyphs::CLOSE, ICON), "Close", move || close(Screen::Browse))),
                )
                .child(
                    view()
                        .flex_row()
                        .justify_center()
                        .gap_2()
                        .pt_4()
                        .pb_3()
                        .child(round_target("Copy", move || if copied.get() { glyphs::CHECK } else { glyphs::LINK }, true, copy))
                        .child(round_target("QR Code", || glyphs::QR_CODE, false, move || ui.show(Screen::Qr)))
                        .child(round_target(
                            "More…",
                            || glyphs::ELLIPSIS,
                            false,
                            move || {
                                if let Some(url) = shared.clone() {
                                    super::panels::share(ui, url);
                                }
                            },
                        )),
                )
                .child(view().px_3().child(group().overflow_hidden().child(actions))),
        )
}
