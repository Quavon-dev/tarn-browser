use browser_core::tabs::TabKey;
use devkit::json_view::{Kind, Tree};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::panels::section;
use super::tools::{line, note};
use super::{Panel, Ui, WINDOWS, act, act_quietly, read};
use crate::controls::{group, group_row, icon_button, label, pill, small, switch};
use crate::icons;
use crate::theme::{Paint, TEXT_SMALL, Theme};

const MONO: &str = "monospace";
const ROW: f32 = 20.;

pub(super) fn json_text(ui: Ui, key: TabKey, data: JsonValue) {
    let text = data["text"].as_str().unwrap_or_default().to_owned();
    drop(spawn(async move {
        let tree = if text.is_empty() { None } else { blocking(move || Tree::parse(&text).ok()).await.ok().flatten() };
        act_quietly(ui, |b| {
            if b.json_request(key, &data)
                && let Some(tree) = tree
            {
                b.show_json(key, data["url"].as_str().unwrap_or_default(), tree);
            }
        });
    }));
}

fn value_color(kind: Kind) -> Rgba {
    match kind {
        Kind::String => Theme::success(),
        Kind::Number | Kind::Bool => Theme::warning(),
        Kind::Null | Kind::Array | Kind::Object => Theme::muted(),
    }
}

fn tree_rows(ui: Ui, tree: &Tree) -> View {
    let rows: Vec<AnyElement> = tree
        .rows
        .iter()
        .map(|row| {
            let chevron = view()
                .w(px(12.))
                .flex_none()
                .children(row.expandable().then(|| icons::icon(if row.open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 10.)));
            let label = (!row.label.is_empty()).then(|| text(format!("{}:", row.label)).text_color(Theme::accent_text()));
            let value = text(row.value.clone()).text_color(value_color(row.kind)).truncate();
            let line =
                view().flex_row().items_center().gap_1().h(px(ROW)).pl(px(12. + row.depth as f32 * 16.)).pr_3().child(chevron).children(label).child(value);
            if row.expandable() {
                let path = row.path.clone();
                let name = if row.open { "Collapse" } else { "Expand" };
                crate::controls::pressable(name, move || act(ui, |b| b.toggle_json_node(&path)))
                    .flex_row()
                    .hover(|s| s.bg(Theme::hover()))
                    .child(line.flex_1())
                    .into_element()
            } else {
                line.into_element()
            }
        })
        .collect();
    let more = tree.truncated().then(|| small("Only the first 2,000 rows are shown.").px_3());
    view().flex_col().py_2().font_family(MONO).text_size(px(TEXT_SMALL)).children(rows).children(more)
}

pub(super) fn json_view(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.json.is_some()) && ui.panel.get() == Panel::None && !ui.editing.get(),
        move || {
            dynamic(move || {
                let tree = ui.snap.with(|s| s.json.clone())?;
                let mode = |name: &'static str, raw: bool| {
                    let press = move || act(ui, move |b| b.set_json_raw(raw));
                    if tree.raw == raw { crate::controls::accent_pill(name, press) } else { pill(name, press) }
                };
                let header = view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h(px(40.))
                    .px_3()
                    .border_b_1()
                    .border_color(Theme::hairline())
                    .text_color(Theme::muted())
                    .child(icons::icon(icons::CODE, 14.))
                    .child(label("JSON").font_weight(FontWeight::MEDIUM))
                    .child(view().flex_1())
                    .child(mode("Tree", false))
                    .child(mode("Raw", true));
                let body = if tree.raw {
                    view().p_3().child(text(tree.pretty.clone()).font_family(MONO).text_size(px(TEXT_SMALL)).text_color(Theme::text()))
                } else {
                    tree_rows(ui, &tree)
                };
                Some(view().flex_col().flex_1().bg(Theme::page()).child(header).child(scroll().flex_1().child(body)))
            })
        },
    )
    .into_element()
}

pub(super) fn responsive_button(ui: Ui) -> AnyElement {
    dynamic(move || {
        if !ui.snap.with(|s| s.devtools) {
            return None;
        }
        let preset = ui.snap.with(|s| s.responsive);
        let color = if preset.is_some() { Theme::accent_text() } else { Theme::muted() };
        let tip = preset.map_or_else(|| "Responsive Size".to_owned(), |p| format!("Responsive Size: {} {}×{}", p.name, p.width, p.height));
        let button = icon_button(icons::SMARTPHONE, &tip, move || act(ui, |b| b.cycle_responsive())).text_color(color);
        Some(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(button)
                .children(preset.map(|p| small(format!("{} {}×{}", p.name, p.width, p.height)).text_color(color))),
        )
    })
}

pub(super) fn measure_button(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.snap.with(|s| s.devtools && s.tab && !super::no_page(s)).then(|| {
            let on = ui.page_overlay.with(|o| o.as_ref().is_some_and(|o| o.purpose == tarn_nsh_shared::page_overlay::Purpose::Measure));
            icon_button(icons::CODE, if on { "Close Measure" } else { "Measure" }, move || {
                super::capture::open(ui, tarn_nsh_shared::page_overlay::Purpose::Measure)
            })
            .text_color(Theme::muted())
            .when(on, |b| b.bg(Theme::pressed()))
        })
    })
}

pub(super) fn refresh_servers(ui: Ui) {
    ui.root.run(|| {
        drop(spawn(async move {
            let ports = blocking(devkit::localhost::open_dev_ports).await.unwrap_or_default();
            act(ui, |b| b.local_servers = Some(ports));
        }))
    });
}

fn open_server(ui: Ui, port: u16) {
    ui.panel.set(Panel::None);
    act(ui, |b| b.open_local_server(port));
}

pub(super) fn server_strip(ui: Ui) -> AnyElement {
    dynamic(move || {
        if !ui.snap.with(|s| s.devtools) {
            return None;
        }
        let servers = ui.snap.with(|s| s.local_servers.clone());
        let heading = match &servers {
            None => "Looking for local servers…",
            Some(p) if p.is_empty() => "No local servers on common dev ports",
            Some(_) => "Local servers",
        };
        let chips: Vec<AnyElement> =
            servers.unwrap_or_default().into_iter().map(|port| pill(format!("localhost:{port}"), move || open_server(ui, port)).into_element()).collect();
        Some(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .mt_3()
                .p_2()
                .rounded(px(Theme::radius_panel()))
                .surface()
                .text_color(Theme::muted())
                .child(icons::icon(icons::CODE, 12.))
                .child(small(heading))
                .children(chips)
                .child(pill("Refresh", move || refresh_servers(ui))),
        )
    })
}

pub(super) fn pane(ui: Ui, body: View) -> View {
    let Some((on, dev_profile, servers)) = read(ui, |b| (b.developer(), b.dev_profile(), b.local_servers.clone())) else { return body };
    let toggle = move || {
        act(ui, move |b| b.set_developer(!on));
        WINDOWS.with_borrow(Vec::clone).into_iter().filter(|w| w.window != ui.window).for_each(|w| act(w, |_| {}));
        if !on {
            refresh_servers(ui);
        }
    };
    let profile = match dev_profile {
        Some(name) => group_row().child(label(name).flex_1()).child(small("Created")),
        None => group_row().child(label("A separate profile for development").flex_1()).child(pill("Create", move || {
            ui.panel.set(Panel::None);
            act(ui, |b| b.create_dev_profile());
            refresh_servers(ui);
        })),
    };
    let servers = match servers {
        None => view().child(note("Not checked yet.")),
        Some(ports) if ports.is_empty() => view().child(note("Nothing is listening on the common dev ports (3000, 5173, 8080 and others).")),
        Some(ports) => {
            let rows: Vec<AnyElement> =
                ports.into_iter().map(|port| line(format!("localhost:{port}")).child(pill("Open", move || open_server(ui, port))).into_element()).collect();
            view().flex_col().children(rows)
        }
    };
    body.child(section("Developer tools"))
        .child(group().child(switch(on, "Web Inspector, JSON viewer, responsive sizes and local servers", toggle)))
        .child(note("Applies to every profile; open pages reload. Right-click a page and choose Inspect Element; the toolbar gains responsive sizes. Never on Lockdown pages."))
        .child(section("Developer profile"))
        .child(group().child(profile))
        .child(note("Web Inspector and the developer tools are always on there; caches are cleared after every page load and Reload bypasses the cache."))
        .child(view().flex_row().items_center().child(section("Local servers").flex_1()).child(pill("Refresh", move || refresh_servers(ui))))
        .child(servers)
}
