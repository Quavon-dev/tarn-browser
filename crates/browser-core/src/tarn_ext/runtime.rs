//! The part of running Tarn extensions every shell shares: the checked installs of each profile,
//! the session's site answers, the router and the install pages the shell opened. A shell reads
//! its state, forwards what the engine reports and carries out the returned `Effect`s.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use engine::tarn_ext::{FrameNav, PageKind, World, WorldScripts};
use shield::ShieldLevel;
use store::{Id, InstalledExtension, Store};
use url::Url;

use super::grants::{Answer, Install};
use super::router::{ActionState, Env, From, Out, Router, Sender, TabInfo, Target};
use super::{Access, Context, FrameDecision, LoadError, Package, Resolver, Session};

/// What the shell knows about a persistent profile: the global "every site" switch, the hosts set
/// to Lockdown and the profile's Shield level.
#[derive(Clone, Debug)]
pub struct Policy {
    pub every_site: bool,
    pub lockdown_hosts: Vec<String>,
    pub level: ShieldLevel,
}

impl Policy {
    pub fn context(&self, view_level: ShieldLevel) -> Context {
        Context { private: false, burner: false, profile_level: self.level, view_level }
    }

    /// Whether `page` is on a host set to Lockdown, where no frame of it runs extensions.
    pub fn locks(&self, page: &Url) -> bool {
        let host = page.host_str().unwrap_or_default().trim_end_matches('.').to_ascii_lowercase();
        self.lockdown_hosts.iter().any(|h| h.trim_end_matches('.').eq_ignore_ascii_case(&host))
    }
}

impl<D: crate::WebsiteData> crate::Core<D> {
    /// The extension policy of a persistent profile: its Lockdown hosts and level while it is
    /// open, else its stored level; Lockdown while its state is being changed.
    pub fn extension_policy(&self, profile: Id, every_site: bool) -> Policy {
        let locked = Policy { every_site, lockdown_hosts: Vec::new(), level: ShieldLevel::Lockdown };
        let Ok(profiles) = self.profiles.try_borrow() else { return locked };
        let Some(rt) = profiles.get(&(false, profile)) else {
            return Policy { level: self.disk.profile(profile).map_or(ShieldLevel::Lockdown, |p| p.shield_level), ..locked };
        };
        let Ok(overrides) = rt.overrides.try_borrow() else { return locked };
        let lockdown_hosts = overrides.iter().filter(|(_, level)| **level == ShieldLevel::Lockdown).map(|(host, _)| host.clone()).collect();
        Policy { every_site, lockdown_hosts, level: rt.profile.shield_level }
    }
}

/// What the shell knows at the moment of a message or click.
pub struct Call<'a> {
    pub tabs: &'a [TabInfo],
    pub store: Option<&'a Store>,
    pub policy: &'a Policy,
    pub now: Instant,
}

pub struct Loaded<I> {
    pub package: Rc<Package>,
    pub scripts: Rc<WorldScripts>,
    pub row: InstalledExtension,
    pub stored: Vec<(String, bool)>,
    pub icon: Option<I>,
}

impl<I> Loaded<I> {
    fn install<'a>(&'a self, policy: &'a Policy) -> Install<'a> {
        Install {
            row: &self.row,
            manifest: &self.package.manifest,
            every_site_pref: policy.every_site,
            stored: &self.stored,
            lockdown_hosts: &policy.lockdown_hosts,
        }
    }
}

/// One toolbar button or menu entry of an install with an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Action<I> {
    pub install_id: String,
    pub name: String,
    pub state: ActionState,
    pub popup: bool,
    pub icon: Option<I>,
}

/// What the shell carries out. `Post` with `Target::Page` goes to `Runtime::page`; `Close` hands
/// back a page view to drop or close outside any borrow of the runtime.
pub enum Effect<P> {
    Post { profile: Id, install_id: String, target: Target, json: String },
    StartBackground { profile: Id, install_id: String },
    Close(P),
    OpenTab { profile: Id, url: Url, active: bool },
    ActionChanged(Id),
}

struct Profile<P, I> {
    router: Router,
    loaded: BTreeMap<String, Loaded<I>>,
    resolver: Resolver,
    pages: HashMap<(String, PageKind), P>,
    rules: Option<BTreeMap<String, String>>,
}

pub struct Runtime<P, I = ()> {
    pub session: Session,
    profiles: HashMap<Id, Profile<P, I>>,
    expiry: Option<Instant>,
}

impl<P, I> Default for Runtime<P, I> {
    fn default() -> Self {
        Runtime { session: Session::default(), profiles: HashMap::new(), expiry: None }
    }
}

/// Loads every enabled Tarn install of `rows` from `dir(row)`, each checked against its recorded
/// tree hash. Blocks; run it off the main thread.
pub fn load_rows(
    rows: Vec<InstalledExtension>,
    dir: impl Fn(&InstalledExtension) -> PathBuf,
    seed: &str,
    locales: &[String],
) -> Vec<(InstalledExtension, Result<Package, LoadError>)> {
    rows.into_iter()
        .filter(|r| r.enabled)
        .filter_map(|row| {
            let dir = dir(&row);
            super::is_tarn_dir(&dir).then(|| {
                let loaded = super::load(&dir, Some(&row.sha256), &row.install_id, seed, locales);
                (row, loaded)
            })
        })
        .collect()
}

pub fn row_dir(row: &InstalledExtension) -> PathBuf {
    Path::new(&row.path).to_owned()
}

impl<P, I: Clone> Runtime<P, I> {
    fn profile(&mut self, profile: Id) -> &mut Profile<P, I> {
        self.profiles.entry(profile).or_insert_with(|| Profile {
            router: Router::new(profile),
            loaded: BTreeMap::new(),
            resolver: Resolver::default(),
            pages: HashMap::new(),
            rules: None,
        })
    }

    pub fn profiles(&self) -> Vec<Id> {
        self.profiles.keys().copied().collect()
    }

    pub fn has_profile(&self, profile: Id) -> bool {
        self.profiles.contains_key(&profile)
    }

    pub fn loaded(&self, profile: Id, install_id: &str) -> Option<&Loaded<I>> {
        self.profiles.get(&profile)?.loaded.get(install_id)
    }

    /// The address the install's page of `kind` opens at; none under a Lockdown profile, where no
    /// extension page opens.
    pub fn page_url(&self, profile: Id, install_id: &str, kind: PageKind, policy: &Policy) -> Option<Url> {
        let package = &self.loaded(profile, install_id).filter(|_| policy.level != ShieldLevel::Lockdown)?.package;
        match kind {
            PageKind::Background => package.background_url(),
            PageKind::Popup => package.popup_url(),
        }
    }

    pub fn installs(&self, profile: Id) -> impl Iterator<Item = &Loaded<I>> {
        self.profiles.get(&profile).into_iter().flat_map(|p| p.loaded.values())
    }

    pub fn has_installs(&self) -> bool {
        self.profiles.keys().any(|&p| self.installs(p).next().is_some())
    }

    pub fn scripts(&self, profile: Id) -> Vec<Rc<WorldScripts>> {
        self.installs(profile).map(|l| l.scripts.clone()).collect()
    }

    pub fn resolver(&self, profile: Id) -> Option<Resolver> {
        Some(self.profiles.get(&profile)?.resolver.clone())
    }

    /// Runs a checked package. An install already running from the same files only takes the new
    /// row and site answers; another version replaces it.
    pub fn insert(&mut self, row: InstalledExtension, package: Package, stored: Vec<(String, bool)>, icon: Option<I>) -> Vec<Effect<P>> {
        let profile = row.profile_id;
        let id = row.install_id.clone();
        if let Some(l) = self.profile(profile).loaded.get_mut(&id)
            && l.row.path == row.path
            && l.row.sha256 == row.sha256
        {
            l.row = row;
            l.stored = stored;
            return Vec::new();
        }
        let effects = self.unload(profile, &id);
        let p = self.profile(profile);
        p.router.add(&package);
        package.serve(&p.resolver);
        let scripts = Rc::new(package.scripts.clone());
        p.loaded.insert(id, Loaded { package: Rc::new(package), scripts, row, stored, icon });
        effects
    }

    fn unload(&mut self, profile: Id, install_id: &str) -> Vec<Effect<P>> {
        let Some(p) = self.profiles.get_mut(&profile) else { return Vec::new() };
        let Some(old) = p.loaded.remove(install_id) else { return Vec::new() };
        p.resolver.forget(&old.package.origin_id);
        let outs = p.router.remove(install_id);
        let mut effects = self.effects(profile, outs);
        if let Some(p) = self.profiles.get_mut(&profile) {
            effects.extend(p.pages.extract_if(|(i, _), _| i == install_id).map(|(_, page)| Effect::Close(page)));
        }
        effects
    }

    /// Brings the profile in line with its stored rows: an install removed, turned off or moved
    /// stops and its session answers go; the others take their current row and site answers.
    pub fn retain(&mut self, profile: Id, rows: &[InstalledExtension], store: &Store) -> Vec<Effect<P>> {
        let Some(p) = self.profiles.get_mut(&profile) else { return Vec::new() };
        let gone: Vec<String> = p
            .loaded
            .iter()
            .filter(|(id, l)| rows.iter().find(|r| r.install_id == **id).is_none_or(|r| !r.enabled || r.path != l.row.path))
            .map(|(id, _)| id.clone())
            .collect();
        let mut effects = Vec::new();
        for id in gone {
            effects.extend(self.unload(profile, &id));
            self.session.forget(profile, Some(&id));
        }
        if let Some(p) = self.profiles.get_mut(&profile) {
            for (id, l) in &mut p.loaded {
                if let Some(row) = rows.iter().find(|r| r.install_id == *id) {
                    l.row = row.clone();
                    l.stored = store.extension_hosts(profile, id).unwrap_or_default();
                }
            }
        }
        effects
    }

    /// The profile's running installs become `results`: a changed tree turns its install off and
    /// drops its site answers. Returns the effects and the installs that did not load.
    pub fn replace(
        &mut self,
        profile: Id,
        results: Vec<(InstalledExtension, Result<Package, LoadError>)>,
        store: &Store,
        icon: impl Fn(&Package) -> Option<I>,
    ) -> (Vec<Effect<P>>, Vec<(InstalledExtension, LoadError)>) {
        self.profile(profile);
        let (ok, failed): (Vec<_>, Vec<_>) = results.into_iter().partition(|(_, r)| r.is_ok());
        let rows: Vec<InstalledExtension> = ok.iter().map(|(row, _)| row.clone()).collect();
        let mut effects = self.retain(profile, &rows, store);
        for (row, package) in ok {
            let Ok(package) = package else { continue };
            let stored = store.extension_hosts(profile, &row.install_id).unwrap_or_default();
            let icon = icon(&package);
            effects.extend(self.insert(row, package, stored, icon));
        }
        let mut errors = Vec::new();
        for (row, result) in failed {
            let Err(e) = result else { continue };
            if e == LoadError::Changed {
                self.turn_off(store, &row);
            }
            errors.push((row, e));
        }
        (effects, errors)
    }

    /// Turns an install off whose files changed: it stops asking and its site answers go.
    pub fn turn_off(&mut self, store: &Store, row: &InstalledExtension) {
        crate::extensions::turn_off(store, row);
        self.session.forget(row.profile_id, Some(&row.install_id));
    }

    pub fn access(&self, profile: Id, install_id: &str, host: &str, policy: &Policy, now: Instant) -> Access {
        let Some(l) = self.loaded(profile, install_id) else { return Access::Denied };
        self.session.access(&l.install(policy), host, now)
    }

    /// The engine's gate for one frame. `ask` is set only the first time in the session, when
    /// the shell raises the site prompt.
    pub fn gate(&mut self, profile: Id, world: &World, nav: &FrameNav, policy: &Policy, view_level: ShieldLevel, now: Instant) -> FrameDecision {
        let install_id = world.install_id();
        let Some(l) = self.loaded(profile, install_id) else { return FrameDecision::default() };
        if !nav.main_frame && policy.locks(nav.top) {
            return FrameDecision::default();
        }
        let session = &self.session;
        let mut decision = super::frame_decision(&l.package, &policy.context(view_level), nav, |host| session.access(&l.install(policy), host, now));
        decision.ask = decision.ask.filter(|host| self.session.ask(profile, install_id, host));
        decision
    }

    /// The shell dropped a site prompt from `gate` without showing it: the next visit asks again.
    pub fn prompt_dropped(&mut self, profile: Id, install_id: &str, host: &str) {
        self.session.unask(profile, install_id, host);
    }

    /// The installs' network rule lists, by install, for views at `view_level`.
    pub fn rule_lists(&self, profile: Id, policy: &Policy, view_level: ShieldLevel, now: Instant) -> BTreeMap<String, String> {
        let context = policy.context(view_level);
        self.installs(profile)
            .filter_map(|l| Some((l.row.install_id.clone(), super::rule_list(&l.package, &context, &self.session.scope(&l.install(policy), now))?)))
            .collect()
    }

    /// The profile's rule lists for views at its own level, kept for `rules`; the lists they
    /// replace when they changed.
    pub fn update_rules(&mut self, profile: Id, policy: &Policy, now: Instant) -> Option<BTreeMap<String, String>> {
        self.expiry = self.session.next_expiry(now);
        let lists = self.rule_lists(profile, policy, policy.level, now);
        let p = self.profiles.get_mut(&profile)?;
        if p.rules.as_ref() == Some(&lists) {
            return None;
        }
        Some(p.rules.replace(lists).unwrap_or_default())
    }

    pub fn rules(&self, profile: Id) -> Option<&BTreeMap<String, String>> {
        self.profiles.get(&profile)?.rules.as_ref()
    }

    /// The profile's views were rebuilt: the next `update_rules` hands them every list again.
    pub fn reset_rules(&mut self, profile: Id) {
        if let Some(p) = self.profiles.get_mut(&profile) {
            p.rules = None;
        }
    }

    fn effects(&mut self, profile: Id, outs: Vec<Out>) -> Vec<Effect<P>> {
        outs.into_iter()
            .filter_map(|out| match out {
                Out::Post { install_id, target, json } => Some(Effect::Post { profile, install_id, target, json }),
                Out::StartBackground(install_id) => Some(Effect::StartBackground { profile, install_id }),
                Out::StopBackground(install_id) => self.profiles.get_mut(&profile)?.pages.remove(&(install_id, PageKind::Background)).map(Effect::Close),
                Out::OpenTab { url, active } => Some(Effect::OpenTab { profile, url, active }),
                Out::ActionChanged(_) => Some(Effect::ActionChanged(profile)),
            })
            .collect()
    }

    fn route(&mut self, profile: Id, call: &Call, f: impl FnOnce(&mut Router, &Env) -> Vec<Out>) -> Vec<Effect<P>> {
        let Some(p) = self.profiles.get_mut(&profile) else { return Vec::new() };
        let mut router = std::mem::take(&mut p.router);
        let outs = {
            let access = |install: &str, host: &str| self.access(profile, install, host, call.policy, call.now);
            f(&mut router, &Env { tabs: call.tabs, access: &access, store: call.store, now: call.now })
        };
        if let Some(p) = self.profiles.get_mut(&profile) {
            p.router = router;
        }
        self.effects(profile, outs)
    }

    /// A message from the install's world in a tab, or from one of its own open pages.
    pub fn handle(&mut self, profile: Id, sender: &Sender, json: &str, call: &Call) -> Vec<Effect<P>> {
        let refused = call.policy.level == ShieldLevel::Lockdown
            || match sender.from {
                From::Page(kind) => self.page(profile, &sender.install_id, kind).is_none(),
                From::Tab { tab, .. } => call.tabs.iter().find(|t| t.id == tab).and_then(|t| t.url.as_ref()).is_none_or(|u| call.policy.locks(u)),
            };
        if refused {
            return Vec::new();
        }
        self.route(profile, call, |router, env| router.handle(sender, json, env))
    }

    /// The user chose the install's action with `tab` in front. The shell opens its popup, if it
    /// has one.
    pub fn clicked(&mut self, profile: Id, install_id: &str, tab: &TabInfo, call: &Call) -> Vec<Effect<P>> {
        if call.policy.level == ShieldLevel::Lockdown {
            return Vec::new();
        }
        self.route(profile, call, |router, env| router.clicked(install_id, tab, env))
    }

    /// The shell created one of the install's page views. Returns a page it did not keep: the
    /// one replaced, or `page` itself once the install is gone.
    pub fn page_opened(&mut self, profile: Id, install_id: &str, kind: PageKind, page: P, now: Instant) -> Option<P> {
        let Some(p) = self.profiles.get_mut(&profile).filter(|p| p.loaded.contains_key(install_id)) else { return Some(page) };
        p.router.page_opened(install_id, kind, now);
        p.pages.insert((install_id.to_owned(), kind), page)
    }

    pub fn page(&self, profile: Id, install_id: &str, kind: PageKind) -> Option<&P> {
        self.profiles.get(&profile)?.pages.get(&(install_id.to_owned(), kind))
    }

    pub fn pages(&self) -> impl Iterator<Item = (Id, &str, PageKind, &P)> {
        self.profiles.iter().flat_map(|(id, p)| p.pages.iter().map(move |((i, k), page)| (*id, i.as_str(), *k, page)))
    }

    /// Takes the page view out, for the shell to close; `page_closed` follows once it is gone.
    pub fn take_page(&mut self, profile: Id, install_id: &str, kind: PageKind) -> Option<P> {
        self.profiles.get_mut(&profile)?.pages.remove(&(install_id.to_owned(), kind))
    }

    /// The page view closed: callers waiting for it get their error.
    pub fn page_closed(&mut self, profile: Id, install_id: &str, kind: PageKind) -> Vec<Effect<P>> {
        let Some(p) = self.profiles.get_mut(&profile) else { return Vec::new() };
        let page = p.pages.remove(&(install_id.to_owned(), kind));
        let outs = p.router.page_closed(install_id, kind);
        let mut effects: Vec<Effect<P>> = page.map(Effect::Close).into_iter().collect();
        effects.extend(self.effects(profile, outs));
        effects
    }

    /// The user's answer to the site prompt; a remembered one is stored. The shell then applies
    /// the rule lists again.
    pub fn answer(&mut self, answer: &Answer, store: &Store, now: Instant) {
        if let Some((host, allow)) = self.session.answer(answer, now) {
            store.set_extension_host(answer.profile, &answer.install_id, &host, allow).ok();
            let stored = store.extension_hosts(answer.profile, &answer.install_id).unwrap_or_default();
            if let Some(l) = self.profiles.get_mut(&answer.profile).and_then(|p| p.loaded.get_mut(&answer.install_id)) {
                l.stored = stored;
            }
        }
        self.expiry = self.session.next_expiry(now);
    }

    /// Forgets the install's site answers, stored and of this session: it asks again on every site.
    pub fn forget_answers(&mut self, store: &Store, profile: Id, install_id: &str) -> Result<(), store::Error> {
        store.retain_extension_hosts(profile, install_id, |_| false)?;
        self.session.forget(profile, Some(install_id));
        if let Some(l) = self.profiles.get_mut(&profile).and_then(|p| p.loaded.get_mut(install_id)) {
            l.stored.clear();
        }
        Ok(())
    }

    /// The profile's Shield level changed: at Lockdown its open pages close, since no extension
    /// page runs there.
    pub fn level_changed(&mut self, profile: Id, policy: &Policy) -> Vec<Effect<P>> {
        if policy.level != ShieldLevel::Lockdown {
            return Vec::new();
        }
        let open: Vec<(String, PageKind)> = self.profiles.get(&profile).map(|p| p.pages.keys().cloned().collect()).unwrap_or_default();
        open.into_iter().flat_map(|(install_id, kind)| self.page_closed(profile, &install_id, kind)).collect()
    }

    /// A tab's view went away: the replies it waited for end, and when the tab `closed`, its Once
    /// grants too. The shell then applies the rule lists again.
    pub fn tab_gone(&mut self, tab: u64, closed: bool) -> Vec<Effect<P>> {
        if closed {
            self.session.tab_closed(tab);
        }
        let mut effects = Vec::new();
        for profile in self.profiles() {
            let outs = self.profiles.get_mut(&profile).map(|p| p.router.tab_closed(tab)).unwrap_or_default();
            effects.extend(self.effects(profile, outs));
        }
        effects
    }

    /// The tab's main frame committed a new document.
    pub fn navigated(&mut self, profile: Id, tab: u64) {
        if let Some(p) = self.profiles.get_mut(&profile) {
            p.router.navigated(tab);
        }
    }

    /// Stops idle background pages; true when a Once grant ran out and the rule lists must be
    /// applied again.
    pub fn tick(&mut self, now: Instant) -> (Vec<Effect<P>>, bool) {
        let mut effects = Vec::new();
        for profile in self.profiles() {
            let outs = self.profiles.get_mut(&profile).map(|p| p.router.tick(now)).unwrap_or_default();
            effects.extend(self.effects(profile, outs));
        }
        let expired = self.expiry.is_some_and(|at| at <= now);
        if expired {
            self.expiry = self.session.next_expiry(now);
        }
        (effects, expired)
    }

    pub fn action(&self, profile: Id, install_id: &str, tab: Option<u64>) -> Option<ActionState> {
        Some(self.profiles.get(&profile)?.router.action(install_id, tab))
    }

    /// The installs with an action, with their title and badge for `tab`.
    pub fn actions(&self, profile: Id, tab: Option<u64>) -> Vec<Action<I>> {
        let Some(p) = self.profiles.get(&profile) else { return Vec::new() };
        p.loaded
            .iter()
            .filter(|(_, l)| l.package.manifest.action.is_some())
            .map(|(id, l)| Action {
                install_id: id.clone(),
                name: crate::theme_fetch::shown(&l.package.name),
                state: p.router.action(id, tab),
                popup: l.package.popup_url().is_some(),
                icon: l.icon.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package(dir: &Path, install: &str, action: bool) -> Package {
        let mut manifest = json!({
            "manifest_version": 3, "name": "Helper", "version": "2.1",
            "browser_specific_settings": {"tarn": {"runtime": 1}},
            "permissions": ["storage"],
            "content_scripts": [{"matches": ["*://*.shop.test/*"], "js": ["c.js"]}],
            "background": {"service_worker": "bg.js"},
        });
        if action {
            manifest["action"] = json!({"default_title": "Shop helper"});
        }
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        std::fs::write(dir.join("c.js"), "window.c = 1;").unwrap();
        std::fs::write(dir.join("bg.js"), "").unwrap();
        super::super::load(dir, None, install, "SEED", &[]).unwrap()
    }

    fn row(install: &str, dir: &Path) -> InstalledExtension {
        InstalledExtension {
            profile_id: 1,
            install_id: install.into(),
            source: "file".into(),
            path: dir.to_string_lossy().into_owned(),
            enabled: true,
            ..Default::default()
        }
    }

    fn policy() -> Policy {
        Policy { every_site: false, lockdown_hosts: Vec::new(), level: ShieldLevel::Standard }
    }

    type Rt = Runtime<&'static str, u8>;

    #[test]
    fn actions_list_installs_with_an_action_and_their_state() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut rt = Rt::default();
        rt.insert(row("aa", a.path()), package(a.path(), "aa", true), Vec::new(), Some(7));
        rt.insert(row("bb", b.path()), package(b.path(), "bb", false), Vec::new(), None);
        let actions = rt.actions(1, Some(7));
        assert_eq!(actions.len(), 1);
        assert_eq!((actions[0].install_id.as_str(), actions[0].state.title.as_str(), actions[0].icon), ("aa", "Shop helper", Some(7)));
        assert!(rt.actions(2, None).is_empty());
        assert!(rt.has_installs());
        assert!(!Rt::default().has_installs());
        assert_eq!(rt.loaded(1, "bb").map(|l| l.package.manifest.version.as_str()), Some("2.1"));
    }

    #[test]
    fn a_site_is_granted_only_after_its_answer_and_never_under_lockdown() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", false), Vec::new(), None);
        let now = Instant::now();
        assert_eq!(rt.access(1, "aa", "shop.test", &policy(), now), Access::Ask);
        assert_eq!(rt.access(1, "zz", "shop.test", &policy(), now), Access::Denied);
        rt.answer(&Answer { profile: 1, install_id: "aa".into(), host: "shop.test".into(), allow: true, remember: false, tab: Some(3) }, &store, now);
        assert_eq!(rt.access(1, "aa", "shop.test", &policy(), now), Access::Granted);
        let lockdown = Policy { lockdown_hosts: vec!["shop.test".into()], ..policy() };
        assert_eq!(rt.access(1, "aa", "shop.test", &lockdown, now), Access::Denied);
        assert!(rt.tab_gone(3, true).is_empty());
        assert_ne!(rt.access(1, "aa", "shop.test", &policy(), now), Access::Granted);
    }

    #[test]
    fn the_gate_asks_once_and_runs_after_a_grant() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        assert_eq!(profile, 1);
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", false), Vec::new(), None);
        let (world, url, now) = (World::for_install("aa").unwrap(), Url::parse("https://www.shop.test/cart").unwrap(), Instant::now());
        let nav = FrameNav { url: &url, top: &url, main_frame: true };
        let first = rt.gate(1, &world, &nav, &policy(), ShieldLevel::Standard, now);
        assert_eq!(first, FrameDecision { run: Vec::new(), ask: Some("www.shop.test".into()) });
        assert_eq!(rt.gate(1, &world, &nav, &policy(), ShieldLevel::Standard, now), FrameDecision::default());
        assert!(rt.gate(1, &world, &nav, &policy(), ShieldLevel::Lockdown, now).ask.is_none());
        rt.answer(&Answer { profile: 1, install_id: "aa".into(), host: "www.shop.test".into(), allow: true, remember: true, tab: None }, &store, now);
        assert_eq!(rt.gate(1, &world, &nav, &policy(), ShieldLevel::Standard, now).run, [0]);
        assert_eq!(rt.loaded(1, "aa").unwrap().stored, [("www.shop.test".to_owned(), true)]);
    }

    #[test]
    fn forgetting_answers_ends_stored_and_session_answers() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", false), Vec::new(), None);
        let (world, now) = (World::for_install("aa").unwrap(), Instant::now());
        let (once, kept) = (Url::parse("https://a.shop.test/").unwrap(), Url::parse("https://b.shop.test/").unwrap());
        let run = |rt: &mut Rt, url: &Url| rt.gate(profile, &world, &FrameNav { url, top: url, main_frame: true }, &policy(), ShieldLevel::Standard, now).run;
        rt.answer(&Answer { profile, install_id: "aa".into(), host: "a.shop.test".into(), allow: true, remember: false, tab: Some(3) }, &store, now);
        rt.answer(&Answer { profile, install_id: "aa".into(), host: "b.shop.test".into(), allow: true, remember: true, tab: None }, &store, now);
        assert_eq!((run(&mut rt, &once), run(&mut rt, &kept)), (vec![0], vec![0]));
        rt.forget_answers(&store, profile, "aa").unwrap();
        assert!(run(&mut rt, &once).is_empty() && run(&mut rt, &kept).is_empty(), "neither answer holds any more");
        assert!(store.extension_hosts(profile, "aa").unwrap().is_empty());
        assert!(rt.loaded(profile, "aa").unwrap().stored.is_empty());
    }

    #[test]
    fn frames_in_a_lockdown_page_run_and_send_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = Rt::default();
        let mut p = package(dir.path(), "aa", false);
        p.manifest.content_scripts[0].all_frames = true;
        rt.insert(InstalledExtension { all_sites: true, source: "official:x".into(), ..row("aa", dir.path()) }, p, Vec::new(), None);
        let (world, now) = (World::for_install("aa").unwrap(), Instant::now());
        let (frame, top) = (Url::parse("https://cdn.shop.test/w").unwrap(), Url::parse("https://locked.test/").unwrap());
        let open = Policy { every_site: true, ..policy() };
        let locked = Policy { lockdown_hosts: vec!["Locked.test.".into()], ..open.clone() };
        let nav = FrameNav { url: &frame, top: &top, main_frame: false };
        assert_eq!(rt.gate(1, &world, &nav, &open, ShieldLevel::Standard, now).run, [0]);
        assert!(rt.gate(1, &world, &nav, &locked, ShieldLevel::Standard, now).run.is_empty());
        let tabs = [TabInfo { id: 4, url: Some(top.clone()), title: String::new(), active: true, current_window: true }];
        let sender = Sender { install_id: "aa".into(), from: From::Tab { tab: 4, frame: engine::tarn_ext::FrameId(9) }, origin: Some(frame.clone()) };
        let json = r#"{"id":1,"call":"storage.get","args":[null]}"#;
        assert_eq!(rt.handle(1, &sender, json, &Call { tabs: &tabs, store: None, policy: &open, now }).len(), 1);
        assert!(rt.handle(1, &sender, json, &Call { tabs: &tabs, store: None, policy: &locked, now }).is_empty());
    }

    #[test]
    fn no_extension_page_opens_under_a_lockdown_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", true), Vec::new(), None);
        let background = rt.loaded(1, "aa").unwrap().package.background_url();
        assert!(background.is_some());
        assert_eq!(rt.page_url(1, "aa", PageKind::Background, &policy()), background);
        let locked = Policy { level: ShieldLevel::Lockdown, ..policy() };
        assert_eq!(rt.page_url(1, "aa", PageKind::Background, &locked), None);
        assert_eq!(rt.page_url(1, "aa", PageKind::Popup, &locked), None);
        assert_eq!(rt.page_url(1, "zz", PageKind::Background, &policy()), None);
    }

    #[test]
    fn a_profile_moved_into_lockdown_closes_its_pages_and_hears_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", true), Vec::new(), None);
        let now = Instant::now();
        rt.page_opened(1, "aa", PageKind::Background, "background", now);
        rt.page_opened(1, "aa", PageKind::Popup, "popup", now);
        let sender = Sender { install_id: "aa".into(), from: From::Page(PageKind::Background), origin: None };
        let json = r#"{"id":1,"call":"storage.get","args":[null]}"#;
        let locked = Policy { level: ShieldLevel::Lockdown, ..policy() };
        assert!(rt.handle(1, &sender, json, &Call { tabs: &[], store: None, policy: &locked, now }).is_empty());
        assert_eq!(rt.handle(1, &sender, json, &Call { tabs: &[], store: None, policy: &policy(), now }).len(), 1);
        let tab = TabInfo { id: 4, url: Url::parse("https://shop.test/").ok(), title: String::new(), active: true, current_window: true };
        assert!(rt.clicked(1, "aa", &tab, &Call { tabs: &[], store: None, policy: &locked, now }).is_empty());
        assert!(rt.level_changed(1, &policy()).is_empty());
        let mut closed: Vec<&str> = rt.level_changed(1, &locked).into_iter().filter_map(|e| if let Effect::Close(p) = e { Some(p) } else { None }).collect();
        closed.sort_unstable();
        assert_eq!(closed, ["background", "popup"]);
        assert!(rt.pages().next().is_none());
    }

    #[test]
    fn pages_are_kept_until_their_install_goes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let mut rt = Rt::default();
        let r = row("aa", dir.path());
        rt.insert(r.clone(), package(dir.path(), "aa", true), Vec::new(), None);
        let now = Instant::now();
        assert_eq!(rt.page_opened(1, "aa", PageKind::Popup, "popup", now), None);
        assert_eq!(rt.page_opened(1, "zz", PageKind::Popup, "stray", now), Some("stray"));
        assert_eq!(rt.page(1, "aa", PageKind::Popup), Some(&"popup"));
        assert!(rt.insert(r.clone(), package(dir.path(), "aa", true), Vec::new(), None).is_empty(), "the same files keep running");
        let effects = rt.retain(1, &[InstalledExtension { enabled: false, ..r }], &store);
        assert!(matches!(effects.as_slice(), [Effect::Close("popup")]));
        assert!(rt.loaded(1, "aa").is_none() && rt.page(1, "aa", PageKind::Popup).is_none());
    }

    #[test]
    fn a_changed_tree_is_turned_off_on_replace() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        package(dir.path(), "aa", false);
        let row =
            InstalledExtension { profile_id: profile, sha256: crate::ext_package::tree_sha256(dir.path()).unwrap(), all_sites: true, ..row("aa", dir.path()) };
        store.put_extension(&row).unwrap();
        store.set_extension_host(profile, "aa", "shop.test", true).unwrap();
        let mut rt = Rt::default();
        let (_, failed) = rt.replace(profile, load_rows(vec![row.clone()], row_dir, "SEED", &[]), &store, |_| None);
        assert!(failed.is_empty() && rt.loaded(profile, "aa").is_some());
        std::fs::write(dir.path().join("c.js"), "steal()").unwrap();
        let (_, failed) = rt.replace(profile, load_rows(vec![row], row_dir, "SEED", &[]), &store, |_| None);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].1, LoadError::Changed);
        let stored = store.extensions(profile).unwrap().remove(0);
        assert!(!stored.enabled && !stored.all_sites);
        assert!(store.extension_hosts(profile, "aa").unwrap().is_empty());
        assert!(rt.loaded(profile, "aa").is_none());
    }

    #[test]
    fn rules_are_handed_over_only_when_they_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = Rt::default();
        rt.insert(row("aa", dir.path()), package(dir.path(), "aa", false), Vec::new(), None);
        let now = Instant::now();
        assert_eq!(rt.update_rules(1, &policy(), now), Some(BTreeMap::new()));
        assert_eq!(rt.update_rules(1, &policy(), now), None);
        rt.reset_rules(1);
        assert!(rt.update_rules(1, &policy(), now).is_some());
        assert!(!rt.tick(now).1);
    }
}
