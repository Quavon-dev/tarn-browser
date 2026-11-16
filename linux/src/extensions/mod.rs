pub mod popup;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use browser_core::Core;
use browser_core::extensions::{EVERY_SITE_SETTING, every_site_setting};
use browser_core::tarn_ext::grants::Answer;
use browser_core::tarn_ext::router::{From, Sender, TabInfo, Target};
use browser_core::tarn_ext::runtime::{self, Action, Call, Effect, Policy};
use browser_core::tarn_ext::{LoadError, Package};
use engine::tarn_ext::{FrameId, FrameNav, PageKind, World};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use shield::ShieldLevel;
use store::{Id, InstalledExtension};
use url::Url;

use crate::data::{Data, StoreKey};
use crate::webview::ext::{self as view, Attach, PageSpec, PageView, ViewInfo};

#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Ask { profile: Id, tab: u64, install: String, host: String },
    OpenTab { url: Url, active: bool },
    Changed,
}

type Runtime = runtime::Runtime<Rc<PageView>, String>;
type Effects = Vec<Effect<Rc<PageView>>>;
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

    fn attach(&self, info: &ViewInfo) -> Option<(Id, Attach)> {
        let StoreKey::Persistent(_) = info.key else { return None };
        let tab: u64 = info.tab.strip_prefix("false-")?.parse().ok()?;
        let core = self.core()?;
        let space = core.disk.tab(tab as Id).ok()?.space_id;
        let profile = self.rt.profiles().into_iter().find(|id| core.disk.spaces(*id).unwrap_or_default().iter().any(|s| s.id == space))?;
        let policy = policy(&core, profile);
        if !policy.context(info.level).runs_extensions() || self.rt.installs(profile).next().is_none() {
            return None;
        }
        let rules = self
            .rt
            .rule_lists(profile, &policy, info.level, Instant::now())
            .into_iter()
            .map(|(id, json)| {
                let hash = shield::groups::hex(shield::lists::fingerprint(&[&json]));
                (format!("tarn-ext-{id}-{}", &hash[..16]), json)
            })
            .collect();
        let gate: engine::tarn_ext::Gate = Rc::new(move |world: &World, nav: &FrameNav| with(|rt| rt.gate(profile, tab, world, nav)));
        Some((profile, Attach { gate, worlds: self.rt.scripts(profile), rules }))
    }

    fn tabs(&self, profile: Id) -> Vec<TabInfo> {
        let Some(core) = self.core() else { return Vec::new() };
        let spaces = core.disk.spaces(profile).unwrap_or_default();
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
        let l = self.rt.loaded(profile, install_id)?;
        let url = match kind {
            PageKind::Background => l.package.background_url()?,
            PageKind::Popup => l.package.popup_url()?,
        };
        let key = core.profiles.borrow().get(&(false, profile)).map(|rt| rt.data.key.clone());
        Some(PageSpec {
            key: key.or_else(|| Some(StoreKey::Persistent(self.uuids.get(&profile)?.clone())))?,
            level: policy(&core, profile).level,
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
                    Target::Frame { tab, frame } => view::post(Some(&tab_prop(tab)), &world, Some(frame), &json),
                    Target::Tab(tab) => view::post(Some(&tab_prop(tab)), &world, None, &json),
                    Target::AllTabs => view::post(None, &world, None, &json),
                    Target::Page(kind) => {
                        if let Some(page) = with(|rt| rt.rt.page(profile, &install_id, kind).cloned()) {
                            page.post(&json);
                        }
                    }
                }
            }
            Effect::StartBackground { profile, install_id } => start_page(profile, &install_id, PageKind::Background, |_| {}),
            Effect::Close(page) => drop(page),
            Effect::OpenTab { url, active, .. } => with(|rt| note(rt, Note::OpenTab { url, active })),
            Effect::ActionChanged(_) => with(|rt| note(rt, Note::Changed)),
        }
    }
}

fn start_page(profile: Id, install_id: &str, kind: PageKind, shown: impl FnOnce(Rc<PageView>) + 'static) {
    let Some(spec) = with(|rt| rt.page_spec(profile, install_id, kind)) else { return };
    let (install, message_install) = (install_id.to_owned(), install_id.to_owned());
    let message = move |json: String| page_message(profile, &message_install, kind, &json);
    view::open_page(spec, message, move |page| {
        let returned = with(|rt| rt.rt.page_opened(profile, &install, kind, page.clone(), Instant::now()));
        let kept = !returned.as_ref().is_some_and(|r| Rc::ptr_eq(r, &page));
        drop(returned);
        if kept {
            shown(page);
        }
    });
}

pub fn provider() -> view::Provider {
    Rc::new(|info: &ViewInfo| with(|rt| rt.attach(info)).map(|(_, a)| a))
}

pub fn init(core: Rc<Core<Data>>) -> UnboundedReceiver<Note> {
    let (tx, rx) = unbounded();
    let profiles: Vec<Id> = core.disk.profiles().unwrap_or_default().into_iter().map(|p| p.id).collect();
    with(|rt| {
        rt.core = Some(core);
        rt.notes = Some(tx);
    });
    view::set_provider(provider());
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
    webkit6::glib::MainContext::default().spawn_local(async move {
        let worker = seed.clone();
        let results = webkit6::gio::spawn_blocking(move || runtime::load_rows(rows, runtime::row_dir, &worker, &locales)).await.unwrap_or_default();
        apply(profile, &seed, results);
    });
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
    view::refresh();
}

pub fn message(profile: Id, tab: u64, world: &str, json: &str) {
    let Some(world) = World::parse(world) else { return };
    let sender = Sender { install_id: world.install_id().to_owned(), from: From::Tab { tab, frame: FrameId::MAIN }, origin: None };
    carry_out(with(|rt| rt.call(profile, |rt, call| rt.handle(profile, &sender, json, call))));
}

fn page_message(profile: Id, install_id: &str, kind: PageKind, json: &str) {
    let sender = Sender { install_id: install_id.to_owned(), from: From::Page(kind), origin: None };
    carry_out(with(|rt| rt.call(profile, |rt, call| rt.handle(profile, &sender, json, call))));
}

pub fn activated(tab: Option<u64>) {
    with(|rt| rt.active = tab);
}

pub fn tab_closed(tab: u64) {
    carry_out(with(|rt| rt.rt.tab_gone(tab, true)));
}

pub fn navigated(profile: Id, tab: u64) {
    with(|rt| rt.rt.navigated(profile, tab));
}

pub fn tick() {
    let (effects, expired) = with(|rt| rt.rt.tick(Instant::now()));
    carry_out(effects);
    if expired {
        view::refresh();
    }
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

pub fn open_popup(profile: Id, install_id: &str, shown: impl FnOnce(Rc<PageView>) + 'static) {
    start_page(profile, install_id, PageKind::Popup, shown);
}

pub fn popup_closed(profile: Id, install_id: &str) {
    carry_out(with(|rt| rt.rt.page_closed(profile, install_id, PageKind::Popup)));
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
        }
    });
    view::refresh();
    allow
}

#[cfg(test)]
pub mod tests;
