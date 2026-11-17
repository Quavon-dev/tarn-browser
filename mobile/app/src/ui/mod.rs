mod appearance;
mod bars;
mod controls;
mod extensions;
mod mac;
mod panels;
mod prompt;
mod share;
mod sheets;
mod split;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use browser_core::tabs::TabKey;
use nsh_core::prelude::*;
use nsh_core::{LayoutAnimation, LayoutCurve, LayoutTransition};
use serde_json::{Value, json};
use tarn_nsh_shared::controls::Density;

use crate::data::{Data, Platform};
use crate::shell::{Effect, Shell, ViewProps};
use crate::theme::{MARK_SIZE, Paint, Secure, Theme, WIDE_LAYOUT, themed};
pub use controls::{
    accent_pill, caption, detail_row, framed, heading, icon_button, icons, label, pill, row, section, segmented, small, surface, switch_row, text_field,
};

pub type AppShell = Shell<Data>;

const SEARCH_ROW: f32 = 48.;
const SEARCH_WIDTH: f32 = 520.;
const PANE_HEADER: f32 = 40.;
const PANE_GAP: f32 = 2.;

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
    Share,
    ReadingList,
    Move(TabKey),
    SpaceNew,
    SpaceEdit(usize),
    Licences,
    Licence(usize),
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
    /// Moves on every action that may change the profile's stored data, not on the pages' events.
    changes: Signal<u64>,
    /// The page area's frame in the window.
    pub area: Signal<Bounds<Points>>,
    /// A pane header being dragged, and the pane it would swap with.
    pub dragging: Signal<Option<(TabKey, Option<TabKey>)>>,
    /// The suggestion row the keyboard's arrows highlight.
    pub picked: Signal<Option<usize>>,
    pub screen: Signal<Screen>,
    pub address: Signal<String>,
    pub editing: Signal<bool>,
    pub suggest: Signal<Vec<store::HistoryEntry>>,
    pub field: ElementRef,
    pub fresh: Signal<Option<(TabKey, TabKey)>>,
    pub mac: Signal<bool>,
    pub popovers: mac::popovers::Anchors,
}

impl Ui {
    pub fn read<T>(self, f: impl FnOnce(&AppShell) -> T) -> T {
        self.rev.get();
        SHELL.with(|s| f(s.borrow().as_ref().expect("the shell opens with the app")))
    }

    /// Reads what only the actions change, such as the profile's stored data, and not the pages' events.
    pub fn read_data<T>(self, f: impl FnOnce(&AppShell) -> T) -> T {
        self.changes.get();
        SHELL.with(|s| f(s.borrow().as_ref().expect("the shell opens with the app")))
    }

    pub fn act(self, f: impl FnOnce(&mut AppShell) -> Vec<Effect>) {
        self.run(f, true);
    }

    /// An action that changes what is on screen but none of the profile's stored data.
    pub fn act_on_screen(self, f: impl FnOnce(&mut AppShell) -> Vec<Effect>) {
        self.run(f, false);
    }

    fn event(self, key: TabKey, e: &NativeEvent) {
        if &*e.name == "pressed" && self.read(|s| s.model.active == Some(key)) {
            return;
        }
        if &*e.name == browser_core::extensions::STORE_EVENT {
            extensions::store_link(self, key, e.data["id"].as_str().unwrap_or_default());
        }
        if &*e.name == "iconData" {
            let (name, data) = (e.name.to_string(), e.data.clone());
            let encoded = data.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_owned();
            spawn(async move {
                let png = blocking(move || engine::favicon::normalize(&encoded)).await.ok().flatten();
                self.run(
                    |s| {
                        s.icon_event(key, &name, &data, png);
                        Vec::new()
                    },
                    false,
                );
            });
            return;
        }
        // A picked element is stored, and a pop-up may open in another profile's space.
        self.run(|s| s.event(key, &e.name, &e.data, browser_core::now()), matches!(&*e.name, "picked" | "popup"));
    }

    fn run(self, f: impl FnOnce(&mut AppShell) -> Vec<Effect>, changed: bool) {
        let effects = SHELL.with(|s| {
            let mut shell = s.borrow_mut();
            let shell = shell.as_mut().expect("the shell opens with the app");
            let mut effects = f(shell);
            effects.extend(shell.take_effects());
            effects
        });
        self.rev.update(|r| *r += 1);
        if changed {
            self.changes.update(|r| *r += 1);
        }
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
        if screen == Screen::Find {
            self.act(|s| {
                s.found = None;
                Vec::new()
            });
        }
        if screen != self.screen.get_untracked() && !untracked(reduce_motion) {
            let fade = LayoutTransition::new(LayoutCurve::EaseOut, tarn_nsh_shared::motion::FADE.as_secs_f32() * 1000.);
            animate_layout(LayoutAnimation { create: Some(fade), ..LayoutAnimation::default() });
        }
        self.screen.set(screen);
    }

    pub fn toggle(self, screen: Screen) {
        self.show(if self.screen.get_untracked() == screen { Screen::Browse } else { screen });
    }

    pub fn edit_address(self) {
        if self.mac.get_untracked() {
            mac::open_palette(self);
        } else {
            self.field.focus();
        }
    }

    pub fn wide(self) -> bool {
        window().width.0 >= WIDE_LAYOUT
    }

    /// Where `key` sits in the split on screen; a phone shows one pane at a time.
    fn pane(self, key: TabKey) -> Option<browser_core::split::Share> {
        if self.wide() { self.read(|s| s.pane_share(key)) } else { None }
    }

    fn browsing(self) -> bool {
        let screen = self.screen.get();
        (matches!(screen, Screen::Browse | Screen::Find | Screen::Share) || self.mac.get() && mac::owns(screen))
            && (!self.editing.get() || self.mac.get())
            && self.read(|s| s.active_prompt().is_none() && s.external.is_none())
    }
}

fn shell_dir() -> std::path::PathBuf {
    Data::data_dir().or_else(|| browser_core::data_dir(data_dir().as_deref())).unwrap_or_else(|| std::env::temp_dir().join("tarn"))
}

fn open_shell() {
    SHELL.with(|s| {
        if s.borrow().is_none() {
            let dir = shell_dir();
            std::fs::create_dir_all(&dir).ok();
            *s.borrow_mut() = Some(AppShell::open(&dir));
        }
    });
}

fn web_view(ui: Ui, item: ReadSignal<ViewProps>) -> impl IntoElement {
    let key = item.get_untracked().key;
    let handle = element_ref();
    VIEWS.with(|v| v.borrow_mut().insert(key, handle));
    let page = native("tarn.webview")
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
        .prop("active", move || {
            let shown = ui.pane(key).is_some();
            json!(ui.read(|s| (shown || s.model.active == Some(key)) && s.active_prompt().is_none() && s.external.is_none()))
        })
        .on_native_event(move |e: &NativeEvent| {
            if &*e.name == crate::extensions::FRAME {
                return crate::platform::ext::frame_event(&e.data);
            }
            ui.event(key, e)
        })
        .flex_1();
    let focused = move || ui.read(|s| s.model.active == Some(key)) || ui.dragging.get().is_some_and(|(_, over)| over == Some(key));
    view()
        .style_with(move |s| match ui.pane(key) {
            Some((x, y, w, h)) => {
                s.absolute().left(relative(x)).top(relative(y)).w(relative(w)).h(relative(h)).p(px(PANE_GAP)).border_1().border_color(if focused() {
                    Theme::accent()
                } else {
                    Theme::surface_edge()
                })
            }
            None => s.flex_1(),
        })
        .style_when(
            move || !(ui.browsing() && (ui.pane(key).is_some() || ui.read(|s| s.model.active == Some(key) && !s.active_url().is_empty()))),
            |s| s.hidden(),
        )
        .child(dynamic(move || ui.pane(key).is_some().then(|| pane_header(ui, key))))
        .child(page)
}

fn pane_header(ui: Ui, key: TabKey) -> impl IntoElement {
    let title = move || ui.read(|s| Some(s.tab_title(key)).filter(|t| !t.is_empty()).unwrap_or_else(|| "New Tab".to_owned()));
    split::draggable(ui, key, view())
        .flex_row()
        .items_center()
        .h(px(PANE_HEADER))
        .bg(move || if ui.read(|s| s.model.active == Some(key)) { Theme::panel_row() } else { Theme::surface() })
        .child(
            nsh_base::button()
                .accessibility_label(title())
                .flex_1()
                .h_full()
                .justify_center()
                .pl_3()
                .on_press(move || {
                    ui.act(|s| {
                        s.activate(key);
                        Vec::new()
                    })
                })
                .child(label(title).number_of_lines(1)),
        )
        .child(icon_button(icons::SHARE, "Remove from Split View", move || {
            ui.act(|s| {
                s.leave_split(key);
                Vec::new()
            })
        }))
        .child(icon_button(icons::CLOSE, "Close Tab", move || {
            ui.act(|s| {
                s.close_tab(key);
                Vec::new()
            })
        }))
}

fn notice(ui: Ui) -> impl IntoElement {
    let message = move || ui.read(|s| s.notice.clone());
    let shown = computed(move || ui.read(|s| (s.notice.clone(), s.undo.clone())));
    let timer: Rc<RefCell<Option<Spawned<()>>>> = Rc::default();
    effect(move || {
        if let Some(old) = timer.borrow_mut().take() {
            old.cancel();
        }
        if let (Some(shown), _) = shown.get() {
            *timer.borrow_mut() = Some(set_timeout(Duration::from_secs(5), move || {
                ui.act(|s| {
                    if s.notice.as_ref() == Some(&shown) {
                        s.notice = None;
                    }
                    Vec::new()
                })
            }));
        }
    });
    dynamic(move || {
        message().map(|m| {
            let local = ui.read(|s| s.can_open_anyway());
            let base = if local {
                tarn_nsh_shared::controls::secure_surface(Theme::radius_panel()).flex_row().items_center().gap_2().pl_4().pr_1().py_1()
            } else {
                tarn_nsh_shared::controls::toast(ui.read(|s| s.notice_tone())).self_center()
            };
            base.bind_motion(&tarn_nsh_shared::motion::pop_in())
                .mx_3()
                .my_1()
                .shadow(Theme::elevation(4., 14.))
                .child(
                    label(m)
                        .flex_shrink(1.)
                        .flex_grow(if local { 1. } else { 0. })
                        .test_id("notice")
                        .text_color(move || if local { Secure::text() } else { Theme::text() }),
                )
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
                    ui.read(|s| s.can_undo_sort()).then(|| {
                        tarn_nsh_shared::controls::toast_action("Undo Sort", move || {
                            ui.act(|s| {
                                s.undo_sort();
                                Vec::new()
                            })
                        })
                    })
                }))
                .child(dynamic(move || {
                    ui.read(|s| s.can_undo()).then(|| {
                        tarn_nsh_shared::controls::toast_action("Undo", move || {
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
        move || ui.read(|s| s.active_url().is_empty()) && ui.read(|s| s.model.active).and_then(|a| ui.pane(a)).is_none() && ui.browsing(),
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
        .on_press(move || ui.edit_address())
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

fn back(ui: Ui) -> bool {
    if ui.read(|s| s.active_prompt().is_some() || s.external.is_some()) {
        return true;
    }
    if ui.editing.get_untracked() {
        bars::cancel(ui);
        return true;
    }
    match ui.screen.get_untracked() {
        Screen::Browse => {
            let can = ui.read(|s| s.active_page().is_some_and(|p| p.back));
            if can {
                ui.act(|s| s.active_command("back", Value::Null));
            }
            can
        }
        screen if ui.mac.get_untracked() && mac::owns(screen) => {
            ui.show(mac::back_from(screen));
            true
        }
        screen => {
            panels::leave(ui, panels::parent(screen).unwrap_or(Screen::Browse));
            true
        }
    }
}

#[component]
pub fn App() -> impl IntoElement {
    open_shell();
    let ui = Ui {
        rev: signal(0),
        changes: signal(0),
        area: signal(Bounds::default()),
        dragging: signal(None),
        picked: signal(None),
        screen: signal(Screen::Browse),
        address: signal(String::new()),
        editing: signal(false),
        suggest: signal(Vec::new()),
        field: element_ref(),
        fresh: signal(None),
        mac: signal(false),
        popovers: mac::popovers::Anchors::default(),
    };
    let extensions_open = std::cell::Cell::new(false);
    effect(move || {
        let open = ui.screen.get() == Screen::Extensions;
        if extensions_open.replace(open) && !open {
            untracked(tarn_nsh_shared::extensions::close_store_sheet);
        }
    });
    effect(move || {
        let light = !dark();
        untracked(|| {
            ui.act(|s| {
                s.follow_appearance(light);
                Vec::new()
            })
        });
    });
    effect(move || {
        let wide = ui.wide();
        untracked(|| {
            ui.act(|s| {
                s.set_wide(wide);
                Vec::new()
            })
        });
    });
    effect(move || {
        let size = window();
        let setting = ui.read_data(|s| s.layout);
        let mac = browser_core::layout::effective(setting, size.width.0, size.height.0) == browser_core::layout::Layout::Mac;
        if mac != ui.mac.get_untracked() {
            if ui.editing.get_untracked() {
                bars::cancel(ui);
            }
            tarn_nsh_shared::controls::set_density(if mac { Density::Desktop } else { Density::Touch });
            ui.mac.set(mac);
        }
    });
    on_back_press(move || untracked(|| back(ui)));
    let profile = computed(move || ui.read_data(|s| s.profile_props()));
    let pages = computed(move || profile.with(|p| ui.read(|s| s.views_from(p.as_ref()))));
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
        .child(mac::frame(
            ui,
            move || bars::top_bar(ui),
            view()
                .flex_1()
                .relative()
                .on_layout(move |bounds: &Bounds<Points>| ui.area.set(*bounds))
                .child(
                    view()
                        .z_index(1)
                        .style_when(move || ui.browsing(), |s| s.absolute().top(px(0.)).left(px(0.)).right(px(0.)))
                        .child(notice(ui))
                        .child(sheets::picking_banner(ui))
                        .child(bars::page_error(ui)),
                )
                .child(each(move || pages.get(), |v: &ViewProps| (v.key, v.born), move |item, _| web_view(ui, item)))
                .child(split::dividers(ui))
                .child(start(ui))
                .child(bars::suggestions(ui))
                .child(prompt::prompt_panel(ui))
                .child(panels::panel(ui)),
            move || bars::bottom_bar(ui),
        ))
        .child(share::share_sheet(ui))
        .child(extensions::backgrounds(ui))
}
