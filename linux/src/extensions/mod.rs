pub mod popup;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use browser_core::Core;
use browser_core::extensions::{EVERY_SITE_SETTING, every_site_setting};
use browser_core::tarn_ext::grants::Answer;
use browser_core::tarn_ext::router::{From, Sender, TabInfo, Target};
use browser_core::tarn_ext::runtime::{self, Action, Call, Effect, Policy};
use browser_core::tarn_ext::{LoadError, Package};
use engine::tarn_ext::{FrameId, FrameNav, PageKind, World};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt;
use shield::ShieldLevel;
use store::{Id, InstalledExtension};
use url::Url;

use crate::data::{Data, StoreKey};
use crate::webview::ext::{self as view, Event, PageSpec};

#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Ask { profile: Id, tab: u64, install: String, host: String },
    OpenTab { profile: Id, url: Url, active: bool },
    Changed,
    PageClosed(u64),
}

/// An extension page the runtime keeps open; it is made and closed on the GTK thread.
#[derive(Debug)]
pub struct Page(u64);

impl Drop for Page {
    fn drop(&mut self) {
        view::close_page(self.0);
    }
}

static NEXT_PAGE: AtomicU64 = AtomicU64::new(1);

type Runtime = runtime::Runtime<Page, String>;
type Effects = Vec<Effect<Page>>;
pub type Button = Action<String>;

#[derive(Default)]
struct State {
    core: Option<Rc<Core<Data>>>,
    rt: Runtime,
    uuids: HashMap<Id, String>,
    active: Option<u64>,
    notes: Option<UnboundedSender<Note>>,
    #[cfg(test)]
    sent: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asking {
    pub name: String,
    pub badge: String,
}

const ICON_PIXELS: u32 = 32;

thread_local! {
    static RT: RefCell<State> = RefCell::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    RT.with_borrow_mut(f)
}

fn tab_prop(tab: u64) -> String {
    format!("false-{tab}")
}

fn note(rt: &mut State, n: Note) {
    #[cfg(test)]
    rt.sent.push(n.clone());
    if let Some(tx) = &rt.notes {
        drop(tx.unbounded_send(n));
    }
}

fn policy(core: &Core<Data>, profile: Id) -> Policy {
    core.extension_policy(profile, every_site_setting(core.disk.setting(EVERY_SITE_SETTING).ok().flatten().as_deref()))
}

pub fn ui_locales() -> Vec<String> {
    let lang = std::env::var("LANGUAGE").ok().or_else(|| std::env::var("LANG").ok()).unwrap_or_default();
    let mut out: Vec<String> =
        lang.split(':').map(|l| l.split(['.', '@']).next().unwrap_or_default().replace('_', "-")).filter(|l| !l.is_empty() && l != "C").collect();
    out.push("en".to_owned());
    out
}

impl State {
    fn core(&self) -> Option<Rc<Core<Data>>> {
        self.core.clone()
    }

    fn gate(&mut self, profile: Id, tab: u64, world: &World, nav: &FrameNav) -> Vec<usize> {
        let Some(core) = self.core() else { return Vec::new() };
        let view_level = core.level(false, profile, Some(nav.top)).unwrap_or(ShieldLevel::Lockdown);
        let decision = self.rt.gate(profile, world, nav, &policy(&core, profile), view_level, Instant::now());
        if let Some(host) = decision.ask {
            note(self, Note::Ask { profile, tab, install: world.install_id().to_owned(), host });
        }
        decision.run
    }

    fn profile_of(&self, store: &str) -> Option<Id> {
        let core = self.core()?;
        let profiles = core.profiles.borrow();
        let mut users = profiles.iter().filter(|((private, _), rt)| !private && rt.profile.data_store_uuid == store).map(|((_, id), _)| *id);
        let only = users.next().filter(|_| users.next().is_none())?;
        self.uuids.get(&only).is_some_and(|own| own == store).then_some(only)
    }

    fn rule_ids(&self, profile: Id) -> Vec<(String, String)> {
        self.rt
            .rules(profile)
            .into_iter()
            .flatten()
            .map(|(id, json)| {
                let hash = shield::groups::hex(shield::lists::fingerprint(&[json]));
                (format!("tarn-ext-{id}-{}", &hash[..16]), json.clone())
            })
            .collect()
    }

    /// Hands the profile's worlds and rules to its views.
    fn push(&mut self, profile: Id) {
        let Some(store) = self.uuids.get(&profile).cloned() else { return };
        view::set_scripts(store, self.rt.scripts(profile).iter().map(|w| (**w).clone()).collect());
        self.rt.reset_rules(profile);
        self.push_rules(profile);
    }

    /// The profile's network rules where its installs hold grants now; sent when they changed.
    fn push_rules(&mut self, profile: Id) {
        let (Some(core), Some(store)) = (self.core(), self.uuids.get(&profile).cloned()) else { return };
        if self.rt.update_rules(profile, &policy(&core, profile), Instant::now()).is_some() {
            view::set_rules(store, self.rule_ids(profile));
        }
    }

    fn push_all_rules(&mut self) {
        self.rt.profiles().into_iter().for_each(|p| self.push_rules(p));
    }

    fn tabs(&self, profile: Id) -> Vec<TabInfo> {
        let Some(core) = self.core() else { return Vec::new() };
        let spaces = core.disk.every_space(profile).unwrap_or_default();
        let tabs: Vec<store::Tab> = spaces.iter().flat_map(|s| core.disk.tabs(s.id).unwrap_or_default()).collect();
        let active_space = tabs.iter().find(|t| Some(t.id as u64) == self.active).map(|t| t.space_id);
        tabs.into_iter()
            .map(|t| TabInfo {
                id: t.id as u64,
                url: Url::parse(&t.url).ok(),
                title: t.title,
                active: Some(t.id as u64) == self.active,
                current_window: Some(t.space_id) == active_space,
            })
            .collect()
    }

    fn call(&mut self, profile: Id, f: impl FnOnce(&mut Runtime, &Call) -> Effects) -> Effects {
        let Some(core) = self.core() else { return Vec::new() };
        let tabs = self.tabs(profile);
        let policy = policy(&core, profile);
        f(&mut self.rt, &Call { tabs: &tabs, store: Some(&core.disk), policy: &policy, now: Instant::now() })
    }

    fn lists(&self) -> Vec<String> {
        self.core().map(|c| shield::groups::switched_on(|k| c.disk.setting(k).ok().flatten())).unwrap_or_default()
    }

    fn page_spec(&self, profile: Id, install_id: &str, kind: PageKind) -> Option<PageSpec> {
        let core = self.core()?;
        let policy = policy(&core, profile);
        let url = self.rt.page_url(profile, install_id, kind, &policy)?;
        let l = self.rt.loaded(profile, install_id)?;
        let key = core.profiles.borrow().get(&(false, profile)).map(|rt| rt.data.key.clone());
        Some(PageSpec {
            key: key.or_else(|| Some(StoreKey::Persistent(self.uuids.get(&profile)?.clone())))?,
            level: policy.level,
            lists: self.lists(),
            origin: l.package.origin_id.clone(),
            url,
            bootstrap: l.package.bootstrap(true),
            resources: self.rt.resolver(profile)?.handler(),
        })
    }
}

/// Carries out the runtime's effects, outside its borrow.
fn carry_out(effects: Effects) {
    for effect in effects {
        match effect {
            Effect::Post { profile, install_id, target, json } => {
                let Some(world) = World::for_install(&install_id) else { continue };
                match target {
                    Target::Frame { tab, frame } => view::post(Some(tab_prop(tab)), world, Some(frame), json),
                    Target::Tab(tab) => view::post(Some(tab_prop(tab)), world, None, json),
                    Target::AllTabs => view::post(None, world, None, json),
                    Target::Page(kind) => {
                        if let Some(page) = with(|rt| rt.rt.page(profile, &install_id, kind).map(|p| p.0)) {
                            view::post_page(page, json);
                        }
                    }
                }
            }
            Effect::StartBackground { profile, install_id } => drop(start_page(profile, &install_id, PageKind::Background)),
            Effect::Close(page) => {
                let id = page.0;
                drop(page);
                with(|rt| note(rt, Note::PageClosed(id)));
            }
            Effect::OpenTab { profile, url, active } => with(|rt| note(rt, Note::OpenTab { profile, url, active })),
            Effect::ActionChanged(_) => with(|rt| note(rt, Note::Changed)),
        }
    }
}

fn start_page(profile: Id, install_id: &str, kind: PageKind) -> Option<u64> {
    let spec = with(|rt| rt.page_spec(profile, install_id, kind))?;
    let id = NEXT_PAGE.fetch_add(1, Ordering::Relaxed);
    let returned = with(|rt| rt.rt.page_opened(profile, install_id, kind, Page(id), Instant::now()));
    let kept = returned.as_ref().is_none_or(|p| p.0 != id);
    if kept {
        view::open_page(id, spec);
    }
    if let Some(old) = returned.filter(|p| p.0 != id) {
        let old_id = old.0;
        drop(old);
        with(|rt| note(rt, Note::PageClosed(old_id)));
    }
    kept.then_some(id)
}

/// Connects the views on the GTK thread to this thread's runtime; their questions and page
/// messages arrive on the receiver, for `on_event`.
pub fn connect() -> UnboundedReceiver<Event> {
    let (tx, rx) = unbounded();
    view::connect(tx);
    rx
}

pub fn on_event(event: Event) {
    match event {
        Event::Frame(frame) => {
            let view::Frame { request, tab, store, worlds, url, top, main } = *frame;
            let nav = FrameNav { url: &url, top: &top, main_frame: main };
            let tab: Option<u64> = tab.strip_prefix("false-").and_then(|t| t.parse().ok());
            let runs = with(|rt| {
                let (Some(profile), Some(tab)) = (rt.profile_of(&store), tab) else { return Vec::new() };
                worlds
                    .into_iter()
                    .map(|w| {
                        let run = rt.gate(profile, tab, &w, &nav);
                        (w, run)
                    })
                    .collect()
            });
            view::answer(request, runs);
        }
        Event::PageMessage { page, json } => {
            let Some((profile, install_id, kind)) = page_of(page) else { return };
            let sender = Sender { install_id, from: From::Page(kind), origin: None };
            carry_out(with(|rt| rt.call(profile, |rt, call| rt.handle(profile, &sender, &json, call))));
        }
        Event::PageFailed { page } => {
            let Some((profile, install_id, kind)) = page_of(page) else { return };
            eprintln!("tarn: a page of extension {install_id} could not open");
            carry_out(with(|rt| rt.rt.page_closed(profile, &install_id, kind)));
        }
    }
}

fn page_of(page: u64) -> Option<(Id, String, PageKind)> {
    with(|rt| rt.rt.pages().find(|(_, _, _, p)| p.0 == page).map(|(profile, install, kind, _)| (profile, install.to_owned(), kind)))
}

/// Runs `future` on this thread for as long as it needs, whatever view started it.
fn detached(future: impl Future<Output = ()> + 'static) {
    nsh_core::prelude::Owner::detached().run(|| drop(nsh_core::prelude::spawn(future)));
}

pub fn init(core: Rc<Core<Data>>) -> UnboundedReceiver<Note> {
    let (tx, rx) = unbounded();
    let profiles: Vec<Id> = core.disk.profiles().unwrap_or_default().into_iter().map(|p| p.id).collect();
    with(|rt| {
        rt.core = Some(core);
        rt.notes = Some(tx);
    });
    let mut events = connect();
    detached(async move {
        while let Some(event) = events.next().await {
            on_event(event);
        }
    });
    profiles.into_iter().for_each(reload);
    rx
}

type Results = Vec<(InstalledExtension, Result<Package, LoadError>)>;

pub fn reload(profile: Id) {
    let Some(core) = with(|rt| rt.core()) else { return };
    let Ok(row) = core.disk.profile(profile) else { return };
    if row.is_private {
        return;
    }
    let rows: Vec<InstalledExtension> = core.disk.extensions(profile).unwrap_or_default().into_iter().filter(|r| r.enabled).collect();
    let seed = row.data_store_uuid.clone();
    if rows.is_empty() {
        return apply(profile, &seed, Vec::new());
    }
    let locales = ui_locales();
    detached(async move {
        let worker = seed.clone();
        let results = nsh_core::prelude::blocking(move || runtime::load_rows(rows, runtime::row_dir, &worker, &locales)).await.unwrap_or_default();
        apply(profile, &seed, results);
    });
}

pub fn restart(profile: Id) {
    if let Some(seed) = with(|rt| rt.uuids.get(&profile).cloned()) {
        apply(profile, &seed, Vec::new());
    }
    reload(profile);
}

pub fn unload(profile: Id, seed: &str) {
    apply(profile, seed, Vec::new());
}

fn icon_file(package: &Package) -> Option<String> {
    let bytes = package.icon_png(ICON_PIXELS)?;
    let dir = crate::data::root().join("extension-icons");
    let path = dir.join(format!("{}.png", package.origin_id));
    std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, bytes)).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn apply(profile: Id, seed: &str, results: Results) {
    let Some(core) = with(|rt| rt.core()) else { return };
    let (effects, failed) = with(|rt| {
        rt.uuids.insert(profile, seed.to_owned());
        let done = rt.rt.replace(profile, results, &core.disk, icon_file);
        rt.push(profile);
        note(rt, Note::Changed);
        done
    });
    for (row, error) in failed {
        match error {
            LoadError::Changed => eprintln!("tarn: the files of extension {} changed on disk; it was turned off", row.install_id),
            LoadError::Refused(why) => eprintln!("tarn: extension {} was not loaded: {why}", row.install_id),
        }
    }
    carry_out(effects);
}

pub fn message(profile: Id, tab: u64, world: &str, json: &str) {
    let Some(world) = World::parse(world) else { return };
    let sender = Sender { install_id: world.install_id().to_owned(), from: From::Tab { tab, frame: FrameId::MAIN }, origin: None };
    carry_out(with(|rt| rt.call(profile, |rt, call| rt.handle(profile, &sender, json, call))));
}

pub fn activated(tab: Option<u64>) {
    with(|rt| rt.active = tab);
}

pub fn tab_closed(tab: u64) {
    carry_out(with(|rt| {
        let effects = rt.rt.tab_gone(tab, true);
        rt.push_all_rules();
        effects
    }));
}

pub fn prompt_dropped(profile: Id, install_id: &str, host: &str) {
    with(|rt| rt.rt.prompt_dropped(profile, install_id, host));
}

pub fn navigated(profile: Id, tab: u64) {
    with(|rt| rt.rt.navigated(profile, tab));
}

pub fn tick() {
    let effects = with(|rt| {
        if !rt.rt.has_installs() {
            return Vec::new();
        }
        let (effects, expired) = rt.rt.tick(Instant::now());
        if expired {
            rt.push_all_rules();
        }
        effects
    });
    carry_out(effects);
}

pub fn buttons(profile: Id, tab: Option<u64>) -> Vec<Button> {
    with(|rt| rt.rt.actions(profile, tab))
}

pub fn clicked(profile: Id, install_id: &str, tab: u64) -> bool {
    let (popup, effects) = with(|rt| {
        let popup = rt.rt.loaded(profile, install_id).is_some_and(|l| l.package.popup_url().is_some());
        let Some(info) = rt.tabs(profile).into_iter().find(|t| t.id == tab) else { return (false, Vec::new()) };
        (popup, rt.call(profile, |rt, call| rt.clicked(profile, install_id, &info, call)))
    });
    carry_out(effects);
    popup
}

/// Opens the install's popup; the GTK thread shows the returned page with `show_page`.
pub fn open_popup(profile: Id, install_id: &str) -> Option<u64> {
    start_page(profile, install_id, PageKind::Popup)
}

pub fn popup_closed(profile: Id, install_id: &str, page: u64) {
    carry_out(with(|rt| {
        if rt.rt.page(profile, install_id, PageKind::Popup).is_none_or(|p| p.0 != page) {
            return Vec::new();
        }
        rt.rt.page_closed(profile, install_id, PageKind::Popup)
    }));
}

/// Forgets the install's site answers.
pub fn forget_answers(profile: Id, install_id: &str) -> Result<(), String> {
    with(|rt| {
        let core = rt.core().ok_or("Extensions are not running.")?;
        let forgot = rt.rt.forget_answers(&core.disk, profile, install_id).map_err(|e| e.to_string());
        rt.push_rules(profile);
        forgot
    })
}

/// The profile's Shield level or a site's level changed: its installs' rules are worked out
/// again, and at a Lockdown profile its extension pages close.
pub fn levels_changed(profile: Id) {
    carry_out(with(|rt| {
        rt.push_rules(profile);
        let Some(core) = rt.core() else { return Vec::new() };
        rt.rt.level_changed(profile, &policy(&core, profile))
    }));
}

pub fn asking(profile: Id, install_id: &str) -> Option<Asking> {
    with(|rt| {
        let l = rt.rt.loaded(profile, install_id)?;
        Some(Asking { name: browser_core::theme_fetch::shown(&l.package.name), badge: browser_core::extensions::badge(&l.row.source) })
    })
}

pub fn answer(profile: Id, install_id: &str, host: &str, allow: bool, remember: bool, tab: u64) -> bool {
    let answer = Answer { profile, install_id: install_id.to_owned(), host: host.to_owned(), allow, remember, tab: Some(tab) };
    with(|rt| {
        if let Some(core) = rt.core() {
            rt.rt.answer(&answer, &core.disk, Instant::now());
            rt.push_rules(profile);
        }
    });
    allow
}

#[cfg(test)]
pub mod tests;
