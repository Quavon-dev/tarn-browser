//! Tarn extensions in the desktop shell (`browser_core::tarn_ext`): checked installs go into the
//! profile's views, the site prompt is raised where the gate asks, and the runtime's effects are
//! carried out. Every decision is the core's; this module only renders and forwards.

#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser_core::tarn_ext::grants::Answer;
use browser_core::tarn_ext::router::{From, Sender, TabInfo, Target};
use browser_core::tarn_ext::runtime::{Action, Call, Effect, Policy, Runtime};
use browser_core::tarn_ext::{self as core_ext, LoadError, Package};
use engine::tarn_ext::{ExtensionView, FrameId, FrameNav, Gate, PageKind, PageSpec, World};
use engine::{Event, ExtensionPage, Host, Prompt};
use futures::channel::mpsc::UnboundedSender;
use gpui::{
    App, AsyncApp, ClickEvent, Image, ImageFormat, InteractiveElement, IntoElement, ParentElement, Rgba, StatefulInteractiveElement, Styled, div, img,
    prelude::FluentBuilder, px,
};
use shield::ShieldLevel;
use store::{Id, InstalledExtension};
use ui::controls::{icon_button, tooltip};
use ui::icon::Icon;
use ui::theme::{TEXT_TINY, Theme};
use url::Url;

use crate::browser::{Browser, TabKey};
use crate::prefs::Pref;
use crate::shared::AppCore;

pub(crate) const TICK: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const GROUP: &str = "tarn-ext";

/// What reaches the shell from the engine and the UI, handled outside any window's update.
pub(crate) enum Msg {
    Tab { profile: Id, tab: u64, world: World, frame: FrameId, origin: Option<Url>, json: String },
    Page { profile: Id, install: String, kind: PageKind, event: Event },
    PageClosed { profile: Id, install: String, kind: PageKind },
    Answered(Answer),
    Clicked { profile: Id, install: String, tab: Option<u64>, host: Host, x: f64, y: f64 },
    TabClosed(u64),
    Navigated { profile: Id, tab: u64 },
}

type Effects = Vec<Effect<ExtensionPage>>;

#[derive(Default)]
pub(crate) struct Tarn {
    rt: Runtime<ExtensionPage, Arc<Image>>,
    /// Hosts the gate asked about, by tab, raised once the tab's page commits.
    asks: HashMap<u64, Vec<(Id, String, String)>>,
    sender: Option<UnboundedSender<Msg>>,
    /// The worlds each profile's views were given.
    worlds: HashMap<Id, HashSet<World>>,
    /// Profiles whose rule lists Chromium's request filter holds.
    chromium_rules: HashSet<Id>,
}

impl Tarn {
    pub(crate) fn set_sender(&mut self, sender: UnboundedSender<Msg>) {
        self.sender = Some(sender);
    }

    pub(crate) fn send(&self, msg: Msg) {
        if let Some(sender) = &self.sender {
            sender.unbounded_send(msg).ok();
        }
    }

    pub(crate) fn loaded(&self, profile: Id, install: &str) -> Option<(String, String)> {
        let l = self.rt.loaded(profile, install)?;
        Some((l.package.name.clone(), l.package.manifest.version.clone()))
    }
}

pub(crate) fn ui_locales() -> Vec<String> {
    #[cfg(any(target_os = "macos", windows))]
    return engine::translation::preferred_languages();
    #[cfg(not(any(target_os = "macos", windows)))]
    Vec::new()
}

fn seed(core: &AppCore, profile: Id) -> Option<String> {
    core.disk.profile(profile).ok().map(|p| p.data_store_uuid)
}

fn policy(core: &AppCore, profile: Id) -> Policy {
    core.extension_policy(profile, core.pref(Pref::ExtensionsEverySite))
}

/// The engine's gate for a view of tab `tab` at `view_level`: the core decides, and a host the
/// extension wants but was not asked about yet is kept for the prompt after the page commits.
fn gate(core: &Rc<AppCore>, profile: Id, tab: u64, view_level: ShieldLevel) -> Gate {
    let weak = Rc::downgrade(core);
    Rc::new(move |world: &World, frame: &FrameNav| {
        let Some(core) = weak.upgrade() else { return Vec::new() };
        let policy = policy(&core, profile);
        let Ok(mut tarn) = core.tarn.try_borrow_mut() else { return Vec::new() };
        let decision = tarn.rt.gate(profile, world, frame, &policy, view_level, Instant::now());
        if let Some(host) = decision.ask {
            tarn.asks.entry(tab).or_default().push((profile, world.install_id().to_owned(), host));
        }
        decision.run
    })
}

/// Gives a tab's new view the gate and the profile's extensions; never in a private space.
pub(crate) fn attach(core: &Rc<AppCore>, key: TabKey, profile: Id, view: &dyn ExtensionView, view_level: ShieldLevel) {
    if key.private {
        return;
    }
    view.set_extension_gate(gate(core, profile, key.id as u64, view_level));
    let scripts = core.tarn.borrow().rt.scripts(profile);
    for scripts in scripts {
        view.add_extension_scripts(scripts);
    }
}

/// Loads a Tarn install whose tree hash matched its row, off the main thread, hashing it again
/// as it reads it: a change turns it off and drops its site permissions.
pub(crate) fn load(profile: Id, row: InstalledExtension, cx: &mut App) {
    let core = crate::shared::core(cx);
    let Some(seed) = seed(&core, profile) else { return };
    let (dir, expected, install) = (row.path.clone(), row.sha256.clone(), row.install_id.clone());
    let work =
        cx.background_executor().spawn(async move { core_ext::load(Path::new(&dir), Some(expected.as_str()), &install, &seed, &ui_locales()).map(Box::new) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = work.await;
        cx.update(|cx| {
            let core = crate::shared::core(cx);
            match result {
                Ok(package) => {
                    add(&core, row.clone(), *package);
                    core.extensions.borrow_mut().mark_verified(&row.path);
                }
                Err(LoadError::Changed) => {
                    crate::extensions::turn_off_row(&core, profile, &row, "changed on disk and was turned off; its site permissions were removed.")
                }
                Err(LoadError::Refused(why)) => crate::extensions::turn_off_row(
                    &core,
                    profile,
                    &row,
                    &format!("could not be loaded and was turned off ({})", crate::extensions::shown(&why)),
                ),
            }
            crate::extensions::refresh(profile, cx);
        })
        .ok();
    })
    .detach();
}

/// Runs a confirmed or checked package in its profile.
pub(crate) fn add(core: &AppCore, row: InstalledExtension, package: Package) {
    let stored = core.disk.extension_hosts(row.profile_id, &row.install_id).unwrap_or_default();
    let icon = package.icon_png(32).map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
    let replaced = core.tarn.borrow_mut().rt.insert(row, package, stored, icon);
    drop(replaced);
}

/// Brings the profile's views, pages and rules in line with its rows: an install turned off or
/// removed stops, its session answers and pages go.
pub(crate) fn reconcile(profile: Id, cx: &mut App) {
    let core = crate::shared::core(cx);
    let rows = core.disk.extensions(profile).unwrap_or_default();
    let effects = {
        let mut tarn = core.tarn.borrow_mut();
        if !tarn.rt.has_profile(profile) {
            return;
        }
        tarn.rt.retain(profile, &rows, &core.disk)
    };
    run(&core, effects, cx);
    sync_views(&core, profile, cx);
    apply_rules(&core, profile);
}

/// Every view of the profile in every window gets the running installs' scripts and loses the others.
fn sync_views(core: &Rc<AppCore>, profile: Id, cx: &mut App) {
    let (scripts, dropped) = {
        let mut tarn = core.tarn.borrow_mut();
        let scripts = tarn.rt.scripts(profile);
        let now: HashSet<World> = scripts.iter().map(|s| s.world.clone()).collect();
        let before = tarn.worlds.insert(profile, now.clone()).unwrap_or_default();
        (scripts, before.difference(&now).cloned().collect::<Vec<World>>())
    };
    crate::shared::each(cx, |b, _, _| {
        for view in b.profile_views(profile) {
            for world in &dropped {
                view.remove_extension_scripts(world);
            }
            for s in &scripts {
                view.add_extension_scripts(s.clone());
            }
        }
    });
}

/// The installs' network rules for the hosts they hold grants for now: a content rule list per
/// install in WebKit, the same lists evaluated per request in Chromium.
pub(crate) fn apply_rules(core: &Rc<AppCore>, profile: Id) {
    if !core.profiles.borrow().contains_key(&(false, profile)) {
        return;
    }
    let policy = policy(core, profile);
    let (lists, before) = {
        let mut tarn = core.tarn.borrow_mut();
        if !tarn.rt.has_profile(profile) {
            return;
        }
        let before = tarn.rt.update_rules(profile, &policy, Instant::now());
        if before.is_some() {
            tarn.chromium_rules.remove(&profile);
        }
        let lists = tarn.rt.rules(profile).cloned().unwrap_or_default();
        let before = before.unwrap_or_else(|| lists.clone());
        (lists, before)
    };
    #[cfg(any(windows, feature = "chromium"))]
    let all: Vec<String> = lists.values().cloned().collect();
    #[cfg(windows)]
    if !chromium_applied(core, profile)
        && let Some(rt) = core.profiles.borrow().get(&(false, profile))
    {
        rt.data.set_extension_rules(&all);
    }
    #[cfg(feature = "chromium")]
    if core.chromium_profile.get() == Some(profile) && !chromium_applied(core, profile) {
        engine::chromium::set_extension_rules(&all);
    }
    #[cfg(target_os = "macos")]
    webkit_rules(core, profile, &lists, &before);
    #[cfg(not(target_os = "macos"))]
    drop((lists, before));
}

/// Marks the profile's lists as given to Chromium; true when they already were.
#[cfg(any(windows, feature = "chromium"))]
fn chromium_applied(core: &AppCore, profile: Id) -> bool {
    !core.tarn.borrow_mut().chromium_rules.insert(profile)
}

/// A profile's data store opened (or opened again): it gets every rule list afresh.
pub(crate) fn opened(core: &Rc<AppCore>, profile: Id) {
    {
        let mut tarn = core.tarn.borrow_mut();
        tarn.rt.reset_rules(profile);
        tarn.chromium_rules.remove(&profile);
    }
    apply_rules(core, profile);
}

/// The profile whose choices Chromium reads changed (macOS): its lists go to Chromium.
#[cfg(feature = "chromium")]
pub(crate) fn chromium_owner_changed(core: &Rc<AppCore>, profile: Id) {
    core.tarn.borrow_mut().chromium_rules.remove(&profile);
    apply_rules(core, profile);
}

#[cfg(target_os = "macos")]
fn webkit_rules(core: &Rc<AppCore>, profile: Id, lists: &BTreeMap<String, String>, before: &BTreeMap<String, String>) {
    use engine::FilterLists;
    for id in before.keys().filter(|id| !lists.contains_key(*id)) {
        let group = format!("{GROUP}.{id}");
        if let Some(rt) = core.profiles.borrow().get(&(false, profile)) {
            rt.data.set_extension_rule_lists(&group, None);
        }
        FilterLists::remove(&group);
    }
    for (id, json) in lists.iter().filter(|(id, json)| before.get(*id) != Some(*json)) {
        let group = format!("{GROUP}.{id}");
        let key = shield::groups::hex(shield::lists::fingerprint(&[json]));
        let (weak, id, expected) = (Rc::downgrade(core), id.clone(), json.clone());
        FilterLists::compile(&group.clone(), &key, vec![json.clone()], move |result| {
            let Some(core) = weak.upgrade() else { return };
            let current = core.tarn.borrow().rt.rules(profile).and_then(|r| r.get(&id).cloned()) == Some(expected);
            match result {
                Ok(lists) if current => {
                    if let Some(rt) = core.profiles.borrow().get(&(false, profile)) {
                        rt.data.set_extension_rule_lists(&group, Some(&lists));
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("tarn: an extension's rules failed to compile: {e}"),
            }
        });
    }
}

/// What the router needs to know now: the profile's tabs.
fn tab_infos(core: &AppCore, profile: Id, cx: &mut App) -> Vec<TabInfo> {
    let (mut active, mut current) = (HashSet::new(), HashSet::new());
    crate::shared::each(cx, |b, window, _| {
        if let Some(key) = b.model.active.filter(|k| !k.private && b.profile_of(*k) == Some(profile)) {
            active.insert(key.id);
        }
        if window.is_window_active() && b.model.current_space().is_some_and(|s| !s.private && s.space.profile_id == profile) {
            current.extend(b.model.tabs.iter().map(|t| t.id));
        }
    });
    let spaces = core.disk.every_space(profile).unwrap_or_default();
    spaces
        .iter()
        .flat_map(|s| core.disk.tabs(s.id).unwrap_or_default())
        .map(|t| TabInfo {
            id: t.id as u64,
            url: Url::parse(&t.url).ok(),
            title: t.title,
            active: active.contains(&t.id),
            current_window: current.contains(&t.id),
        })
        .collect()
}

fn handle(core: &Rc<AppCore>, profile: Id, sender: Sender, json: &str, cx: &mut App) {
    let tabs = tab_infos(core, profile, cx);
    let policy = policy(core, profile);
    let call = Call { tabs: &tabs, store: Some(&core.disk), policy: &policy, now: Instant::now() };
    let effects = core.tarn.borrow_mut().rt.handle(profile, &sender, json, &call);
    run(core, effects, cx);
}

pub(crate) fn on_msg(msg: Msg, cx: &mut App) {
    let core = crate::shared::core(cx);
    match msg {
        Msg::Tab { profile, tab, world, frame, origin, json } => {
            handle(&core, profile, Sender { install_id: world.install_id().to_owned(), from: From::Tab { tab, frame }, origin }, &json, cx)
        }
        Msg::Page { profile, install, kind, event } => match event {
            Event::ExtensionMessage { json, .. } => handle(&core, profile, Sender { install_id: install, from: From::Page(kind), origin: None }, &json, cx),
            Event::OpenTab { url, .. } => {
                if let Some(seed) = seed(&core, profile) {
                    crate::extensions::open_tab(&seed, &url, true, cx);
                }
            }
            _ => {}
        },
        Msg::PageClosed { profile, install, kind } => {
            let effects = core.tarn.borrow_mut().rt.page_closed(profile, &install, kind);
            run(&core, effects, cx);
        }
        Msg::Answered(answer) => answered(&core, answer, cx),
        Msg::Clicked { profile, install, tab, host, x, y } => clicked(&core, profile, &install, tab, (&host, x, y), cx),
        Msg::TabClosed(tab) => {
            let (effects, profiles) = {
                let mut tarn = core.tarn.borrow_mut();
                for (profile, install, host) in tarn.asks.remove(&tab).unwrap_or_default() {
                    tarn.rt.prompt_dropped(profile, &install, &host);
                }
                (tarn.rt.tab_gone(tab, true), tarn.rt.profiles())
            };
            run(&core, effects, cx);
            for profile in profiles {
                apply_rules(&core, profile);
            }
        }
        Msg::Navigated { profile, tab } => core.tarn.borrow_mut().rt.navigated(profile, tab),
    }
}

/// The profile's Shield level changed: at Lockdown its extension pages close.
pub(crate) fn level_changed(core: &Rc<AppCore>, profile: Id, cx: &mut App) {
    let policy = policy(core, profile);
    let effects = core.tarn.borrow_mut().rt.level_changed(profile, &policy);
    run(core, effects, cx);
}

/// Forgets the install's site answers, stored and of this session: it asks again on every site.
pub(crate) fn forget_answers(core: &Rc<AppCore>, profile: Id, install: &str) -> Result<(), store::Error> {
    core.tarn.borrow_mut().rt.forget_answers(&core.disk, profile, install)?;
    apply_rules(core, profile);
    Ok(())
}

fn answered(core: &Rc<AppCore>, answer: Answer, cx: &mut App) {
    core.tarn.borrow_mut().rt.answer(&answer, &core.disk, Instant::now());
    apply_rules(core, answer.profile);
    if answer.allow
        && let Some(tab) = answer.tab
    {
        crate::shared::each(cx, |b, _, _| {
            b.reload_view(TabKey { private: false, id: tab as Id }, false);
        });
    }
}

/// A click on the install's toolbar button at `x, y` of `host`: its popup opens, or closes when
/// it is open; without one the router tells the background page.
fn clicked(core: &Rc<AppCore>, profile: Id, install: &str, tab: Option<u64>, (host, x, y): (&Host, f64, f64), cx: &mut App) {
    let tabs = tab_infos(core, profile, cx);
    let Some(info) = tab.and_then(|t| tabs.iter().find(|i| i.id == t)).cloned() else { return };
    let open = core.tarn.borrow_mut().rt.take_page(profile, install, PageKind::Popup);
    if let Some(page) = open {
        page.close_popup();
        return;
    }
    let policy = policy(core, profile);
    let call = Call { tabs: &tabs, store: Some(&core.disk), policy: &policy, now: Instant::now() };
    let effects = core.tarn.borrow_mut().rt.clicked(profile, install, &info, &call);
    run(core, effects, cx);
    let popup = core.tarn.borrow().rt.page_url(profile, install, PageKind::Popup, &policy);
    if let Some(url) = popup
        && let Some(page) = open_page(core, profile, install, PageKind::Popup, host)
    {
        page.load(&url);
        page.show_popup(host, x, y);
        let replaced = core.tarn.borrow_mut().rt.page_opened(profile, install, PageKind::Popup, page, Instant::now());
        drop(replaced);
    }
}

/// One of the install's own pages, with the profile's built-in rules.
fn open_page(core: &Rc<AppCore>, profile: Id, install: &str, kind: PageKind, host: &Host) -> Option<ExtensionPage> {
    let level = policy(core, profile).level;
    let (spec, sender) = {
        let tarn = core.tarn.borrow();
        let l = tarn.rt.loaded(profile, install)?;
        let inspectable = core.profiles.borrow().get(&(false, profile)).is_some_and(|rt| rt.profile.is_dev);
        let spec = PageSpec {
            world: l.package.world.clone(),
            origin: l.package.origin_id.clone(),
            bootstrap: l.package.bootstrap(true),
            resources: tarn.rt.resolver(profile)?.handler(),
            inspectable,
        };
        (spec, tarn.sender.clone()?)
    };
    let (events, closing, id) = (sender.clone(), sender, install.to_owned());
    let sink: Rc<dyn Fn(Event)> = Rc::new(move |event| drop(events.unbounded_send(Msg::Page { profile, install: id.clone(), kind, event })));
    let id = install.to_owned();
    let closed: Rc<dyn Fn()> = Rc::new(move || drop(closing.unbounded_send(Msg::PageClosed { profile, install: id.clone(), kind })));
    let profiles = core.profiles.borrow();
    let rt = profiles.get(&(false, profile))?;
    ExtensionPage::new(host, &rt.data, level, spec, sink, closed)
}

/// Carries out the runtime's effects.
fn run(core: &Rc<AppCore>, effects: Effects, cx: &mut App) {
    for effect in effects {
        match effect {
            Effect::Post { profile, install_id, target, json } => post(core, profile, &install_id, target, &json, cx),
            Effect::StartBackground { profile, install_id } => {
                let Some(host) = any_host(cx) else { continue };
                let url = core.tarn.borrow().rt.page_url(profile, &install_id, PageKind::Background, &policy(core, profile));
                let Some(url) = url else { continue };
                if let Some(page) = open_page(core, profile, &install_id, PageKind::Background, &host) {
                    page.load(&url);
                    let replaced = core.tarn.borrow_mut().rt.page_opened(profile, &install_id, PageKind::Background, page, Instant::now());
                    drop(replaced);
                }
            }
            Effect::Close(page) => drop(page),
            Effect::OpenTab { profile, url, active } => {
                if let Some(seed) = seed(core, profile) {
                    crate::extensions::open_tab(&seed, &url, active, cx);
                }
            }
            Effect::ActionChanged(_) => cx.refresh_windows(),
        }
    }
}

fn any_host(cx: &mut App) -> Option<Host> {
    let mut found = None;
    crate::shared::each(cx, |b, _, _| {
        if found.is_none() {
            found = b.host.clone();
        }
    });
    found
}

fn post(core: &AppCore, profile: Id, install: &str, target: Target, json: &str, cx: &mut App) {
    let Some(world) = World::for_install(install) else { return };
    if let Target::Page(kind) = target {
        if let Some(page) = core.tarn.borrow().rt.page(profile, install, kind) {
            page.post(json);
        }
        return;
    }
    crate::shared::each(cx, |b, _, _| {
        for (key, view) in b.profile_views_keyed(profile) {
            match target {
                Target::Frame { tab, frame } if key.id as u64 == tab => view.post_extension_message(&world, Some(frame), json),
                Target::Tab(tab) if key.id as u64 == tab => view.post_extension_message(&world, None, json),
                Target::AllTabs => view.post_extension_message(&world, None, json),
                _ => {}
            }
        }
    });
}

/// Every `TICK`: stops idle background pages and, once a Once grant ran out, applies the rule
/// lists again.
pub(crate) fn tick(cx: &mut App) {
    let core = crate::shared::core(cx);
    let (effects, expired) = core.tarn.borrow_mut().rt.tick(Instant::now());
    run(&core, effects, cx);
    if expired {
        let profiles = core.tarn.borrow().rt.profiles();
        for profile in profiles {
            apply_rules(&core, profile);
        }
    }
}

impl Browser {
    /// Raises the site prompt for the hosts the gate asked about in this tab, once its page shows.
    pub(crate) fn tarn_committed(&mut self, key: TabKey, url: &Url, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        if key.private {
            return;
        }
        let Some(profile) = self.profile_of(key) else { return };
        let host = url.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase());
        let (asks, sender) = {
            let mut tarn = self.core.tarn.borrow_mut();
            tarn.send(Msg::Navigated { profile, tab: key.id as u64 });
            let (asks, dropped): (Vec<_>, Vec<_>) =
                tarn.asks.remove(&(key.id as u64)).unwrap_or_default().into_iter().partition(|(_, _, h)| Some(h) == host.as_ref());
            for (profile, install, asked) in dropped {
                tarn.rt.prompt_dropped(profile, &install, &asked);
            }
            (asks, tarn.sender.clone())
        };
        let Some(sender) = sender else { return };
        for (profile, install, asked) in asks {
            let Some((name, _)) = self.core.tarn.borrow().loaded(profile, &install) else { continue };
            let (sender, id, h, tab) = (sender.clone(), install.clone(), asked.clone(), key.id as u64);
            let prompt = Prompt::extension_access(name, install, asked, move |allow, remember| {
                if let Some(allow) = allow {
                    let answer = Answer { profile, install_id: id, host: h, allow, remember, tab: Some(tab) };
                    sender.unbounded_send(Msg::Answered(answer)).ok();
                }
            });
            self.queue_prompt(key, prompt, window, cx);
        }
    }

    pub(crate) fn tarn_clicked(&mut self, install: String, x: f64, y: f64) {
        let (Some(host), Some(profile)) = (self.host.clone(), self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id)) else { return };
        let tab = self.model.active.filter(|k| !k.private).map(|k| k.id as u64);
        self.core.tarn.borrow().send(Msg::Clicked { profile, install, tab, host, x, y });
    }
}

impl Browser {
    /// This window's live views of the profile's persistent tabs.
    pub(crate) fn profile_views(&self, profile: Id) -> Vec<Rc<engine::WebView>> {
        self.profile_views_keyed(profile).into_iter().map(|(_, v)| v).collect()
    }

    pub(crate) fn profile_views_keyed(&self, profile: Id) -> Vec<(TabKey, Rc<engine::WebView>)> {
        if self.model.agent_profile.is_some() {
            return Vec::new();
        }
        self.views.iter().filter(|(k, _)| !k.private && self.profile_of(**k) == Some(profile)).map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Gives this window's view of `key` the gate and the profile's Tarn extensions.
    pub(crate) fn attach_tarn(&self, key: TabKey) {
        let (Some(view), Some(profile)) = (self.views.get(&key).filter(|_| self.model.agent_profile.is_none()), self.profile_of(key)) else { return };
        let url = view.url().or_else(|| self.tab_url(key).and_then(|u| Url::parse(&u).ok()));
        let level = self.core.level(key.private, profile, url.as_ref()).unwrap_or(ShieldLevel::Lockdown);
        attach(&self.core, key, profile, view.as_ref(), level);
    }
}

/// The toolbar buttons of the Tarn extensions running in the window's profile: icon, title,
/// badge; a click opens the popup or tells the background page.
pub(crate) fn buttons(b: &Browser, cx: &mut gpui::Context<Browser>) -> Vec<gpui::AnyElement> {
    let Some(profile) = b.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id) else { return Vec::new() };
    let tab = b.model.active.filter(|k| !k.private).map(|k| k.id as u64);
    let buttons: Vec<Action<Arc<Image>>> = b.core.tarn.borrow().rt.actions(profile, tab);
    buttons
        .into_iter()
        .enumerate()
        .map(|(ix, button)| {
            let element = match button.icon {
                Some(icon) => div()
                    .id(("tarn-extension", ix))
                    .w(px(28.))
                    .h(px(28.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(Theme::radius_row()))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::hover()))
                    .active(|s| s.bg(Theme::pressed()))
                    .child(img(icon).w(px(16.)).h(px(16.))),
                None => icon_button(("tarn-extension", ix), Icon::Braces, true),
            };
            let badge = button.state.badge_text.clone();
            let color = button.state.badge_color.map_or(Theme::accent(), |[r, g, bl, a]| Rgba {
                r: f32::from(r) / 255.,
                g: f32::from(g) / 255.,
                b: f32::from(bl) / 255.,
                a: f32::from(a) / 255.,
            });
            let install = button.install_id.clone();
            element
                .relative()
                .tooltip(tooltip(button.state.title.clone(), None))
                .when(!badge.is_empty(), |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom(px(1.))
                            .right(px(1.))
                            .px(px(2.))
                            .rounded(px(3.))
                            .bg(color)
                            .text_color(Theme::on_fill(color))
                            .text_size(px(TEXT_TINY))
                            .child(badge),
                    )
                })
                .on_click(cx.listener(move |b, event: &ClickEvent, _, _| {
                    let at = event.position();
                    b.tarn_clicked(install.clone(), f64::from(f32::from(at.x)), f64::from(f32::from(at.y)));
                }))
                .into_any_element()
        })
        .collect()
}
