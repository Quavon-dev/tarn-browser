use std::rc::Rc;

use browser_core::tabs::TabKey;
use devkit::json_view::{MAX_BYTES, Tree};
use devkit::responsive::{PRESETS, Preset, fit};
use engine::view_policy::MIN_PAGE_ZOOM;
use serde_json::{Value, json};

use super::Browser;
use super::pages::Request;

impl Browser {
    pub fn developer(&self) -> bool {
        self.core.disk.setting(devkit::SETTING).ok().flatten().as_deref() == Some("on")
    }

    pub fn set_developer(&mut self, on: bool) {
        if self.core.disk.set_setting(devkit::SETTING, if on { "on" } else { "off" }).is_ok() && !on && !self.current_is_dev() {
            self.set_responsive(None);
        }
    }

    pub fn is_dev_tab(&self, key: TabKey) -> bool {
        self.model.profile_of(&self.core, key).and_then(|p| self.core.profiles.borrow().get(&(key.private, p)).map(|rt| rt.profile.is_dev)).unwrap_or(false)
    }

    pub fn current_is_dev(&self) -> bool {
        self.model.profile().and_then(|k| self.core.profiles.borrow().get(&k).map(|rt| rt.profile.is_dev)).unwrap_or(false)
    }

    pub fn devtools_on(&self) -> bool {
        self.developer() || self.current_is_dev()
    }

    pub fn dev_profile(&self) -> Option<String> {
        self.core.disk.profiles().ok()?.into_iter().find(|p| p.is_dev).map(|p| engine::view_policy::title(&p.name))
    }

    pub fn create_dev_profile(&mut self) {
        let existing = self.core.disk.profiles().unwrap_or_default().into_iter().find(|p| p.is_dev);
        let space = match existing {
            Some(p) => self.core.disk.spaces(p.id).ok().and_then(|s| s.into_iter().next()),
            None => self.core.disk.create_dev_profile(&uuid::Uuid::new_v4().to_string()).ok().map(|(_, space)| space),
        };
        let Some(space) = space else { return };
        self.model.reload_spaces(&self.core);
        let ix = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id).unwrap_or(0);
        self.switch_space(ix);
    }

    pub fn open_local_server(&mut self, port: u16) {
        self.new_tab(&format!("http://localhost:{port}/"));
    }

    pub(super) fn after_load_devtools(&mut self, key: TabKey) {
        let dev = self.is_dev_tab(key);
        if dev {
            self.outbox.push((key, "clearCache", Value::Null));
        }
        if (dev || self.developer()) && self.unlocked(key).is_some() {
            self.ask(key, Request::Json(key), "readJson", json!({ "maxBytes": MAX_BYTES }));
        }
    }

    pub fn json_request(&mut self, key: TabKey, data: &Value) -> bool {
        self.answered(data, key) == Some(Request::Json(key))
    }

    pub fn show_json(&mut self, key: TabKey, url: &str, tree: Tree) {
        if self.pages.get(&key).is_some_and(|p| p.url == url) {
            self.json_pages.insert(key, (url.to_owned(), Rc::new(tree)));
        }
    }

    pub fn active_json(&self) -> Option<Rc<Tree>> {
        let key = self.model.active?;
        let (url, tree) = self.json_pages.get(&key)?;
        self.pages.get(&key).is_some_and(|p| p.url == *url).then(|| tree.clone())
    }

    pub fn set_json_raw(&mut self, raw: bool) {
        self.change_json(|tree| tree.raw = raw);
    }

    pub fn toggle_json_node(&mut self, path: &[usize]) {
        self.change_json(|tree| tree.toggle(path));
    }

    fn change_json(&mut self, change: impl FnOnce(&mut Tree)) {
        if let Some((_, tree)) = self.model.active.and_then(|k| self.json_pages.get_mut(&k)) {
            let mut changed = Tree::clone(tree);
            change(&mut changed);
            *tree = Rc::new(changed);
        }
    }

    pub fn cycle_responsive(&mut self) {
        let next = match self.responsive {
            None => Some(0),
            Some(i) if i + 1 < PRESETS.len() => Some(i + 1),
            Some(_) => None,
        };
        self.set_responsive(next);
    }

    pub(super) fn set_responsive(&mut self, preset: Option<usize>) {
        self.responsive = preset;
        if preset.is_none() {
            self.restore_zoom();
        }
    }

    fn restore_zoom(&mut self) {
        for (key, _) in std::mem::take(&mut self.responsive_zoom) {
            self.zoom_back(key);
        }
    }

    fn zoom_back(&mut self, key: TabKey) {
        let zoom = self.pages.get(&key).map_or(1.0, |p| if p.zoom > 0.0 { p.zoom } else { 1.0 });
        self.outbox.push((key, "zoom", json!({ "factor": zoom })));
    }

    pub fn responsive_preset(&self) -> Option<Preset> {
        self.responsive.filter(|_| self.devtools_on()).and_then(|i| PRESETS.get(i)).copied()
    }

    pub fn fit_responsive(&mut self, width: f32, height: f32) {
        let Some(preset) = self.responsive_preset() else { return self.restore_zoom() };
        let split: Vec<TabKey> = self.responsive_zoom.keys().copied().filter(|k| self.in_split(*k)).collect();
        for key in split {
            self.responsive_zoom.remove(&key);
            self.zoom_back(key);
        }
        let Some(key) = self.model.active.filter(|k| !self.in_split(*k)) else { return };
        let scale = f64::from(fit(&preset, width, height).scale).max(MIN_PAGE_ZOOM);
        if self.responsive_zoom.get(&key).is_none_or(|z| (z - scale).abs() > 0.001) {
            self.responsive_zoom.insert(key, scale);
            self.outbox.push((key, "zoom", json!({ "factor": scale })));
        }
    }

    pub(super) fn forget_devtools_page(&mut self, key: TabKey) {
        self.json_pages.remove(&key);
    }

    pub(super) fn forget_devtools(&mut self, key: TabKey) {
        self.json_pages.remove(&key);
        self.responsive_zoom.remove(&key);
    }
}
