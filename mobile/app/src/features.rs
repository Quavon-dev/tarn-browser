use std::collections::{BTreeMap, HashMap};

use browser_core::bookmarks;
use browser_core::snapshot::{self, SpaceSnapshot};
use browser_core::split::{Share, Split};
use browser_core::tabs::{TabKey, duplicates};
use browser_core::undo::Step;
use store::{Bookmark, Counter, Id, Snapshot};
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

pub const MAX_SITE_CSS: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct SiteRule {
    pub space: Option<usize>,
    pub choices: Vec<(usize, String)>,
}

impl<D: Platform> Shell<D> {
    pub fn toast(&mut self, text: String) {
        self.undo = self.model.closed_tabs.newest().map(|serial| (text.clone(), serial));
        self.notice = Some(text);
    }

    pub fn toast_as(&mut self, tone: browser_core::toast::Tone, text: String) {
        self.tone = Some((text.clone(), tone));
        self.notice = Some(text);
    }

    /// A notice naming `keys` after `command` ran from the command bar, if the profile's hint
    /// throttle allows one and no other notice shows; none in a private space or a burner tab.
    pub fn offer_hint(&mut self, command: &str, keys: &str, typing: bool) {
        if self.notice.is_some() {
            return;
        }
        if let Some(text) = self.core.feature_hint(self.model.profile(), command, keys, browser_core::now(), typing) {
            self.toast_as(browser_core::toast::Tone::Info, text);
        }
    }

    pub fn notice_tone(&self) -> browser_core::toast::Tone {
        self.tone.as_ref().filter(|(text, _)| self.notice.as_ref() == Some(text)).map_or_else(Default::default, |(_, tone)| *tone)
    }

    pub fn clean_up(&mut self) {
        let private = self.model.private();
        let urls: HashMap<store::Id, String> = self.model.tabs.iter().map(|t| (t.id, self.tab_url(TabKey { private, id: t.id }))).collect();
        let Some(sorted) = browser_core::sidebar::clean_up(&mut self.model, &self.core, |t| urls.get(&t.id).cloned().unwrap_or_else(|| t.url.clone())) else {
            return;
        };
        self.toast_as(sorted.tone, sorted.text.clone());
        self.sort_undo = sorted.undo.map(|undo| (sorted.text, undo));
    }

    /// Clean Up's assistant request for the current space's loose tabs, and that space.
    pub fn assistant_grouping(&self) -> Option<((bool, store::Id), browser_core::cleanup::ai::Request)> {
        let private = self.model.private();
        browser_core::sidebar::assistant_grouping(&self.model, &self.core, |t| {
            let key = TabKey { private, id: t.id };
            let url = self.tab_url(key);
            let level = self.model.level_for(&self.core, key, &url);
            (url, level)
        })
    }

    /// Files the tabs the assistant grouped, of those it was sent, as an undoable Clean Up.
    pub fn group_as_proposed(&mut self, space: (bool, store::Id), sent: &std::collections::HashSet<store::Id>, proposal: &[(String, Vec<store::Id>)]) {
        let done = browser_core::sidebar::group_as_proposed(&mut self.model, &self.core, space, sent, proposal);
        self.toast_as(done.tone, done.text.clone());
        self.sort_undo = done.undo.map(|undo| (done.text, undo));
    }

    pub fn can_undo_sort(&self) -> bool {
        self.sort_undo.as_ref().is_some_and(|(text, _)| self.notice.as_ref() == Some(text))
    }

    pub fn undo_sort(&mut self) {
        let Some((_, (private, undo))) = self.sort_undo.take() else { return };
        self.notice = None;
        if !self.run_undo(Step::CleanUp(undo), private) {
            self.toast_as(browser_core::toast::Tone::Warning, "Could not undo the sort".to_owned());
        }
    }

    pub fn can_undo(&self) -> bool {
        self.undo.as_ref().is_some_and(|(text, serial)| self.notice.as_ref() == Some(text) && self.model.closed_tabs.newest() == Some(*serial))
    }

    pub fn can_restore_downloads(&self) -> bool {
        self.cleared_downloads.as_ref().is_some_and(|(text, ..)| self.notice.as_ref() == Some(text))
    }

    pub fn restore_downloads(&mut self) {
        let Some((_, (private, id), list)) = self.cleared_downloads.take() else { return };
        self.notice = None;
        self.core.restore_downloads(private, id, &list);
    }

    pub(crate) fn download_notice(&mut self, text: String, warns: bool) {
        browser_core::toast::push_notice(&mut self.download_notices, (text, warns), |n| n.1);
        self.notice = self.download_notices.first().map(|n| n.0.clone());
    }

    pub fn notice_held(&self) -> bool {
        self.download_notices.first().is_some_and(|(text, warns)| *warns && self.notice.as_ref() == Some(text))
    }

    pub fn dismiss_notice(&mut self) {
        if self.download_notices.first().is_some_and(|(text, _)| self.notice.as_ref() == Some(text)) {
            self.download_notices.remove(0);
        }
        self.notice = self.download_notices.first().map(|n| n.0.clone());
    }

    pub fn undo_close(&mut self) -> bool {
        let Some((_, serial)) = self.undo.take().filter(|(_, s)| self.model.closed_tabs.newest() == Some(*s)) else { return false };
        self.notice = None;
        self.run_undo(Step::Closed(serial), false)
    }

    /// The hardware keyboard's Undo outside a text field and the page: the newest close, Clean Up
    /// or sidebar move that can still be taken back; one that fails gives way to the next.
    pub fn undo_last(&mut self) -> bool {
        while let Some(step) = self.model.pop_undo(&self.core) {
            if self.run_undo(step, false) {
                return true;
            }
        }
        false
    }

    /// Takes back `step`, from Undo or a toast's Undo button; `private` is the space a Clean Up ran in.
    pub fn run_undo(&mut self, step: Step, private: bool) -> bool {
        self.model.undo.forget(&step);
        match step {
            Step::Closed(serial) => self.model.closed_tabs.take(serial, &self.model.spaces).is_some_and(|(ix, closed)| self.restore_closed(ix, closed)),
            Step::CleanUp(undo) => browser_core::sidebar::undo_sort(&mut self.model, &self.core, &(private, undo)),
            Step::Moved(from) => from.revert(&mut self.model, &self.core),
        }
    }

    pub(crate) fn tab_url(&self, key: TabKey) -> String {
        let page = self.pages.get(&key).and_then(|p| p.committed.clone().or_else(|| Some(p.url.clone()))).filter(|u| !u.is_empty());
        page.or_else(|| self.model.tab(key).map(|t| t.url.clone())).unwrap_or_default()
    }

    pub fn shown_split(&self) -> Option<&Split<TabKey>> {
        self.model.active.and_then(|a| self.splits.of(a))
    }

    /// Where `key` sits in the split on screen, as shares of the page area.
    pub fn pane_share(&self, key: TabKey) -> Option<Share> {
        let split = self.shown_split()?;
        split.layout().get(split.panes().iter().position(|k| *k == key)?).copied()
    }

    /// Adds `key` to the active tab's split or starts one with the two; the active tab itself gets
    /// the space's most recently used tab outside a split, else a copy of itself.
    pub fn split_with(&mut self, key: TabKey) {
        let Some(active) = self.model.active else { return };
        let partner = if active != key { Some(key) } else { self.splits.partner(active, self.model.tabs.iter().map(|t| (self.model.key(t), t.last_active))) };
        let url = self.tab_url(active);
        // The burner has one tab, and a copy would load in it instead of beside it.
        let partner = match partner {
            None if self.in_burner() => return self.notice = Some("Split View needs a second tab.".into()),
            None if url::Url::parse(&url).is_err() => return,
            None => match self.new_tab(Some(&url)) {
                Some(copy) => copy,
                None => return,
            },
            Some(partner) => partner,
        };
        if self.splits.join(active, partner) {
            self.activate(partner);
        }
    }

    /// Takes `key` out of its split; a split left with one pane ends.
    pub fn leave_split(&mut self, key: TabKey) {
        if let Some(heir) = self.splits.leave(key).filter(|_| self.model.active == Some(key)) {
            self.activate(heir);
        }
    }

    /// The tabs Close Duplicates closes: every later tab with the page of an earlier one.
    pub fn duplicate_tabs(&self) -> Vec<TabKey> {
        let urls: BTreeMap<Id, String> = self.model.tabs.iter().map(|t| (t.id, self.tab_url(self.model.key(t)))).collect();
        let ids = duplicates(&self.model.tabs, self.model.active.map(|k| k.id), |t| urls.get(&t.id).cloned().unwrap_or_default());
        let private = self.model.private();
        ids.into_iter().map(|id| TabKey { private, id }).collect()
    }

    /// Whether Zap Element has a page to pick from that keeps what it hides.
    pub fn zappable(&self) -> bool {
        !self.model.private() && self.share_url().is_some()
    }

    pub fn permissions(&self) -> Vec<browser_core::permissions::SitePermission> {
        self.profile_id().map(|p| browser_core::permissions::list(self.store(), p)).unwrap_or_default()
    }

    pub fn close_duplicates(&mut self) {
        let keys = self.duplicate_tabs();
        match keys.len() {
            0 => self.notice = Some("No duplicate tabs in this space.".into()),
            _ => self.close_many(&keys),
        }
    }

    pub fn close_many(&mut self, keys: &[TabKey]) {
        if keys.is_empty() {
            return;
        }
        if self.in_burner() {
            self.burn();
            return;
        }
        if self.close_tabs(keys).is_some() {
            self.toast(if keys.len() == 1 { "Tab closed".into() } else { format!("{} tabs closed", keys.len()) });
        }
    }

    pub fn web_link_of(&self, key: TabKey) -> Option<String> {
        let url = self.tab_url(key);
        Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https")).map(|_| url)
    }

    pub fn copy_link(&mut self, clean: bool) -> Option<String> {
        let key = self.model.active?;
        self.copy_link_of(key, clean)
    }

    pub fn copy_link_of(&mut self, key: TabKey, clean: bool) -> Option<String> {
        if clean {
            return self.copy_clean_link_of(key);
        }
        let link = self.web_link_of(key)?;
        self.toast_as(browser_core::toast::Tone::Success, "Link copied".into());
        Some(link)
    }

    pub fn copy_clean_link_of(&mut self, key: TabKey) -> Option<String> {
        let url = self.web_link_of(key)?;
        let (url, removed) = shield::url_clean::clean_link(&url);
        if removed > 0
            && let Some((private, id)) = self.model.profile()
        {
            let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned));
            self.core.store(private).count(id, Counter::TrackingParams, removed, host.as_deref()).ok();
        }
        let text = match removed {
            0 => "Link copied".to_owned(),
            1 => "Link copied, 1 tracking parameter removed".to_owned(),
            n => format!("Link copied, {n} tracking parameters removed"),
        };
        self.toast_as(browser_core::toast::Tone::Success, text);
        Some(url)
    }

    pub fn open_burner(&mut self) {
        if D::private_process() {
            self.notice = Some("Burner tabs open outside the private space.".into());
            return;
        }
        if let Some(ix) = self.model.spaces.iter().position(|s| s.private && Some(s.space.id) == self.model.burner) {
            self.switch_space(ix);
            return;
        }
        let from = self.model.current_space().map(|s| (s.private, s.space.id));
        let Some(ix) = self.model.open_burner(&self.core) else {
            self.toast_as(browser_core::toast::Tone::Warning, "Tarn could not open a burner tab.".into());
            return;
        };
        self.switch_space(ix);
        self.burner_from = from;
    }

    pub fn burn(&mut self) {
        let back = self.burner_from.and_then(|(private, id)| self.model.spaces.iter().position(|s| s.private == private && s.space.id == id));
        let back = back.or_else(|| self.model.spaces.iter().position(|s| !s.private)).unwrap_or(0);
        if self.in_burner() && back != self.model.space_ix {
            self.switch_space(back);
        } else {
            self.drop_burner();
        }
    }

    pub(crate) fn drop_burner(&mut self) {
        let profile = self.model.burner.and_then(|id| self.model.spaces.iter().find(|s| s.private && s.space.id == id)).map(|s| s.space.profile_id);
        if let Some(profile) = profile {
            self.model.closed_tabs.clear_profile(true, profile);
            self.forget_icons((true, profile));
        }
        for key in self.model.burner_tabs(&self.core) {
            self.forget(key);
        }
        self.burner_from = None;
        browser_core::open_file::clear(&self.opened.join("burner"));
        self.model.burn(&self.core);
        self.model.reload_space(&self.core);
    }

    pub fn choose_space(&mut self, ix: usize, now: i64) {
        if ix != self.model.space_ix {
            self.switched_at = Some(now);
        }
        self.switch_space(ix);
    }

    pub fn follow_schedule(&mut self, weekday: u8, hour: u8, now: i64) -> bool {
        if self.model.private() || D::private_process() {
            return false;
        }
        let Some(ix) = browser_core::schedule::due(&self.model.spaces, self.model.space_ix, weekday, hour, self.switched_at, now) else { return false };
        self.switch_space(ix);
        true
    }

    pub fn move_targets(&self, key: TabKey) -> Vec<(usize, String)> {
        self.model.move_targets(&self.core, key).into_iter().filter_map(|ix| self.model.spaces.get(ix).map(|s| (ix, s.space.name.clone()))).collect()
    }

    pub fn move_tab(&mut self, key: TabKey, ix: usize) -> bool {
        let url = self.tab_url(key);
        let name = self.model.spaces.get(ix).map(|s| s.space.name.clone()).unwrap_or_default();
        if self.in_burner() || !self.model.move_to_space(&self.core, key, ix, &url) {
            return false;
        }
        let active = self.model.active == Some(key);
        let next = active.then(|| self.model.close_neighbour(key, &[], false)).flatten();
        self.forget(key);
        self.model.reload_space(&self.core);
        if active {
            self.model.active = None;
            match next.or_else(|| self.model.pick(|_| false)) {
                Some(next) => self.activate(next),
                None => {
                    self.new_tab(None);
                }
            }
        }
        self.notice = Some(format!("Moved to {name}"));
        true
    }

    pub(crate) fn routed_space(&self, url: &Url) -> Option<usize> {
        let from_private = self.model.current_space().is_none_or(|s| s.private);
        let space_id = browser_core::routing::route(&self.core.disk.space_rules().ok()?, url, from_private)?;
        self.model.spaces.iter().position(|s| !s.private && s.space.id == space_id).filter(|ix| *ix != self.model.space_ix)
    }

    pub fn site_rule(&self) -> Option<SiteRule> {
        let host = self.active_host().filter(|_| !self.model.private())?;
        let rule = self.core.disk.space_rules().unwrap_or_default().into_iter().find(|r| r.host_pattern == host);
        let space = rule.and_then(|r| self.model.spaces.iter().position(|s| !s.private && s.space.id == r.space_id));
        let choices = self.model.spaces.iter().enumerate().filter(|(_, s)| !s.private).map(|(ix, s)| (ix, s.space.name.clone())).collect();
        Some(SiteRule { space, choices })
    }

    pub fn set_site_rule(&mut self, ix: Option<usize>) -> bool {
        let Some(host) = self.active_host().filter(|_| !self.model.private()) else { return false };
        let disk = &self.core.disk;
        match ix.and_then(|ix| self.model.spaces.get(ix)).filter(|s| !s.private) {
            Some(space) => disk.set_space_rule(&host, space.space.id).is_ok(),
            None => {
                for rule in disk.space_rules().unwrap_or_default().into_iter().filter(|r| r.host_pattern == host) {
                    disk.delete_space_rule(rule.id).ok();
                }
                true
            }
        }
    }

    pub fn snapshots(&self) -> Vec<Snapshot> {
        match self.model.current_space().filter(|s| !s.private) {
            Some(space) => self.core.disk.snapshots(space.space.profile_id).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    pub fn save_snapshot(&mut self, name: &str, now: i64) -> bool {
        let saved = self.model.current_space().and_then(|space| snapshot::save(&self.core.disk, space, name, now));
        match saved {
            Some(_) => self.toast_as(browser_core::toast::Tone::Success, format!("Saved snapshot {name}")),
            None => self.notice = Some("This space has nothing to save.".into()),
        }
        saved.is_some()
    }

    pub fn restore_snapshot(&mut self, saved: &Snapshot) -> bool {
        if self.model.current_space().is_none_or(|s| s.private || s.space.profile_id != saved.profile_id) {
            self.toast_as(browser_core::toast::Tone::Warning, "Switch to a space of this snapshot's profile to restore it.".into());
            return false;
        }
        let store = &self.core.disk;
        let Ok(snapshot) = SpaceSnapshot::load(store, saved.profile_id, saved.id) else {
            self.toast_as(browser_core::toast::Tone::Warning, "This snapshot could not be read.".into());
            return false;
        };
        let taken: Vec<String> = self.model.spaces.iter().filter(|s| !s.private).map(|s| s.space.name.clone()).collect();
        let name = std::iter::once(saved.name.clone())
            .chain((2..50).map(|n| format!("{} {n}", saved.name)))
            .find(|n| !taken.contains(n))
            .unwrap_or_else(|| "Snapshot".into());
        let Ok(space) = store.create_space(saved.profile_id, &name, "") else { return false };
        let last = snapshot.restore(store, space.id);
        if let Some(id) = last {
            store.touch_tab(id, browser_core::now()).ok();
        }
        self.model.reload_spaces(&self.core);
        if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id) {
            self.switch_space(ix);
        }
        if let Some(id) = last {
            self.activate(TabKey { private: false, id });
        }
        true
    }

    pub fn delete_snapshot(&mut self, saved: &Snapshot) {
        self.core.disk.delete_snapshot(saved.profile_id, saved.id).ok();
    }

    pub fn save_for_later(&mut self, now: i64) -> bool {
        let (Some(id), Some(key)) = (self.profile_id(), self.model.active) else { return false };
        let url = self.tab_url(key);
        let title = self.pages.get(&key).map(|p| p.title.clone()).unwrap_or_default();
        let store = self.store();
        let saved = Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
            && bookmarks::save_for_later(store, id, &store.bookmarks(id).unwrap_or_default(), &url, &title, now);
        let (tone, text) = browser_core::toast::reading_list(saved);
        self.toast_as(tone, text.into());
        saved
    }

    pub fn reading_list(&self) -> Vec<Bookmark> {
        let Some(id) = self.profile_id() else { return Vec::new() };
        bookmarks::reading_list_pages(self.store().bookmarks(id).unwrap_or_default())
    }

    pub fn remove_from_reading_list(&mut self, id: Id) {
        self.store().delete_bookmark(id).ok();
    }

    pub fn site_dark(&self) -> bool {
        match (self.profile_id(), self.active_host()) {
            (Some(id), Some(host)) => self.store().site_is_dark(id, &host).unwrap_or(false),
            _ => false,
        }
    }

    pub fn site_css(&self) -> String {
        match (self.profile_id(), self.active_host()) {
            (Some(id), Some(host)) => self.store().site_css_for(id, &host).unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn set_site_dark(&mut self, dark: bool) -> bool {
        let (Some(id), Some(host)) = (self.profile_id(), self.active_host()) else { return false };
        let saved = self.store().set_site_dark(id, &host, dark).is_ok();
        if saved {
            self.reload_host(&host);
        }
        saved
    }

    pub fn set_site_css(&mut self, css: &str) -> bool {
        let (Some(id), Some(host)) = (self.profile_id(), self.active_host()) else { return false };
        let css = css.replace(['\n', '\r'], " ");
        if css.len() > MAX_SITE_CSS || self.store().set_site_css(id, &host, css.trim()).is_err() {
            self.toast_as(browser_core::toast::Tone::Warning, "Tarn could not save this style sheet.".into());
            return false;
        }
        self.reload_host(&host);
        true
    }

    pub fn site_zaps(&self) -> usize {
        match (self.profile_id(), self.active_host()) {
            (Some(id), Some(host)) => self.store().zapped_elements(id).unwrap_or_default().into_iter().filter(|(h, _)| *h == host).count(),
            _ => 0,
        }
    }

    pub fn clear_site_zaps(&mut self) {
        let (Some(id), Some(host)) = (self.profile_id(), self.active_host()) else { return };
        if self.store().unzap_element(id, &host, None).is_ok() {
            self.reload_host(&host);
        }
    }

    pub(crate) fn reload_host(&mut self, host: &str) {
        let keys: Vec<(TabKey, String)> = self
            .model
            .lifecycle
            .live()
            .filter(|k| self.model.tab(*k).is_some())
            .filter_map(|k| self.pages.get(&k).and_then(|p| p.committed.clone()).map(|u| (k, u)))
            .filter(|(_, u)| Url::parse(u).ok().and_then(|u| browser_core::site::site_host(&u).map(str::to_owned)).as_deref() == Some(host))
            .collect();
        for (key, url) in keys {
            self.load_in(key, &url, None);
        }
    }

    pub fn share_url(&self) -> Option<String> {
        let url = self.active_url();
        Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https")).map(|_| url)
    }

    pub fn site_shield_total(&self) -> i64 {
        match (self.model.active_profile(&self.core), Url::parse(&self.active_url())) {
            (Some((private, id)), Ok(url)) => self.core.site_stopped(private, id, &url),
            _ => 0,
        }
    }

    pub fn qr_code(&self) -> Option<(Vec<Vec<bool>>, String)> {
        let url = self.share_url()?;
        let code = engine::qr::encode(&url)?;
        Some((code, browser_core::command::display_host(&url)))
    }

    pub fn search_shortcuts(&self) -> bool {
        browser_core::settings::search_shortcuts(&self.core.disk)
    }

    pub fn setting_on(&self, key: &str) -> bool {
        self.core.disk.setting(key).ok().flatten().is_some_and(|v| v == "on")
    }

    pub fn set_setting_on(&mut self, key: &str, on: bool) -> Vec<Effect> {
        self.core.disk.set_setting(key, if on { "on" } else { "off" }).ok();
        if key == crate::scripts::HISTORY_TEXT && !on {
            self.core.disk.clear_history_text().ok();
        }
        Vec::new()
    }

    pub fn tabs_on_top(&self) -> bool {
        self.tab_placement == browser_core::layout::TabPlacement::Top
    }

    pub fn switch_on(&self, switch: browser_core::prefs::Switch) -> bool {
        switch.on(&self.core.disk)
    }

    pub fn set_switch(&mut self, switch: browser_core::prefs::Switch, on: bool) {
        switch.set(&self.core.disk, on);
    }

    pub fn set_style(&mut self, modern: bool) {
        browser_core::prefs::set_style(&self.core.disk, modern);
    }

    /// The profile whose latest chats the sidebar lists: its AI is on and it is not private.
    pub fn chat_profile(&self) -> Option<store::Id> {
        // Listing titles sends nothing, so a site at Lockdown does not hide them; the profile's level does.
        let origin = self.model.assistant_origin(&self.core, None).filter(|o| browser_core::assistant::allowed(*o))?;
        self.core.disk.profile(origin.profile).is_ok_and(|p| p.ai_enabled).then_some(origin.profile)
    }

    /// The Modern sidebar of the Mac layout: Chats, Tidy, cards and the footer, while the tabs sit
    /// in the sidebar. The address stays in the toolbar on touch.
    pub fn sidebar_chrome(&self) -> bool {
        !self.tabs_on_top() && self.switch_on(browser_core::prefs::SIDEBAR_TOOLBAR)
    }

    /// Tabs on top hide the sidebar of the Mac layout; back at the side it shows as it was.
    pub fn toggle_tabs_on_top(&mut self) {
        self.tab_placement = self.tab_placement.toggled(&self.core.disk);
        browser_core::prompts::restart(&mut self.transfers_moved, std::time::Instant::now() + tarn_nsh_shared::motion::SLIDE);
    }

    pub fn toggle_sidebar(&mut self) {
        if self.tabs_on_top() {
            return self.toggle_tabs_on_top();
        }
        self.sidebar_open = !self.sidebar_open;
        // The downloads pop-over hangs from the sidebar, so the slide moves its Keep button.
        browser_core::prompts::restart(&mut self.transfers_moved, std::time::Instant::now() + tarn_nsh_shared::motion::SLIDE);
        if let Err(e) = browser_core::layout::save_sidebar(&self.core.disk, self.sidebar_open) {
            self.toast_as(browser_core::toast::Tone::Warning, format!("The sidebar setting could not be saved: {e}"));
        }
    }

    pub fn toggle_pin(&mut self, key: TabKey) {
        if self.model.tab(key).is_some_and(|t| browser_core::tab_menu::pinnable(t.kind, self.model.burner_shown())) {
            browser_core::sidebar::toggle_pin(&mut self.model, &self.core, key);
        }
    }

    pub fn set_layout(&mut self, layout: browser_core::layout::Layout) -> Vec<Effect> {
        self.layout = layout;
        if let Err(e) = layout.save(&self.core.disk) {
            self.toast_as(browser_core::toast::Tone::Warning, format!("The layout could not be saved: {e}"));
        }
        Vec::new()
    }
}
