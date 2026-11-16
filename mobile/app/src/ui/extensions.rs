use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use browser_core::extensions::install::access_title;
use engine::tarn_ext::PageKind;
use nsh_base::button;
use nsh_core::prelude::*;
use serde_json::{Value, json};
use store::{Id, Store};
use tarn_nsh_shared::controls::{secure_button, secure_surface};
use tarn_nsh_shared::extensions::{Host, extensions};
use tarn_nsh_shared::theme::Secure;

use super::{SHELL, Screen, Ui, icons, label, row};
use crate::extensions::{MESSAGE, POST, PageView, TICK};
use crate::platform::ext;
use crate::theme::{TEXT_TITLE, TOUCH_TARGET, Theme};

const POPUP_HEIGHT: f32 = 480.;
pub const NO_WORLDS: &str = "Content scripts are not supported by this Android System WebView. Extensions' network rules, pages and storage still work.";

thread_local! {
    static PAGES: RefCell<HashMap<(String, PageKind), ElementRef>> = RefCell::new(HashMap::new());
}

pub(super) fn post(install_id: String, kind: PageKind, json: String) {
    PAGES.with(|p| {
        if let Some(view) = p.borrow().get(&(install_id.clone(), kind)) {
            view.command(POST, json!({ "world": crate::extensions::world(&install_id), "frame": Value::Null, "json": json }));
        }
    });
}

#[derive(Clone, Copy)]
struct Exts(Ui);

impl Host for Exts {
    type Transport = crate::platform::Transport;
    fn transport() -> crate::platform::Transport {
        crate::platform::Transport
    }
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
        SHELL.with(|s| f(&s.borrow().as_ref().expect("the shell opens with the app").core.disk))
    }
    fn profile(self) -> Option<(Id, String)> {
        self.0.read(|s| {
            let id = s.extension_profile()?;
            Some((id, s.core.profiles.borrow().get(&(false, id))?.own_store.clone()))
        })
    }
    fn root(self) -> PathBuf {
        self.0.read(|s| s.ext.root.clone())
    }
    fn catalog_dir(self) -> PathBuf {
        self.0.read(|s| s.ext.root.with_file_name("catalog-extensions"))
    }
    fn new_id(self) -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }
    fn ui_locales(self) -> Vec<String> {
        vec![locale().to_string()]
    }
    fn changed(self) {
        self.0.act(|s| {
            if let Some(profile) = s.extension_profile() {
                s.load_extensions(profile);
            }
            Vec::new()
        });
    }
    fn notice(self) -> Option<String> {
        (!ext::worlds_supported()).then(|| NO_WORLDS.to_owned())
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    extensions(Exts(ui))
}

/// Registers the frame gate, the UI locale and the timer that stops idle background pages.
pub(super) fn start(ui: Ui) {
    let locale = locale().to_string();
    ui.act(|s| {
        s.ext.locales = vec![locale];
        if let Some(profile) = s.extension_profile() {
            s.load_extensions(profile);
        }
        Vec::new()
    });
    ext::set_gate(Rc::new(move |tab: &str, store: &str, world: &engine::tarn_ext::World, nav: &engine::tarn_ext::FrameNav<'_>| {
        let gated = SHELL.with(|s| {
            let mut shell = s.try_borrow_mut().ok()?;
            let shell = shell.as_mut()?;
            let asked = shell.ext.asks.len();
            let run = shell.gate(tab, store, world, nav, Instant::now());
            Some((run, shell.ext.asks.len() != asked))
        });
        let Some((run, asked)) = gated else { return Vec::new() };
        if asked {
            set_timeout(std::time::Duration::ZERO, move || ui.act(|_| Vec::new()));
        }
        run
    }));
    drop(set_interval(TICK, move || ui.act(|s| s.extensions_tick(Instant::now()))));
}

fn page_view(ui: Ui, page: PageView, hidden: bool) -> impl IntoElement {
    let handle = element_ref();
    PAGES.with(|p| p.borrow_mut().insert((page.install_id.clone(), page.kind), handle));
    let (profile, install_id, kind) = (page.profile, page.install_id.clone(), page.kind);
    native("tarn.extpage")
        .element_ref(&handle)
        .prop("profile", json!(page.store_id))
        .prop("kind", json!(if page.kind == PageKind::Background { "background" } else { "popup" }))
        .prop("world", json!(page.world))
        .prop("bootstrap", json!(page.bootstrap))
        .prop("level", json!(page.level))
        .prop("url", json!(page.url))
        .on_native_event(move |e: &NativeEvent| {
            if &*e.name == MESSAGE {
                ui.act(|s| s.page_message(profile, &install_id, kind, &e.data, Instant::now()));
            }
        })
        .style_when(move || hidden, |s| s.hidden())
        .style_when(move || !hidden, |s| s.h(px(POPUP_HEIGHT)).w_full())
}

/// The hidden background pages of the running extensions.
pub(super) fn backgrounds(ui: Ui) -> impl IntoElement {
    let pages = move || ui.read(|s| s.page_views());
    view().hidden().child(each(
        pages,
        |p: &PageView| (p.profile, p.install_id.clone(), p.url.clone()),
        move |item, _| page_view(ui, item.get_untracked(), true),
    ))
}

/// The action popup, in a sheet with its title and Done.
pub(super) fn popup(ui: Ui) -> impl IntoElement {
    let shown = computed(move || ui.read(|s| s.popup()));
    dynamic(move || {
        let page = shown.get()?;
        Some(
            crate::ui::surface(Theme::radius_card())
                .flex_1()
                .mx_2()
                .mt_1()
                .mb_2()
                .overflow_hidden()
                .child(
                    view().flex_row().items_center().pl_4().pr_2().pt_2().child(label(page.title.clone()).flex_1().font_weight(FontWeight::BOLD)).child(
                        button()
                            .accessibility_label("Done")
                            .h(px(TOUCH_TARGET))
                            .px_3()
                            .justify_center()
                            .on_press(move || close(ui))
                            .child(text("Done").text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::accent)),
                    ),
                )
                .child(page_view(ui, page, false))
                .into_element(),
        )
    })
}

pub(super) fn close(ui: Ui) {
    ui.act(|s| s.close_popup());
    ui.show(Screen::Browse);
}

/// The menu entries of the extensions with an action, with their badge.
pub(super) fn actions(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let items = ui.read(|s| s.action_items());
        (!items.is_empty()).then(|| {
            let rows = items.into_iter().map(|item| {
                let id = item.install_id.clone();
                let badge = item.state.badge_text.clone();
                row(icons::APP_WINDOW, item.state.title.clone(), move || {
                    ui.act(|s| s.click_action(&id, Instant::now()));
                    let opened = ui.read(|s| s.popup().is_some());
                    ui.show(if opened { Screen::ExtensionPopup } else { Screen::Browse });
                })
                .child((!badge.is_empty()).then(|| {
                    let color = item.state.badge_color.map_or_else(Theme::accent, |[r, g, b, a]| Rgba {
                        r: f32::from(r) / 255.,
                        g: f32::from(g) / 255.,
                        b: f32::from(b) / 255.,
                        a: f32::from(a) / 255.,
                    });
                    view().px_2().rounded_full().bg(color).child(text(badge).text_size(px(12.)).text_color(Theme::on_accent()))
                }))
                .into_element()
            });
            tarn_nsh_shared::controls::group().overflow_hidden().child(view().mb(px(-1.)).children(rows)).into_element()
        })
    })
}

/// The per-site question over its tab: Allow Once, Always Allow, Deny, Never.
pub(super) fn ask(ui: Ui) -> Option<View> {
    let ask = ui.read(|s| s.active_ask().cloned())?;
    let answer = move |allow: bool, remember: bool| move || ui.act(|s| s.answer_ask(allow, remember, Instant::now()));
    Some(
        secure_surface(Theme::radius_panel())
            .flex_col()
            .gap_3()
            .p_4()
            .m_3()
            .shadow(Theme::elevation(4., 14.))
            .child(text(access_title(&ask.name, &ask.badge, &ask.host)).font_weight(FontWeight::SEMIBOLD).text_color(Secure::text))
            .child(text("Allow Once lasts an hour or until you close this tab.").text_color(Secure::muted))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(secure_button("Allow Once", answer(true, false), true))
                    .child(secure_button("Always Allow", answer(true, true), false))
                    .child(secure_button("Deny", answer(false, false), false))
                    .child(secure_button("Never", answer(false, true), false)),
            ),
    )
}
