use std::time::Instant;

use browser_core::extensions::install::installed_dir;
use browser_core::extensions::{EVERY_SITE_SETTING, badge, every_site_setting};
use browser_core::tabs::TabKey;
use browser_core::tarn_ext::LoadError;
use browser_core::tarn_ext::grants::Answer;
use browser_core::tarn_ext::router::{From, Sender, TabInfo, Target};
use browser_core::tarn_ext::runtime::{self, Action, Call, Policy};
use browser_core::theme_fetch::shown;
use engine::tarn_ext::{FrameId, FrameNav, PageKind, World};
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Id;
use url::Url;

use crate::data::Platform;
use crate::platform::ext;
use crate::shell::{Effect, Shell};

pub const TICK: std::time::Duration = std::time::Duration::from_secs(5);
pub const POST: &str = "extensionPost";
pub const MESSAGE: &str = "extensionMessage";
pub const FRAME: &str = "extensionFrame";

/// An extension page the shell shows: a hidden background page or the action popup.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    pub profile: Id,
    pub install_id: String,
    pub kind: PageKind,
    pub store_id: String,
    pub world: String,
    pub url: String,
    pub bootstrap: String,
    pub level: String,
    pub title: String,
}

/// The per-site question of an extension, shown over its tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub tab: TabKey,
    pub profile: Id,
    pub install_id: String,
    pub name: String,
    pub badge: String,
    pub host: String,
}

pub type ActionItem = Action<()>;
type Effects = Vec<runtime::Effect<PageView>>;

#[derive(Default)]
pub struct Runtime {
    pub rt: runtime::Runtime<PageView>,
    pub asks: Vec<Ask>,
    pub queued: Vec<Effect>,
    pub locales: Vec<String>,
    pub root: std::path::PathBuf,
}

pub fn world(install_id: &str) -> String {
    World::for_install(install_id).map(|w| w.name().to_owned()).unwrap_or_default()
}

impl<D: Platform> Shell<D> {
    fn persistent_store(&self, profile: Id) -> Option<String> {
        self.core.profiles.borrow().get(&(false, profile)).map(|rt| rt.data.store_id())
    }

    fn profile_of_store(&self, store_id: &str) -> Option<Id> {
        self.core.profiles.borrow().iter().find(|((private, _), rt)| !private && rt.data.store_id() == store_id).map(|((_, id), _)| *id)
    }

    /// The persistent profile on screen; none in a private space or burner tab.
    pub fn extension_profile(&self) -> Option<Id> {
        self.model.profile().filter(|(private, _)| !private).map(|(_, id)| id)
    }

    pub fn store_link_allowed(&self, key: TabKey) -> bool {
        let page = self.pages.get(&key).and_then(|p| p.committed.as_deref()).and_then(|u| Url::parse(u).ok());
        let profile = self.model.profile_of(&self.core, key).filter(|p| self.extension_profile() == Some(*p));
        profile.is_some_and(|p| self.core.store_link_allowed(key.private, p, page.as_ref()))
    }

    fn policy(&self, profile: Id) -> Policy {
        self.core.extension_policy(profile, every_site_setting(self.core.disk.setting(EVERY_SITE_SETTING).ok().flatten().as_deref()))
    }

    /// Loads the enabled installs of an opened persistent profile, each only if its files still
    /// hash as recorded, and hands their scripts, rules and pages to the engine.
    pub fn load_extensions(&mut self, profile: Id) {
        let Some(store_id) = self.persistent_store(profile) else { return };
        let Some(seed) = self.core.profiles.borrow().get(&(false, profile)).map(|rt| rt.own_store.clone()) else { return };
        let rows = self.core.disk.extensions(profile).unwrap_or_default();
        let root = self.ext.root.clone();
        let results = runtime::load_rows(rows, |row| installed_dir(&root, row), &seed, &self.ext.locales);
        let (outs, failed) = self.ext.rt.replace(profile, results, &self.core.disk, |_| None);
        for (row, error) in failed {
            self.notice = Some(match error {
                LoadError::Changed => format!("{} changed on disk and was turned off.", shown(&row.name)),
                LoadError::Refused(why) => format!("{}: {}", shown(&row.name), shown(&why)),
            });
        }
        let rt = &self.ext.rt;
        self.ext.asks.retain(|a| a.profile != profile || rt.loaded(profile, &a.install_id).is_some());
        let effects = self.carry(outs, Instant::now());
        self.ext.queued.extend(effects);
        if let Some(resolver) = self.ext.rt.resolver(profile) {
            ext::serve(&store_id, resolver.handler());
        }
        if ext::worlds_supported() {
            ext::set_scripts(&store_id, self.ext.rt.scripts(profile));
        }
        self.ext.rt.reset_rules(profile);
        self.push_rules(profile, Instant::now());
    }

    /// The installs' network rules where they hold grants now; sent again only when they change.
    pub fn push_rules(&mut self, profile: Id, now: Instant) {
        let Some(store_id) = self.persistent_store(profile) else { return };
        let policy = self.policy(profile);
        if self.ext.rt.update_rules(profile, &policy, now).is_some() {
            ext::set_rule_lists(&store_id, self.ext.rt.rules(profile).cloned().unwrap_or_default().into_iter().collect());
        }
    }

    /// The engine's question for one frame: which of the world's scripts may run there.
    pub fn gate(&mut self, tab: &str, store_id: &str, world: &World, nav: &FrameNav, now: Instant) -> Vec<usize> {
        let (Some(profile), Some(id)) = (self.profile_of_store(store_id), tab.strip_prefix('n').and_then(|n| n.parse::<Id>().ok())) else { return Vec::new() };
        let policy = self.policy(profile);
        let view_level = self.core.level(false, profile, Some(nav.top)).unwrap_or(ShieldLevel::Lockdown);
        let decision = self.ext.rt.gate(profile, world, nav, &policy, view_level, now);
        if let Some(host) = decision.ask
            && let Some(l) = self.ext.rt.loaded(profile, world.install_id())
        {
            let ask = Ask {
                tab: TabKey { private: false, id },
                profile,
                install_id: l.row.install_id.clone(),
                name: l.package.name.clone(),
                badge: badge(&l.row.source),
                host,
            };
            self.ext.asks.push(ask);
        }
        decision.run
    }

    fn tab_infos(&self) -> Vec<TabInfo> {
        if self.model.private() {
            return Vec::new();
        }
        self.model
            .tabs
            .iter()
            .map(|t| {
                let key = self.model.key(t);
                let url = Url::parse(&self.tab_url(key)).ok();
                let title = self.pages.get(&key).map_or_else(|| t.title.clone(), |p| p.title.clone());
                TabInfo { id: key.id as u64, url, title, active: self.model.active == Some(key), current_window: true }
            })
            .collect()
    }

    fn route(&mut self, profile: Id, now: Instant, f: impl FnOnce(&mut runtime::Runtime<PageView>, &Call) -> Effects) -> Vec<Effect> {
        let tabs = if self.extension_profile() == Some(profile) { self.tab_infos() } else { Vec::new() };
        let policy = self.policy(profile);
        let outs = f(&mut self.ext.rt, &Call { tabs: &tabs, store: Some(&self.core.disk), policy: &policy, now });
        self.carry(outs, now)
    }

    /// A message from an install's world in a tab.
    pub fn extension_message(&mut self, key: TabKey, data: &Value, now: Instant) -> Vec<Effect> {
        let (Some(profile), false) = (self.extension_profile(), key.private) else { return Vec::new() };
        let world = data.get("world").and_then(Value::as_str).and_then(World::parse);
        let (Some(world), Some(frame), Some(json)) = (world, data.get("frame").and_then(Value::as_u64), data.get("json").and_then(Value::as_str)) else {
            return Vec::new();
        };
        let origin = data.get("origin").and_then(Value::as_str).and_then(|o| Url::parse(o).ok());
        let sender = Sender { install_id: world.install_id().to_owned(), from: From::Tab { tab: key.id as u64, frame: FrameId(frame) }, origin };
        self.route(profile, now, |rt, call| rt.handle(profile, &sender, json, call))
    }

    /// A message from one of an install's own pages.
    pub fn page_message(&mut self, profile: Id, install_id: &str, kind: PageKind, data: &Value, now: Instant) -> Vec<Effect> {
        let from = data.get("world").and_then(Value::as_str);
        let Some(json) = data.get("json").and_then(Value::as_str).filter(|_| from == Some(world(install_id).as_str())) else { return Vec::new() };
        let sender = Sender { install_id: install_id.to_owned(), from: From::Page(kind), origin: None };
        self.route(profile, now, |rt, call| rt.handle(profile, &sender, json, call))
    }

    fn page(&mut self, profile: Id, install_id: &str, kind: PageKind) -> Option<PageView> {
        let policy = self.policy(profile);
        let url = self.ext.rt.page_url(profile, install_id, kind, &policy)?;
        let package = self.ext.rt.loaded(profile, install_id)?.package.clone();
        let level = policy.level;
        let title = self.ext.rt.action(profile, install_id, None).map_or_else(|| package.name.clone(), |a| a.title);
        Some(PageView {
            profile,
            install_id: install_id.to_owned(),
            kind,
            store_id: self.persistent_store(profile)?,
            world: package.world.name().to_owned(),
            url: url.to_string(),
            bootstrap: package.bootstrap(true),
            level: level.as_str().to_owned(),
            title,
        })
    }

    fn carry(&mut self, outs: Effects, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        for out in outs {
            match out {
                runtime::Effect::Post { profile, install_id, target, json } => {
                    let world = world(&install_id);
                    let post = |key: TabKey, frame: Value| Effect::Command(key, POST, json!({ "world": world, "frame": frame, "json": json }));
                    let live = self.extension_profile() == Some(profile);
                    match target {
                        Target::Frame { tab, frame } if live => effects.push(post(TabKey { private: false, id: tab as Id }, json!(frame.0))),
                        Target::Tab(tab) if live => effects.push(post(TabKey { private: false, id: tab as Id }, Value::Null)),
                        Target::AllTabs if live => {
                            let keys: Vec<TabKey> = self.model.lifecycle.live().filter(|k| !k.private && self.model.tab(*k).is_some()).collect();
                            effects.extend(keys.into_iter().map(|k| post(k, Value::Null)));
                        }
                        Target::Page(kind) => effects.push(Effect::PagePost(install_id, kind, json)),
                        _ => {}
                    }
                }
                runtime::Effect::StartBackground { profile, install_id } => {
                    if let Some(page) = self.page(profile, &install_id, PageKind::Background) {
                        self.ext.rt.page_opened(profile, &install_id, PageKind::Background, page, now);
                    }
                }
                runtime::Effect::OpenTab { profile, url, active } => {
                    if self.extension_profile() == Some(profile)
                        && let Some(key) = self.model.new_tab(&self.core, url.as_str(), None)
                    {
                        self.model.reload_space(&self.core);
                        if active {
                            self.activate(key);
                        }
                    }
                }
                runtime::Effect::Close(_) | runtime::Effect::ActionChanged(_) => {}
            }
        }
        effects
    }

    /// The profile's Shield level changed: at Lockdown its extension pages close.
    pub fn extension_level_changed(&mut self, profile: Id) {
        let policy = self.policy(profile);
        let outs = self.ext.rt.level_changed(profile, &policy);
        let effects = self.carry(outs, Instant::now());
        self.ext.queued.extend(effects);
    }

    /// Forgets the install's site answers, stored and of this session: it asks again on every site.
    pub fn forget_site_answers(&mut self, profile: Id, install_id: &str) -> Result<(), String> {
        self.ext.rt.forget_answers(&self.core.disk, profile, install_id).map_err(|e| e.to_string())?;
        self.push_rules(profile, Instant::now());
        Ok(())
    }

    /// The action entries of the profile on screen, with their title and badge for the active tab.
    pub fn action_items(&self) -> Vec<ActionItem> {
        let Some(profile) = self.extension_profile() else { return Vec::new() };
        self.ext.rt.actions(profile, self.model.active.map(|k| k.id as u64))
    }

    /// The user chose an action entry: its popup opens, or its background hears `onClicked`.
    pub fn click_action(&mut self, install_id: &str, now: Instant) -> Vec<Effect> {
        let Some(profile) = self.extension_profile() else { return Vec::new() };
        let Some(tab) = self.model.active.and_then(|k| self.tab_infos().into_iter().find(|t| t.id == k.id as u64)) else { return Vec::new() };
        let mut effects = self.route(profile, now, |rt, call| rt.clicked(profile, install_id, &tab, call));
        if self.popup().is_none()
            && let Some(page) = self.page(profile, install_id, PageKind::Popup)
        {
            self.ext.rt.page_opened(profile, install_id, PageKind::Popup, page, now);
        }
        effects.extend(std::mem::take(&mut self.ext.queued));
        effects
    }

    /// The action popup on screen.
    pub fn popup(&self) -> Option<PageView> {
        self.ext.rt.pages().find(|(_, _, kind, _)| *kind == PageKind::Popup).map(|(_, _, _, page)| page.clone())
    }

    pub fn close_popup(&mut self) -> Vec<Effect> {
        let Some(page) = self.popup() else { return Vec::new() };
        let outs = self.ext.rt.page_closed(page.profile, &page.install_id, PageKind::Popup);
        self.carry(outs, Instant::now())
    }

    pub fn active_ask(&self) -> Option<&Ask> {
        self.ext.asks.iter().find(|a| Some(a.tab) == self.model.active)
    }

    /// Allow or Deny, Once (an hour, or until the tab closes) or remembered.
    pub fn answer_ask(&mut self, allow: bool, remember: bool, now: Instant) -> Vec<Effect> {
        let Some(at) = self.ext.asks.iter().position(|a| Some(a.tab) == self.model.active) else { return Vec::new() };
        let ask = self.ext.asks.remove(at);
        let answer = Answer { profile: ask.profile, install_id: ask.install_id.clone(), host: ask.host.clone(), allow, remember, tab: Some(ask.tab.id as u64) };
        self.ext.rt.answer(&answer, &self.core.disk, now);
        self.push_rules(ask.profile, now);
        if allow { vec![Effect::Command(ask.tab, "reload", Value::Null)] } else { Vec::new() }
    }

    pub fn has_extensions(&self) -> bool {
        self.ext.rt.has_installs()
    }

    /// Stops idle background pages and renews the network rules when a Once answer ends.
    pub fn extensions_tick(&mut self, now: Instant) -> Vec<Effect> {
        let (outs, expired) = self.ext.rt.tick(now);
        let effects = self.carry(outs, now);
        if expired {
            for profile in self.ext.rt.profiles() {
                self.push_rules(profile, now);
            }
        }
        let live: Vec<TabKey> = self.model.lifecycle.live().collect();
        self.drop_asks(|a| live.contains(&a.tab));
        effects
    }

    /// Keeps the asks `keep` holds; the others were never answered, so their sites may ask again.
    fn drop_asks(&mut self, keep: impl Fn(&Ask) -> bool) {
        let (kept, dropped): (Vec<Ask>, Vec<Ask>) = std::mem::take(&mut self.ext.asks).into_iter().partition(keep);
        self.ext.asks = kept;
        for a in dropped {
            self.ext.rt.prompt_dropped(a.profile, &a.install_id, &a.host);
        }
    }

    /// A tab's view went away (closed or put to sleep): its pending replies end.
    pub(crate) fn extension_tab_gone(&mut self, key: TabKey, closed: bool) {
        if key.private {
            return;
        }
        self.drop_asks(|a| a.tab != key);
        let outs = self.ext.rt.tab_gone(key.id as u64, closed);
        if closed {
            for profile in self.ext.rt.profiles() {
                self.push_rules(profile, Instant::now());
            }
        }
        let effects = self.carry(outs, Instant::now());
        self.ext.queued.extend(effects);
    }

    /// The tab's main frame committed a new document.
    pub(crate) fn extension_navigated(&mut self, key: TabKey) {
        if let Some(profile) = self.extension_profile().filter(|_| !key.private) {
            self.ext.rt.navigated(profile, key.id as u64);
        }
    }

    /// Loads the profile on screen once it is open.
    pub(crate) fn ensure_extensions(&mut self) {
        if let Some(profile) = self.extension_profile()
            && !self.ext.rt.has_profile(profile)
        {
            self.load_extensions(profile);
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.ext.queued)
    }

    pub fn page_views(&self) -> Vec<PageView> {
        let mut pages: Vec<PageView> =
            self.ext.rt.pages().filter(|(_, _, kind, _)| *kind == PageKind::Background).map(|(_, _, _, page)| page.clone()).collect();
        pages.sort_by(|a, b| (a.profile, &a.install_id).cmp(&(b.profile, &b.install_id)));
        pages
    }
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod tests;
