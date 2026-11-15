//! A profile's website data on Windows: its own Chromium profile below the Chromium root, or an
//! in-memory one for private and Lockdown stores.

use std::cell::Cell;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use cef::*;
use shield::ShieldLevel;

use crate::chromium::{jar, runtime};

use crate::Event;
use crate::extensions::{Enabled, Extension};
use crate::request_filter::Shield;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A profile's style sheets and hidden elements, each by host.
type ProfileStyles = (Vec<(String, String)>, Vec<(String, String)>);

thread_local! {
    /// Each profile's style sheets and hidden elements, by Shield key: profiles that share a store
    /// keep their own.
    static STYLES: std::cell::RefCell<std::collections::HashMap<String, ProfileStyles>> = Default::default();
    /// Each profile's page-world script of filtered views (the YouTube script), by Shield key.
    static PAGE_SCRIPTS: std::cell::RefCell<std::collections::HashMap<String, String>> = Default::default();
}

pub(crate) fn page_script_of(profile: &str) -> Option<String> {
    PAGE_SCRIPTS.with_borrow(|s| s.get(profile).cloned())
}

#[cfg(test)]
pub(crate) fn set_page_script_for_test(profile: &str, source: Option<&str>) {
    PAGE_SCRIPTS.with_borrow_mut(|s| match source {
        Some(source) => s.insert(profile.to_owned(), source.to_owned()),
        None => s.remove(profile),
    });
}

/// The style sheets and hidden elements of the profile with this Shield key, plus the filter
/// lists' element hiding for `page`, as one start-of-document script; `None` without any.
pub(crate) fn site_styles_of(profile: &str, page: Option<&url::Url>) -> Option<String> {
    let mut css = STYLES
        .with_borrow(|s| {
            let (styles, hidden) = s.get(profile)?;
            Some(
                styles
                    .iter()
                    .cloned()
                    .chain(hidden.iter().map(|(host, selector)| (host.clone(), format!("{selector} {{ display: none !important; }}"))))
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();
    css.extend(page.and_then(|page| list_hiding(profile, page)));
    crate::page_scripts::site_styles_script(&css)
}

/// The switched-on lists' element hiding for `page`, by its exact host; none where the profile's
/// Shield pauses filtering, as WebKit leaves the rule lists out of paused sites' controllers.
fn list_hiding(profile: &str, page: &url::Url) -> Option<(String, String)> {
    let shield = crate::chromium::shield_for(profile)?;
    let host = page.host_str().filter(|_| matches!(page.scheme(), "http" | "https") && (shield.site_for)(page).filtered)?;
    Some((host.trim_end_matches('.').to_owned(), shield.matcher.hide_css(host))).filter(|(_, css)| !css.is_empty())
}

#[derive(Clone)]
pub struct DataStore {
    key: String,
    /// Names this profile's Shield choices; profiles that share a store keep their own.
    shield: Rc<ShieldKey>,
    /// The profile's store UUID, for a store kept on disk.
    uuid: Option<String>,
    downloads: Rc<dyn Fn(Event)>,
}

/// Removes the profile's Shield choices with its last store handle; its views then fail closed.
struct ShieldKey(String);

impl Drop for ShieldKey {
    fn drop(&mut self) {
        crate::chromium::drop_shield(&self.0);
        STYLES.with_borrow_mut(|s| s.remove(&self.0));
        PAGE_SCRIPTS.with_borrow_mut(|s| s.remove(&self.0));
    }
}

fn next_shield() -> Rc<ShieldKey> {
    Rc::new(ShieldKey(format!("s:{}", NEXT.fetch_add(1, Ordering::Relaxed))))
}

/// A store UUID names a folder, so it may hold nothing but letters, digits and dashes.
fn valid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.len() <= 64 && uuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn folder(uuid: &str) -> Option<PathBuf> {
    valid(uuid).then(|| runtime::root().map(|root| root.join(uuid))).flatten()
}

impl DataStore {
    pub fn persistent(uuid: &str, _extensions: &str, downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        valid(uuid).then(|| DataStore { key: format!("p:{uuid}"), shield: next_shield(), uuid: Some(uuid.to_owned()), downloads })
    }

    pub fn ephemeral(downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Some(DataStore { key: format!("e:{n}"), shield: next_shield(), uuid: None, downloads })
    }

    pub(crate) fn context_key(&self) -> &str {
        &self.key
    }

    pub(crate) fn downloads(&self) -> Rc<dyn Fn(Event)> {
        self.downloads.clone()
    }

    pub(crate) fn shield_key(&self) -> &str {
        &self.shield.0
    }

    /// Where Chromium keeps this store; `None` keeps it in memory.
    pub(crate) fn cache_dir(&self) -> Option<PathBuf> {
        self.uuid.as_deref().and_then(folder)
    }

    /// Replaces the Shield choices this store's requests are checked against; until the first
    /// call every request of its views is cancelled.
    pub fn set_shield(&self, shield: Shield) {
        crate::chromium::set_shield_for(&self.shield.0, shield);
        crate::chromium::view::restyle(&self.shield.0);
    }

    /// Chromium's Safe Browsing needs Google's service, which CEF does not reach; Tarn's malware
    /// list is applied by Shield instead.
    pub fn set_safe_browsing(&self, _on: bool) {}

    /// Sets the page-world script of filtered views (the YouTube script), replacing the previous
    /// one; open pages get it on their next load.
    pub fn set_page_script(&self, source: Option<&str>) {
        let key = &self.shield.0;
        let old = PAGE_SCRIPTS.with_borrow_mut(|s| match source {
            Some(source) => s.insert(key.clone(), source.to_owned()),
            None => s.remove(key),
        });
        if old.as_deref() != source {
            crate::chromium::view::rescript(key);
        }
    }

    /// Sets the style sheets by host pattern (`page_scripts::site_styles_script`), replacing the
    /// previous ones; open pages get them at once, so no view needs remaking and this is never true.
    pub fn set_site_styles(&self, styles: &[(String, String)]) -> bool {
        if STYLES.with_borrow_mut(|s| std::mem::replace(&mut s.entry(self.shield.0.clone()).or_default().0, styles.to_vec()) != styles) {
            crate::chromium::view::restyle(&self.shield.0);
        }
        false
    }

    /// Hides elements by exact host and selector (Zap Element), replacing the previous ones; open
    /// pages hide them at once. WebKit does this with a content rule list; Chromium gets a style sheet.
    pub fn set_hidden_elements(&self, hidden: &[(String, String)]) {
        if STYLES.with_borrow_mut(|s| std::mem::replace(&mut s.entry(self.shield.0.clone()).or_default().1, hidden.to_vec()) != hidden) {
            crate::chromium::view::restyle(&self.shield.0);
        }
    }

    /// Requests of this profile's views the filter cancelled since the last call, per host of the page shown.
    pub fn take_blocked(&self) -> std::collections::BTreeMap<String, i64> {
        crate::chromium::take_blocked_for(&self.shield.0)
    }

    /// New downloads go into a Downloads subfolder by file type.
    pub fn set_sort_downloads(&self, on: bool) {
        crate::chromium::downloads_windows::set_sort(&self.key, on);
    }

    /// `id` is from `Event::DownloadProgress`; Chromium keeps what it needs to continue.
    pub fn pause_download(&self, id: u64) {
        crate::chromium::downloads_windows::pause(&self.key, id);
    }

    pub fn resume_download(&self, id: u64) {
        crate::chromium::downloads_windows::resume(&self.key, id);
    }

    /// For quit: cancels this store's downloads, which Chromium cannot continue after relaunch.
    pub fn quit_downloads(&self) {
        crate::chromium::downloads_windows::quit(&self.key);
    }

    pub fn set_extensions(&self, _enabled: &[Enabled], _blocked: &[String], _level: ShieldLevel) {}

    pub fn remove_extension(&self, _key: &str, done: impl FnOnce() + 'static) {
        done();
    }

    pub fn running_extensions(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn open_extension_popup(&self, _extension: &Extension) {}

    pub fn remove_extension_data(&self, done: impl FnOnce() + 'static) {
        done();
    }

    pub fn wipe_extensions(_uuid: &str) {}

    /// Writes cookies from another browser into this store's Chromium jar. `done` gets how many
    /// were written and how many were refused by `cookies::admit_imported` or by Chromium; all
    /// count as refused when Chromium cannot run or its jar cannot be read.
    pub fn import_cookies(&self, cookies: Vec<crate::cookies::Cookie>, now: i64, done: impl FnOnce(usize, usize) + 'static) {
        let tally = Rc::new(Tally { total: cookies.len(), written: Cell::new(0), done: Some(Box::new(done)) });
        if self.uuid.is_none() || !runtime::root().is_some_and(|root| runtime::ensure_started(&root)) {
            return;
        }
        runtime::with_store_context(self.key.clone(), self.shield_key().to_owned(), self.cache_dir(), move |context| {
            let Some(manager) = context.cookie_manager(None) else { return };
            jar::visit(Some(manager.clone()), HashSet::new(), move |existing| {
                let Some(existing) = existing else { return };
                for c in crate::cookies::admit_imported(cookies, &existing, now).0 {
                    manager.set_cookie(Some(&jar::url(&c)), Some(&jar::to_cef(&c)), Some(&mut Written::new(tally.clone())));
                }
            });
        });
    }

    /// Deletes the store's folder now when no view uses it. A live profile is emptied of what
    /// Chromium can clear at run time, and its folder goes before CEF starts at the next launch.
    pub fn wipe_persistent(uuid: &str) {
        if !delete_folder(uuid) {
            mark_wiped(uuid);
            with_live(&format!("p:{uuid}"), empty);
        }
    }

    /// Deletes the store's folder; `done(false)` while Chromium still has it open, so the shell
    /// retries at a later launch.
    pub fn delete_persistent(uuid: &str, done: impl FnOnce(bool) + 'static) {
        done(delete_folder(uuid));
    }

    pub fn remove_all_data(&self, done: impl FnOnce() + 'static) {
        // A store Chromium has not opened this launch goes as a whole. A live one stays, as the
        // profile keeps using it; storage goes for the sites its open tabs show.
        if runtime::store_context(&self.key).is_none() {
            if let Some(uuid) = self.uuid.as_deref().filter(|u| !delete_folder(u)) {
                mark_wiped(uuid);
            }
        } else {
            crate::chromium::view::clear_store_storage(&self.key);
            with_live(&self.key, empty);
        }
        done();
    }

    pub fn clear_cache(&self) {
        with_live(&self.key, |context| context.clear_http_cache(None));
    }

    /// Removes `host`'s cookies (its parent domains and subdomains included), the storage of its
    /// origins through an open tab of this store, and the HTTP cache, which Chromium cannot clear
    /// per site. `done` gets the number of cookies removed, plus one if storage was cleared.
    pub fn clear_site_data(&self, host: &str, done: impl FnOnce(usize) + 'static) {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let removed = Arc::new(AtomicUsize::new(usize::from(crate::chromium::view::clear_site_storage(&self.key, &host))));
        let counted = removed.clone();
        with_live(&self.key, move |context| {
            if let Some(cookies) = context.cookie_manager(None) {
                cookies.visit_all_cookies(Some(&mut SiteCookies::new(host, counted)));
            }
            context.clear_http_cache(None);
        });
        // shortcut: the visit has no completion callback, so the count is read once it has had
        // time to run; upgrade when the shell needs an exact number.
        runtime::after(Duration::from_millis(500), move || done(removed.load(Ordering::Relaxed)));
    }

    /// Filter lists reach Chromium through the matcher in `set_shield`.
    pub fn set_rule_lists(&self, _group: &str, _lists: Option<&FilterLists>, _pausable: bool) {}
}

const WIPED: &str = "WIPED";

fn mark_wiped(uuid: &str) {
    let Some(root) = runtime::root() else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(root.join(WIPED)) {
        let _ = writeln!(file, "{uuid}");
    }
}

/// Deletes the folders of stores wiped while Chromium had them open; call before CEF starts.
pub(crate) fn sweep_wiped(root: &Path) {
    let list = root.join(WIPED);
    let Ok(text) = std::fs::read_to_string(&list) else { return };
    let kept: Vec<&str> =
        text.lines().filter(|u| valid(u) && std::fs::remove_dir_all(root.join(u)).is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound)).collect();
    let _ = if kept.is_empty() { std::fs::remove_file(&list) } else { std::fs::write(&list, kept.join("\n") + "\n") };
}

fn delete_folder(uuid: &str) -> bool {
    if runtime::store_context(&format!("p:{uuid}")).is_some() {
        return false;
    }
    folder(uuid).is_some_and(|dir| match std::fs::remove_dir_all(&dir) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    })
}

fn with_live(key: &str, then: impl FnOnce(RequestContext)) {
    if let Some(context) = runtime::store_context(key) {
        then(context);
    }
}

fn empty(context: RequestContext) {
    if let Some(cookies) = context.cookie_manager(None) {
        cookies.delete_cookies(None, None, None);
    }
    context.clear_http_cache(None);
    context.clear_http_auth_credentials(None);
    context.clear_certificate_exceptions(None);
    context.close_all_connections(None);
}

/// Reports an import once Chromium has released every write's callback; an unanswered write
/// counts as refused.
struct Tally {
    total: usize,
    written: Cell<usize>,
    done: Option<Box<dyn FnOnce(usize, usize)>>,
}

impl Drop for Tally {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            done(self.written.get(), self.total - self.written.get());
        }
    }
}

wrap_set_cookie_callback! {
    struct Written {
        tally: Rc<Tally>,
    }

    impl SetCookieCallback {
        fn on_complete(&self, success: i32) {
            if success != 0 {
                self.tally.written.set(self.tally.written.get() + 1);
            }
        }
    }
}

wrap_cookie_visitor! {
    struct SiteCookies {
        host: String,
        counted: Arc<AtomicUsize>,
    }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&Cookie>, _count: i32, _total: i32, delete_cookie: Option<&mut i32>) -> i32 {
            if let (Some(cookie), Some(delete)) = (cookie, delete_cookie) {
                let domain = cookie.domain.to_string();
                let domain = domain.trim_start_matches('.');
                if crate::page::site_matches(&self.host, domain) || crate::page::site_matches(domain, &self.host) {
                    *delete = 1;
                    self.counted.fetch_add(1, Ordering::Relaxed);
                }
            }
            1
        }
    }
}

/// WebKit rule lists are not used on Windows; lists apply through Shield's matcher.
pub struct FilterLists(());

impl FilterLists {
    pub fn look_up(_group: &str, _key: &str, _count: usize, done: impl FnOnce(Result<FilterLists, String>) + 'static) {
        done(Ok(FilterLists(())));
    }

    pub fn compile(_group: &str, _key: &str, _chunks: Vec<String>, done: impl FnOnce(Result<FilterLists, String>) + 'static) {
        done(Ok(FilterLists(())));
    }

    pub fn remove(_group: &str) {}
}

pub fn builtin_rules_failed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_plain_uuids_name_a_folder() {
        assert!(super::valid("0b8f3c1e-2d4a-4f6b-9c7d-1e2f3a4b5c6d"));
        for bad in ["", "..", "a/b", "a\\b", "C:", "a b", &"x".repeat(65)] {
            assert!(!super::valid(bad), "{bad}");
        }
    }

    #[test]
    fn wiped_stores_go_and_only_plain_names_are_followed() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        std::fs::create_dir_all(root.join("aa-1/Cache")).unwrap();
        std::fs::create_dir_all(root.join("keep")).unwrap();
        std::fs::write(root.join(super::WIPED), "aa-1\n..\nmissing\n").unwrap();
        super::sweep_wiped(root);
        assert!(!root.join("aa-1").exists());
        assert!(root.join("keep").is_dir() && root.exists());
        assert!(!root.join(super::WIPED).exists());
    }

    #[test]
    fn an_import_reports_once_its_last_write_is_released() {
        let report = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = report.clone();
        let tally = std::rc::Rc::new(super::Tally { total: 3, written: std::cell::Cell::new(0), done: Some(Box::new(move |w, r| slot.set(Some((w, r))))) });
        let write = tally.clone();
        drop(tally);
        write.written.set(1);
        assert_eq!(report.get(), None);
        drop(write);
        assert_eq!(report.get(), Some((1, 2)), "unanswered writes count as refused");
    }
}

#[cfg(test)]
mod style_tests {
    #[test]
    fn a_profile_s_sheets_and_hidden_elements_make_one_replaceable_script() {
        let row = |h: &str, s: &str| (h.to_owned(), s.to_owned());
        super::STYLES.with_borrow_mut(|s| {
            s.insert("s:test".into(), (vec![row("a.test", "body { color: red; }")], vec![row("a.test", "#ad"), row("b.test", ".banner")]))
        });
        let script = super::site_styles_of("s:test", None).unwrap();
        assert!(script.contains("color: red") && script.contains("#ad { display: none !important; }") && script.contains(".banner"), "{script}");
        assert!(script.contains("style[data-tarn-site]") && script.contains("tarnSite"));
        assert!(super::site_styles_of("s:other", None).is_none());
    }

    #[test]
    fn list_hiding_follows_the_page_host_and_the_pause() {
        use crate::nav_policy::{PopupRule, Site};
        let site_for = |url: &url::Url| Site {
            level: shield::ShieldLevel::Standard,
            filtered: url.host_str() != Some("paused.test"),
            blocked: None,
            malware: false,
            popups: PopupRule::Default,
        };
        let matcher = shield::matcher::Matcher::new(&["##.ad\nnews.test##.promo\n"], &[], &[]);
        crate::chromium::set_shield_for("s:lists", crate::request_filter::Shield { site_for: Box::new(site_for), matcher });
        let page = |u: &str| url::Url::parse(u).unwrap();
        let news = super::site_styles_of("s:lists", Some(&page("https://www.news.test/a"))).unwrap();
        assert!(news.contains("www.news.test") && news.contains(".ad{display:none!important}") && news.contains(".promo{"), "{news}");
        let other = super::site_styles_of("s:lists", Some(&page("https://other.test/"))).unwrap();
        assert!(other.contains(".ad{") && !other.contains(".promo"), "{other}");
        for none in ["https://paused.test/", "file:///C:/a.html", "about:blank"] {
            assert!(super::site_styles_of("s:lists", Some(&page(none))).is_none(), "{none}");
        }
        assert!(super::site_styles_of("s:none", Some(&page("https://news.test/"))).is_none());
        crate::chromium::drop_shield("s:lists");
    }
}
