use browser_core::shield_card::{motto, thousands};
use nsh_core::prelude::*;
use shield::ShieldLevel;
use tarn_nsh_shared::controls::{caps_label, empty_state, glyph_row, glyph_switch, group, icon_button, icon_pill, label, segmented, site_tile};
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::popover::{self as pop, Anchor};
use tarn_nsh_shared::sidebar::{MENU_WIDTH, menu_check, menu_item};
use tarn_nsh_shared::toolbar::small;

use super::super::{Screen, Ui, panels, sheets};
use crate::theme::{Secure, Theme};

const SITE_HEADER: f32 = 60.;
const FOOTER: f32 = 40.;
const ZOOM_STEP: f64 = 0.1;
const QR_SIZE: f32 = 204.;

#[derive(Clone, Copy)]
pub struct Anchors {
    pub shield: ElementRef,
    pub share: ElementRef,
    pub downloads: ElementRef,
    pub toggle: ElementRef,
    pub new_space: ElementRef,
}

impl Default for Anchors {
    fn default() -> Self {
        Self { shield: element_ref(), share: element_ref(), downloads: element_ref(), toggle: element_ref(), new_space: element_ref() }
    }
}

pub(super) fn owns(screen: Screen) -> bool {
    matches!(screen, Screen::Shield | Screen::Downloads | Screen::Share | Screen::Qr | Screen::SpaceNew | Screen::SpaceEdit(_))
}

pub(in crate::ui) fn start_pills(ui: Ui) -> impl IntoElement {
    let open = signal(None::<bool>);
    let menu = move |space: bool| {
        dynamic(move || {
            (open.get() == Some(space)).then(|| {
                let items: Vec<AnyElement> = untracked(|| {
                    if space {
                        let private = ui.read(|s| s.model.current_space().is_none_or(|c| c.private));
                        vec![
                            menu_item("Edit Space…", !private, move || {
                                open.set(None);
                                ui.show(Screen::SpaceEdit(ui.read(|s| s.model.space_ix)));
                            })
                            .into_element(),
                            menu_item("New Space…", ui.read(|s| s.can_add_space()), move || {
                                open.set(None);
                                ui.show(Screen::SpaceNew);
                            })
                            .into_element(),
                        ]
                    } else {
                        let current = ui.read_data(|s| s.default_level());
                        ShieldLevel::ALL
                            .into_iter()
                            .map(|level| {
                                menu_check(motto(level).0, level == current, move || {
                                    open.set(None);
                                    ui.act(|s| {
                                        s.set_default_level(level);
                                        Vec::new()
                                    });
                                })
                                .into_element()
                            })
                            .collect()
                    }
                });
                let point = element_ref();
                view().absolute().right(px(0.)).bottom(px(0.)).w(px(0.)).h(px(0.)).element_ref(&point).child(pop::popover(
                    Anchor::Below(point, Align::End),
                    MENU_WIDTH,
                    move || open.set(None),
                    view().flex_col().p_1().children(items),
                ))
            })
        })
    };
    let space = computed(move || {
        ui.read(|s| s.model.current_space().map(|c| (c.space.name.clone(), if c.private { glyphs::HAND } else { glyphs::space_icon(&c.space.icon) })))
    });
    let level = computed(move || ui.read_data(|s| s.default_level()));
    view()
        .flex_row()
        .justify_end()
        .gap_2()
        .pt_2()
        .child(
            view()
                .relative()
                .child(dynamic(move || {
                    let (name, glyph) = space.get().unwrap_or_default();
                    icon_pill(glyph, name, move || open.set(Some(true)))
                }))
                .child(menu(true)),
        )
        .child(
            view()
                .relative()
                .child(dynamic(move || {
                    icon_pill(glyphs::SHIELD, motto(level.get()).0, move || open.set(Some(false)))
                        .accessibility_label(format!("Shield: {}", motto(level.get()).0))
                }))
                .child(menu(false)),
        )
}

pub(super) fn popovers(ui: Ui) -> impl IntoElement {
    let sidebar = computed(move || ui.read_data(|s| s.sidebar_open));
    dynamic(move || {
        let screen = ui.screen.get();
        let open = matches!(screen, Screen::Downloads | Screen::SpaceNew | Screen::SpaceEdit(_)).then(|| sidebar.get());
        let sidebar_shown = open == Some(true);
        untracked(|| {
            let mut footer = None;
            let (anchor, width, body) = match screen {
                Screen::Shield => (Anchor::Below(ui.popovers.shield, Align::Center), pop::SITE_WIDTH, site_popover(ui).into_element()),
                Screen::Share => (Anchor::Below(ui.popovers.share, Align::End), pop::SHARE_WIDTH, share_menu(ui).into_element()),
                Screen::Qr => (Anchor::Below(ui.popovers.share, Align::End), pop::QR_WIDTH, qr(ui).into_element()),
                Screen::Downloads => (downloads_anchor(ui, open == Some(true)), pop::DOWNLOADS_WIDTH, downloads(ui).into_element()),
                Screen::SpaceNew | Screen::SpaceEdit(_) => {
                    let editing = if let Screen::SpaceEdit(ix) = screen { Some(ix) } else { None };
                    let (body, foot) = sheets::space_card_parts(ui, editing);
                    footer = Some(foot.px_3().pb_3().into_element());
                    let card = view().p_3().child(body);
                    let anchor =
                        if sidebar_shown { Anchor::Above(ui.popovers.new_space, Align::Start) } else { Anchor::Below(ui.popovers.toggle, Align::Start) };
                    (anchor, pop::SPACE_WIDTH, card.into_element())
                }
                _ => return None,
            };
            Some(pop::fitted(anchor, width, move || ui.show(Screen::Browse), body, footer))
        })
    })
}

fn downloads_anchor(ui: Ui, sidebar: bool) -> Anchor {
    if sidebar { Anchor::Above(ui.popovers.downloads, Align::Start) } else { Anchor::Below(ui.popovers.toggle, Align::Start) }
}

fn site_header(ui: Ui, host: Option<String>, secure: bool) -> View {
    let url = ui.read(|s| s.active_url());
    let (glyph, tint, connection): (_, fn() -> Rgba, _) =
        if secure { (glyphs::LOCK, Secure::success, "Connection is secure") } else { (glyphs::TRIANGLE_ALERT, Secure::warning, "Not secure") };
    let name = host.clone().unwrap_or_else(|| "This page".to_owned());
    view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .h(px(SITE_HEADER))
        .px_3()
        .child(site_tile(ui.read(|s| s.site_icon(&url)), host.as_deref().unwrap_or_default(), 28.))
        .child(view().flex_col().flex_1().min_w(px(0.)).child(label(name).font_weight(FontWeight::BOLD).number_of_lines(1)).children(
            host.map(|_| view().flex_row().items_center().gap_1().text_color(tint).child(glyphs::icon(glyph, 11.)).child(small(connection).text_color(tint))),
        ))
}

fn site_popover(ui: Ui) -> impl IntoElement {
    let excepted = super::super::cert::excepted(ui);
    let site = computed(move || {
        let (host, https) = ui.read(|s| (s.active_host(), s.active_url().starts_with("https://")));
        (host, https && !excepted())
    });
    dynamic(move || {
        let (host, secure) = site.get();
        untracked(|| site_settings(ui, host, secure))
    })
}

fn site_settings(ui: Ui, host: Option<String>, secure: bool) -> impl IntoElement {
    let card = view().flex_col().child(site_header(ui, host.clone(), secure));
    if host.is_none() {
        return card.child(small("Open a site to choose its level.").px_3().pb_3());
    }
    let more = signal(false);
    let own = move || {
        ui.read(|s| {
            let (private, id) = s.model.profile()?;
            let host = s.active_host()?;
            s.core.has_override(private, id, &host).then(|| s.site_level()).flatten()
        })
    };
    let set = move |level: Option<ShieldLevel>| {
        ui.act(|s| {
            s.set_site_level(level);
            Vec::new()
        })
    };
    let choices = [None, Some(ShieldLevel::Standard), Some(ShieldLevel::Strict), Some(ShieldLevel::Lockdown)]
        .into_iter()
        .map(|level| {
            let name = level.map_or("Default", |l| motto(l).0).to_owned();
            (name, Box::new(move || own() == level) as Box<dyn Fn() -> bool>, Box::new(move || set(level)) as Box<dyn Fn()>)
        })
        .collect();
    let meaning = move || match own() {
        Some(level) => motto(level).1.to_owned(),
        None => {
            let (name, words) = motto(ui.read_data(|s| s.default_level()));
            format!("Follows the profile: {name} — {}", words.to_lowercase())
        }
    };
    let summary = group()
        .mx_3()
        .px_3()
        .py_2()
        .child(
            text(move || thousands(ui.read(|s| s.site_shield_total()).max(0) as u64)).text_size(px(22.)).font_weight(FontWeight::BOLD).text_color(Theme::text),
        )
        .child(small("stopped on this site in this profile"));
    let zoom = move |delta: f64| ui.act(|s| s.zoom_by(delta));
    let list = group()
        .mx_3()
        .overflow_hidden()
        .child(
            glyph_switch(
                glyphs::MOON,
                "Dark mode for this site",
                move || ui.read(|s| s.site_dark()),
                move || {
                    ui.act(|s| {
                        s.set_site_dark(!s.site_dark());
                        Vec::new()
                    })
                },
            )
            .border_b_1()
            .border_color(Theme::separator),
        )
        .child(
            glyph_row(glyphs::ZOOM_IN, "Zoom").child(
                view()
                    .flex_row()
                    .items_center()
                    .child(icon_button(glyphs::icon(glyphs::CHEVRON_LEFT, 14.), "Zoom Out", move || zoom(-ZOOM_STEP)))
                    .child(small(move || format!("{:.0}%", ui.read(|s| s.active_page().map_or(1.0, |p| p.zoom)) * 100.)).w(px(44.)).text_center())
                    .child(icon_button(glyphs::icon(glyphs::CHEVRON_RIGHT, 14.), "Zoom In", move || zoom(ZOOM_STEP)))
                    .child(icon_button(glyphs::icon(glyphs::RELOAD, 14.), "Actual Size", move || zoom(0.0))),
            ),
        );
    let toggle = nsh_base::button()
        .accessibility_label(move || SharedString::from(if more.get() { "Fewer site settings" } else { "More site settings" }))
        .on_press(move || more.update(|m| *m = !*m))
        .flex_row()
        .items_center()
        .gap_1()
        .text_color(Theme::accent)
        .hover(|s| s.opacity(0.8))
        .child(small(move || if more.get() { "Fewer site settings" } else { "More site settings" }).text_color(Theme::accent))
        .child(dynamic(move || glyphs::icon(if more.get() { glyphs::CHEVRON_DOWN } else { glyphs::CHEVRON_RIGHT }, 12.)));
    card.gap_3()
        .child(view().px_3().child(super::super::cert::trusted(ui)))
        .child(summary)
        .child(
            view()
                .flex_col()
                .gap(px(6.))
                .px_3()
                .child(caps_label("Shield for this site"))
                .child(segmented(choices).test_id("site-levels"))
                .child(small(meaning).px_1()),
        )
        .child(list)
        .child(when(move || more.get(), move || view().px_3().child(sheets::site(ui))))
        .child(view().flex_row().items_center().h(px(FOOTER)).px_3().border_t_1().border_color(Theme::separator).child(toggle))
}

fn copy(ui: Ui, clean: bool) {
    ui.copy_link(None, clean);
    ui.show(Screen::Browse);
}

fn share_menu(ui: Ui) -> impl IntoElement {
    let page = computed(move || {
        ui.read(|s| (s.share_url().unwrap_or_default(), s.active_page().map(|p| browser_core::text::bounded_title(&p.title)).unwrap_or_default()))
    });
    dynamic(move || {
        let (url, title) = page.get();
        untracked(|| share_body(ui, url, title))
    })
}

fn share_body(ui: Ui, url: String, title: String) -> impl IntoElement {
    let host = browser_core::command::display_host(&url);
    let shown = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).trim_end_matches('/').to_owned();
    let tile = site_tile(ui.read(|s| s.site_icon(&url)), &host, 28.);
    let row = move |glyph: &'static str, name: &'static str, press: Box<dyn Fn()>| pop::row(pop::ROW, Some(glyph), name, None, press).into_element();
    let more = url.clone();
    view()
        .flex_col()
        .child(pop::header(tile, if title.trim().is_empty() { host.clone() } else { title }, shown))
        .child(pop::rows(vec![
            row(glyphs::LINK, "Copy Link", Box::new(move || copy(ui, false))),
            row(glyphs::BROOM_SPARKLES, "Copy Clean Link", Box::new(move || copy(ui, true))),
            row(glyphs::QR_CODE, "Show QR Code", Box::new(move || ui.show(Screen::Qr))),
        ]))
        .child(pop::separator())
        .child(pop::rows(vec![row(
            glyphs::ELLIPSIS,
            "More…",
            Box::new(move || {
                ui.show(Screen::Browse);
                panels::share(ui, more.clone());
            }),
        )]))
        .child(pop::separator())
        .child(pop::rows(vec![row(
            glyphs::BOOKMARK,
            "Add to Reading List",
            Box::new(move || {
                ui.act(|s| {
                    s.save_for_later(browser_core::now());
                    Vec::new()
                });
                ui.show(Screen::Browse);
            }),
        )]))
}

fn qr(ui: Ui) -> impl IntoElement {
    let code = ui.read(|s| s.qr_code());
    let body = view().flex_col().items_center().gap_3().p_4();
    match code {
        Some((code, host)) => body
            .child(small("Scan to open on a phone"))
            .child(view().p_2().rounded(px(Theme::radius_row())).bg(rgb(0xffffff)).child(tarn_nsh_shared::qr::qr_code(&code, 4, QR_SIZE)))
            .child(small(host).number_of_lines(1)),
        None => body.child(small("Open a web page to show its QR code.")),
    }
}

fn downloads(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read(|s| s.downloads());
    let saving = move || ui.read(|s| !s.transfers.is_empty());
    view()
        .flex_col()
        .child(view().flex_row().items_center().justify_between().h(px(40.)).pl_3().pr_2().child(label("Downloads").font_weight(FontWeight::BOLD)).child(
            dynamic(move || {
                (!rows().is_empty()).then(|| {
                    nsh_base::button()
                        .accessibility_label("Clear Downloads List")
                        .px_1()
                        .hover(|s| s.opacity(0.8))
                        .on_press(move || {
                            ui.act(|s| {
                                s.clear_downloads();
                                Vec::new()
                            })
                        })
                        .child(label("Clear").text_color(Theme::accent))
                })
            }),
        ))
        .child(when(saving, move || view().px(px(pop::PAD)).child(sheets::transfers(ui))))
        .child(scroll().max_h(px(pop::DOWNLOAD_ROW * 5.)).child(view().flex_col().px(px(pop::PAD)).pb(px(pop::PAD)).child(each(
            rows,
            |d: &store::Download| d.id,
            move |d, _| {
                let d = d.get_untracked();
                let name = std::path::Path::new(&d.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.url.clone());
                let failed = d.state == "failed";
                pop::download_row(&name, if failed { "Failed" } else { "Saved" }.to_owned(), failed)
            },
        ))))
        .child(dynamic(move || (rows().is_empty() && !saving()).then(|| empty_state("No downloads yet"))))
}
