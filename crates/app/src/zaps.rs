//! Zap Element: elements the user hid on a site, per profile and host, as a content rule list in
//! WebKit and a style sheet in Chromium on Windows.
//! Private and burner spaces keep none.

use std::rc::Rc;

use engine::FilterLists;
use gpui::{Context, actions};
use shield::{ShieldLevel, lists};
use store::Id;
use url::Url;

use crate::browser::{Browser, Msg, TabKey};
use crate::shared::AppCore;
use shield::groups::hex;

actions!(tarn, [ZapElement]);

const GROUP: &str = "zaps";

fn rules(zaps: &[(String, String)]) -> Option<String> {
    lists::compile_zaps(zaps.iter().map(|(host, selector)| (host.as_str(), selector.as_str())))
}

fn key(zaps: &[(String, String)]) -> Option<String> {
    rules(zaps).map(|json| hex(lists::fingerprint(&[&json])))
}

/// WebKit's rule-list group of the profile whose own store is `own_store`.
fn group(own_store: &str) -> String {
    format!("zap.{own_store}")
}

/// Deletes the compiled hidden elements of a profile whose data is going away.
pub(crate) fn forget(own_store: &str) {
    FilterLists::remove(&group(own_store));
}

/// Puts the profile's stored hidden elements into its WebKit views, open pages included.
pub(crate) fn apply(core: &Rc<AppCore>, private: bool, profile_id: Id) {
    if private {
        return;
    }
    let zaps = core.disk.zapped_elements(profile_id).unwrap_or_default();
    // Chromium has no content rule lists; the Windows store hides the elements with a style sheet.
    #[cfg(windows)]
    if let Some(rt) = core.profiles.borrow().get(&(false, profile_id)) {
        let kept: Vec<(String, String)> =
            lists::kept_zaps(zaps.iter().map(|(h, s)| (h.as_str(), s.as_str()))).map(|(h, s)| (h.to_owned(), s.to_owned())).collect();
        rt.data.set_hidden_elements(&kept);
    }
    if cfg!(windows) {
        return;
    }
    let json = rules(&zaps);
    let group = {
        let profiles = core.profiles.borrow();
        let Some(rt) = profiles.get(&(false, profile_id)) else { return };
        if json.is_none() {
            rt.data.set_rule_lists(GROUP, None, false);
        }
        group(&rt.own_store)
    };
    let Some(json) = json else { return FilterLists::remove(&group) };
    let compiled = hex(lists::fingerprint(&[&json]));
    let core = core.clone();
    FilterLists::compile(&group, &compiled.clone(), vec![json], move |result| match result {
        // A later change may have finished first; only the rules of the stored rows apply.
        Ok(lists) if key(&core.disk.zapped_elements(profile_id).unwrap_or_default()) == Some(compiled) => {
            if let Some(rt) = core.profiles.borrow().get(&(false, profile_id)) {
                rt.data.set_rule_lists(GROUP, Some(&lists), false);
            }
        }
        Ok(_) => {}
        Err(e) => eprintln!("tarn: hidden elements failed to compile: {e}"),
    });
}

impl Browser {
    /// The current profile's hidden elements of `host`, or of every host with `None`; none in a
    /// private space.
    pub fn zapped(&self, host: Option<&str>) -> Vec<(String, String)> {
        let Some((false, profile_id)) = self.model.profile() else { return Vec::new() };
        let mut zaps = self.core.disk.zapped_elements(profile_id).unwrap_or_default();
        zaps.retain(|(h, _)| host.is_none_or(|host| h == host));
        zaps
    }

    /// Shows one hidden element of `host` again in the current profile, or all with `None`.
    pub fn unzap(&mut self, host: &str, selector: Option<&str>, cx: &mut Context<Self>) {
        let Some((false, profile_id)) = self.model.profile() else { return };
        if self.core.disk.unzap_element(profile_id, host, selector).is_ok() {
            apply(&self.core, false, profile_id);
        }
        cx.notify();
    }

    /// Whether `key` may hide elements: a saved WebKit tab outside Lockdown, which keeps no site data.
    pub(crate) fn zappable(&self, key: TabKey, url: &Url) -> bool {
        !key.private && self.views.get(&key).is_some_and(|v| !v.is_chromium()) && self.level_for_tab(key, url.as_str()) != ShieldLevel::Lockdown
    }

    /// Starts the element picker in the active tab; the click comes back as `Msg::Zap`.
    pub fn zap_element(&mut self) {
        let Some((key, url)) = self.site().filter(|(k, u)| self.zappable(*k, u)) else { return };
        let Some(view) = self.views.get(&key) else { return };
        let events = self.events.clone();
        view.pick_element(move |selector| drop(events.unbounded_send(Msg::Zap(key, url.clone(), selector))));
    }

    /// Hides the picked element on its host, if the tab still shows that host. The selector comes
    /// from page content, so only the picker's own shape is stored.
    pub(crate) fn zap_picked(&mut self, key: TabKey, page: Url, selector: String, cx: &mut Context<Self>) {
        let Some(host) = page.host_str() else { return };
        let same_host = self.views.get(&key).and_then(|v| v.url()).is_some_and(|u| u.host_str() == Some(host));
        let ok = same_host && self.zappable(key, &page) && lists::valid_zap(&selector);
        let Some(profile_id) = self.profile_of(key).filter(|_| ok) else { return };
        let disk = &self.core.disk;
        let count = disk.zapped_elements(profile_id).unwrap_or_default().iter().filter(|(h, _)| h == host).count();
        if count < lists::MAX_ZAPS_PER_SITE && disk.zap_element(profile_id, host, &selector).is_ok() {
            apply(&self.core, false, profile_id);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_follow_every_host_and_change_their_key() {
        let row = |h: &str, s: &str| (h.to_owned(), s.to_owned());
        let one = [row("a.test", ".ad")];
        let two = [row("a.test", ".ad"), row("b.test", "#top"), row("b.test", "body")];
        assert_ne!(key(&one), key(&two));
        let compiled: Vec<serde_json::Value> = serde_json::from_str(&rules(&two).unwrap()).unwrap();
        assert_eq!(compiled.len(), 2);
        assert_eq!(key(&[]), None);
    }
}
