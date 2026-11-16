mod appearance;
mod bars;
mod controls;
mod extensions;
mod panels;
mod prompt;
mod sheets;

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use browser_core::tabs::TabKey;
use nsh_core::prelude::*;
use serde_json::{Value, json};

use crate::data::{Data, Platform};
use crate::shell::{Effect, Shell, ViewProps};
use crate::theme::{MARK_SIZE, Paint, Secure, Theme, WIDE_LAYOUT, themed};
pub use controls::{accent_pill, caption, heading, icon_button, icons, label, pill, row, segmented, small, surface, switch_row};

pub type AppShell = Shell<Data>;

const SEARCH_ROW: f32 = 48.;
const SEARCH_WIDTH: f32 = 520.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Browse,
    Tabs,
    Menu,
    Shield,
    Settings,
    History,
    Downloads,
    Spaces,
    Find,
    Reader,
    FindTabs,
    Qr,
    ReadingList,
    Move(TabKey),
    SpaceEdit(usize),
    Extensions,
    ExtensionPopup,
}

thread_local! {
    static SHELL: RefCell<Option<AppShell>> = const { RefCell::new(None) };
    static VIEWS: RefCell<HashMap<TabKey, ElementRef>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy)]
pub struct Ui {
    rev: Signal<u64>,
    pub screen: Signal<Screen>,
    pub address: Signal<String>,
    pub editing: Signal<bool>,
    pub suggest: Signal<Vec<store::HistoryEntry>>,
    pub field: ElementRef,
    pub fresh: Signal<Option<(TabKey, TabKey)>>,
}

impl Ui {
    pub fn read<T>(self, f: impl FnOnce(&AppShell) -> T) -> T {
        self.rev.get();
        SHELL.with(|s| f(s.borrow().as_ref().expect("the shell opens with the app")))
    }

    pub fn act(self, f: impl FnOnce(&mut AppShell) -> Vec<Effect>) {
        let effects = SHELL.with(|s| {
            let mut shell = s.borrow_mut();
            let shell = shell.as_mut().expect("the shell opens with the app");
            let mut effects = f(shell);
            effects.extend(shell.take_effects());
            effects
        });
        self.rev.update(|r| *r += 1);
        for effect in effects {
            match effect {
                Effect::Command(key, name, args) => VIEWS.with(|v| {
                    if let Some(view) = v.borrow().get(&key) {
                        view.command(name, args);
                    }
                }),
                Effect::PagePost(install_id, kind, json) => extensions::post(install_id, kind, json),
                Effect::Open(url) => {
                    spawn(async move {
                        let _ = open_url(url).await;
                    });
                }
            }
        }
    }

    pub fn show(self, screen: Screen) {
        if self.editing.get_untracked() {
            dismiss_keyboard();
        }
        self.editing.set(false);
        self.fresh.set(None);
        self.screen.set(screen);
    }

    pub fn wide(self) -> bool {
        window().width.0 >= WIDE_LAYOUT
    }

    fn browsing(self) -> bool {
        matches!(self.screen.get(), Screen::Browse | Screen::Find) && !self.editing.get() && self.read(|s| s.active_prompt().is_none() && s.external.is_none())
    }
}

fn open_shell() {
    SHELL.with(|s| {
        if s.borrow().is_none() {
            let dir = Data::data_dir().or_else(|| browser_core::data_dir(data_dir().as_deref())).unwrap_or_else(|| std::env::temp_dir().join("tarn"));
            std::fs::create_dir_all(&dir).ok();
            *s.borrow_mut() = Some(AppShell::open(&dir));
        }
    });
}

fn web_view(ui: Ui, item: ReadSignal<ViewProps>) -> impl IntoElement {
    let key = item.get_untracked().key;
    let handle = element_ref();
    VIEWS.with(|v| v.borrow_mut().insert(key, handle));
    native("tarn.webview")
        .element_ref(&handle)
        .prop("tab", move || json!(item.get().tab))
        .prop("profile", move || json!(item.get().profile))
        .prop("level", move || json!(item.get().level.as_str()))
        .prop("levels", move || json!(item.get().levels))
        .prop("lists", move || json!(item.get().lists))
        .prop("styles", move || json!(item.get().styles))
        .prop("themeStyles", move || json!(item.get().theme_styles))
        .prop("zaps", move || json!(item.get().zaps))
        .prop("sortDownloads", move || json!(item.get().sort_downloads))
        .prop("load", move || item.get().load.map_or(Value::Null, |l| json!({ "url": l.url, "seq": l.seq, "opener": l.opener })))
        .prop("active", move || json!(ui.browsing() && ui.read(|s| s.model.active == Some(key))))
        .on_native_event(move |e: &NativeEvent| {
            if &*e.name == crate::extensions::FRAME {
                return crate::platform::ext::frame_event(&e.data);
            }
            ui.act(|s| s.event(key, &e.name, &e.data, browser_core::now()))
        })
        .flex_1()
        .style_when(move || !(ui.browsing() && ui.read(|s| s.model.active == Some(key) && !s.active_url().is_empty())), |s| s.hidden())
}

fn notice(ui: Ui) -> impl IntoElement {
    let message = move || ui.read(|s| s.notice.clone());
    effect(move || {
        if let Some(shown) = message() {
            set_timeout(Duration::from_secs(5), move || {
                ui.act(|s| {
                    if s.notice.as_ref() == Some(&shown) {
                        s.notice = None;
                    }
                    Vec::new()
                })
            });
        }
    });
    dynamic(move || {
        message().map(|m| {
            let local = ui.read(|s| s.can_open_anyway());
            let base = if local { tarn_nsh_shared::controls::secure_surface(Theme::radius_panel()) } else { surface(Theme::radius_panel()) };
            base.flex_row()
                .items_center()
                .gap_2()
                .mx_3()
                .my_1()
                .pl_4()
                .pr_1()
                .py_1()
                .shadow(Theme::elevation(4., 14.))
                .child(label(m).flex_1().test_id("notice").text_color(move || if local { Secure::text() } else { Theme::text() }))
                .child(local.then(|| {
                    tarn_nsh_shared::controls::secure_button(
                        "Open Anyway",
                        move || {
                            ui.act(|s| {
                                s.open_anyway();
                                Vec::new()
                            })
                        },
                        true,
                    )
                }))
                .child(dynamic(move || {
                    ui.read(|s| s.can_undo()).then(|| {
                        pill("Undo", move || {
                            ui.act(|s| {
                                s.undo_close();
                                Vec::new()
                            })
                        })
                    })
                }))
                .child(icon_button(icons::CLOSE, "Dismiss", move || {
                    ui.act(|s| {
                        s.notice = None;
                        Vec::new()
                    })
                }))
        })
    })
}

fn start(ui: Ui) -> impl IntoElement {
    when(
        move || ui.read(|s| s.active_url().is_empty()) && ui.browsing(),
        move || {
            use browser_core::fog::{FRAME, frame, step};
            let fog = tarn_nsh_shared::fog::fog(
                |cols, rows, t| frame(cols, rows, step(t)).into_iter().map(|row| (row.text, row.shades)).collect(),
                Duration::from_secs_f32(FRAME),
            );
            view()
                .flex_1()
                .relative()
                .items_center()
                .justify_center()
                .gap_3()
                .child(fog)
                .child(image(icons::MARK).w(px(MARK_SIZE)).h(px(MARK_SIZE)))
                .child(themed(move || search_card(ui)))
        },
    )
}

fn search_card(ui: Ui) -> impl IntoElement {
    let field = nsh_base::button()
        .accessibility_label("Search or enter an address")
        .on_press(move || ui.field.focus())
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(SEARCH_ROW))
        .px_4()
        .rounded(px(Theme::radius_card()))
        .surface()
        .child(controls::icon(icons::SEARCH, Theme::muted))
        .child(caption("Search or enter an address"));
    view()
        .relative()
        .w_full()
        .max_w(px(SEARCH_WIDTH))
        .px(px(browser_core::effects::MAX_SIDE + 8.))
        .child(view().relative().child(tarn_nsh_shared::search_fx::search_fx(SEARCH_ROW)).child(field))
}

#[component]
pub fn App() -> impl IntoElement {
    open_shell();
    let ui = Ui {
        rev: signal(0),
        screen: signal(Screen::Browse),
        address: signal(String::new()),
        editing: signal(false),
        suggest: signal(Vec::new()),
        field: element_ref(),
        fresh: signal(None),
    };
    effect(move || {
        let light = !dark();
        untracked(|| {
            ui.act(|s| {
                s.follow_appearance(light);
                Vec::new()
            })
        });
    });
    let pages = move || ui.read(|s| s.views());
    let reading = move || ui.read(|s| s.reader.as_ref().map(|r| (r.key, r.url.clone())));
    effect(move || match reading() {
        Some(_) => ui.show(Screen::Reader),
        None if ui.screen.get_untracked() == Screen::Reader => ui.show(Screen::Browse),
        None => {}
    });
    let follow = move || {
        if ui.editing.get_untracked() || !matches!(ui.screen.get_untracked(), Screen::Browse) {
            return;
        }
        ui.act(|s| {
            if let Ok((weekday, hour)) = s.core.disk.local_time() {
                s.follow_schedule(weekday, hour, browser_core::now());
            }
            Vec::new()
        });
    };
    follow();
    set_interval(Duration::from_secs(60), follow);
    extensions::start(ui);
    view()
        .flex_1()
        .pt_safe_area()
        .px_safe_area()
        .relative()
        .text_color(Theme::text)
        .font_family(Theme::font_family)
        .child(themed(|| view().absolute().inset_0().paint(Theme::backdrop())))
        .child(bars::top_bar(ui))
        .child(notice(ui))
        .child(sheets::picking_banner(ui))
        .child(bars::page_error(ui))
        .child(
            view()
                .flex_1()
                .child(each(pages, |v: &ViewProps| (v.key, v.born), move |item, _| web_view(ui, item)))
                .child(start(ui))
                .child(bars::suggestions(ui))
                .child(prompt::prompt_panel(ui))
                .child(panels::panel(ui)),
        )
        .child(themed(move || bars::bottom_bar(ui)))
        .child(extensions::backgrounds(ui))
}
