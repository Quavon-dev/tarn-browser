pub mod ext;
pub mod filters;
pub use engine::view_policy as policy;
mod scripts;
pub mod session;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Instant;

use browser_core::site::SiteRules;
use engine::nav_policy::{self, GateAction, PopupRule, Site};
use nsh_core::JsonValue;
use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::{ComponentContext, NativeComponent};
use serde_json::json;
use shield::{Category, ShieldLevel};
use url::Url;
use webkit6::prelude::*;
use webkit6::{
    AuthenticationRequest, AuthenticationScheme, Credential, CredentialPersistence, Download, FindOptions, GeolocationPermissionRequest, LoadEvent,
    MediaCaptureState, NavigationPolicyDecision, NavigationType, NetworkError, PermissionRequest, PolicyDecisionType, PolicyError, ResponsePolicyDecision,
    ScriptDialog, ScriptDialogType, SnapshotOptions, SnapshotRegion, UserContentFilter, UserContentManager, UserMediaPermissionRequest,
    WebProcessTerminationReason, WebView, gio, glib,
};

use self::policy::{Burst, Levels, Navigation, Verdict};
use crate::data::StoreKey;

pub const NAME: &str = "tarn.webview";
const FIND_MAX: u32 = 1000;
const SHIFT_MASK: u32 = 1;
const CONTROL_MASK: u32 = 1 << 2;
const SNAPSHOT_WIDTH: i32 = 752;

thread_local! {
    static VIEWS: RefCell<Vec<Weak<State>>> = RefCell::default();
}

enum Pending {
    Dialog(ScriptDialog),
    Permission(PermissionRequest),
    Credentials(AuthenticationRequest),
}

#[derive(Clone, PartialEq)]
struct Build {
    key: StoreKey,
    level: ShieldLevel,
    lists: Vec<String>,
    inspectable: bool,
}

#[derive(Default)]
struct State {
    context: Option<ComponentContext>,
    container: Option<gtk::Box>,
    view: RefCell<Option<WebView>>,
    built: RefCell<Option<Build>>,
    tab: RefCell<String>,
    profile: RefCell<String>,
    level: Cell<ShieldLevel>,
    inspectable: Cell<bool>,
    levels: RefCell<Levels>,
    lists: RefCell<Vec<String>>,
    rules: SiteRules,
    unfiltered: Cell<bool>,
    main_pending: Cell<bool>,
    load: RefCell<Option<(String, u64)>>,
    loaded_seq: Cell<Option<u64>>,
    shell_url: RefCell<Option<String>>,
    shell_file: Cell<bool>,
    opened_files: RefCell<std::collections::HashSet<String>>,
    link_url: RefCell<Option<String>>,
    load_link: Cell<bool>,
    load_file: Cell<bool>,
    upgraded: RefCell<Option<(String, bool, bool)>>,
    user_step: Cell<bool>,
    posted: RefCell<Option<Url>>,
    page: RefCell<Option<Url>>,
    failed: Cell<bool>,
    scheduled: Cell<bool>,
    silenced: Cell<bool>,
    commits: Cell<u64>,
    prompts: RefCell<HashMap<u64, Pending>>,
    next_prompt: Cell<u64>,
    bursts: RefCell<Rc<Bursts>>,
    download_burst: RefCell<Burst>,
    auth_failures: RefCell<HashMap<String, u32>>,
    find_text: RefCell<String>,
    zoom: Cell<f64>,
    muted: Cell<bool>,
    content: RefCell<Option<UserContentManager>>,
    style_script: RefCell<Option<webkit6::UserScript>>,
    page_scripts: RefCell<Vec<webkit6::UserScript>>,
    ext: ext::Ext,
    styles: RefCell<Vec<(String, String)>>,
    theme_styles: RefCell<Vec<(String, String)>>,
    zaps: RefCell<Vec<(String, String)>>,
    zap_filter: RefCell<Option<String>>,
    sort_downloads: Cell<bool>,
    youtube: Cell<bool>,
    processes: RefCell<std::collections::HashSet<i32>>,
    process: Cell<Option<i32>>,
    #[cfg(test)]
    events: RefCell<Vec<(String, JsonValue)>>,
}

#[derive(Default)]
struct Bursts {
    prompts: RefCell<Burst>,
    popups: RefCell<Burst>,
}

impl State {
    fn emit(&self, name: &str, data: JsonValue) {
        if cfg!(debug_assertions) && std::env::var_os("TARN_LOG_EVENTS").is_some() {
            eprintln!("[tarn] event {name} {data}");
        }
        #[cfg(test)]
        self.events.borrow_mut().push((name.to_owned(), data.clone()));
        if let Some(context) = &self.context {
            context.emit(name, data);
        }
    }

    fn shielded(&self, kind: &str, url: &str) {
        self.emit("shielded", json!({ "kind": kind, "url": url }));
    }

    fn note_processes(&self) {
        *self.processes.borrow_mut() = crate::memory::web_processes(std::path::Path::new("/proc"), std::process::id() as i32);
    }

    fn claim_process(&self) {
        let now = crate::memory::web_processes(std::path::Path::new("/proc"), std::process::id() as i32);
        let new: Vec<i32> = now.difference(&self.processes.borrow()).copied().collect();
        *self.processes.borrow_mut() = now;
        if let [pid] = new[..]
            && self.process.replace(Some(pid)) != Some(pid)
        {
            self.emit("process", json!({ "pid": pid }));
        }
    }

    fn container(&self) -> &gtk::Box {
        self.container.as_ref().expect("made in new")
    }

    fn web(&self) -> Option<WebView> {
        self.view.borrow().clone()
    }

    fn page_state(&self, web: &WebView) -> JsonValue {
        json!({
            "url": web.uri().map(|u| u.to_string()).unwrap_or_default(),
            "title": policy::title(&web.title().unwrap_or_default()),
            "canGoBack": web.can_go_back(),
            "canGoForward": web.can_go_forward(),
        })
    }

    fn admit_prompt(&self) -> bool {
        self.bursts.borrow().prompts.borrow_mut().admit(policy::PROMPT_BURST, Instant::now())
    }

    fn admit_popup(&self) -> bool {
        self.bursts.borrow().popups.borrow_mut().admit(policy::POPUP_BURST, Instant::now())
    }

    fn site(&self, url: &Url) -> Site {
        filters::site(self.levels.borrow().level(url), &self.rules, url)
    }

    fn listed(&self, url: &str, category: Category) {
        self.main_pending.set(false);
        self.emit("blocked", json!({ "url": url, "reason": "listed", "category": category.as_str() }));
    }

    fn origin(&self) -> String {
        self.page.borrow().as_ref().and_then(policy::origin).unwrap_or_default()
    }

    fn decline_all(&self) {
        for (_, pending) in self.prompts.borrow_mut().drain() {
            decline(pending);
        }
    }

    fn drop_view(&self) {
        self.decline_all();
        ext::detach(self);
        self.style_script.replace(None);
        self.page_scripts.take();
        self.content.replace(None);
        if let Some(old) = self.view.replace(None) {
            old.stop_loading();
            if old.parent().is_some() {
                self.container().remove(&old);
            }
        }
    }
}

impl session::DownloadSink for State {
    fn tab(&self) -> String {
        self.tab.borrow().clone()
    }

    fn admit_download(&self, now: Instant) -> bool {
        self.download_burst.borrow_mut().admit(policy::DOWNLOAD_BURST, now)
    }

    fn sort_downloads(&self) -> bool {
        self.sort_downloads.get()
    }

    fn page(&self) -> Option<Url> {
        self.page.borrow().clone()
    }

    fn malware(&self) -> Option<std::sync::Arc<shield::domains::DomainSet>> {
        self.lists.borrow().iter().any(|l| l == "malware").then(filters::malware).flatten()
    }
}

fn decline(pending: Pending) {
    match pending {
        Pending::Dialog(dialog) => {
            if matches!(dialog.dialog_type(), ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm) {
                dialog.confirm_set_confirmed(false);
            }
            dialog.close();
        }
        Pending::Permission(request) => request.deny(),
        Pending::Credentials(request) => request.cancel(),
    }
}

/// `set_dark` (`docs/shared-core.md` §1). WebKitGTK takes `prefers-color-scheme` from GTK's
/// settings, which NSH sets from the app's colour scheme, so it is set for every view at once.
/// While the theme follows the system the system's scheme is the theme's, and stays followed so
/// NSH keeps reporting the system's appearance.
pub fn set_dark(follows_system: bool, dark: bool) {
    use nsh_core::platform::Appearance;
    nsh_core::prelude::set_color_scheme(match (follows_system, dark) {
        (true, _) => Appearance::System,
        (false, true) => Appearance::Dark,
        (false, false) => Appearance::Light,
    });
}

pub fn adopt_download(download: &Download) {
    let owner = download.web_view();
    let state = VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).find(|s| owner.is_some() && s.web() == owner));
    let url = download.request().and_then(|r| r.uri()).and_then(|u| Url::parse(&u).ok());
    match state {
        Some(state) if !url.is_some_and(|u| nav_policy::gate(state.site(&u).blocked, false, false, true) == GateAction::Cancel) => {
            session::track(download, &*state)
        }
        _ => download.cancel(),
    }
}

fn sets_changed() {
    let views: Vec<Rc<State>> = VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).collect());
    for state in views {
        let Some(url) = state.page.borrow().clone() else { continue };
        if let Some(category) = state.site(&url).blocked.filter(|c| c.warns()) {
            if let Some(web) = state.web() {
                web.stop_loading();
            }
            state.listed(url.as_str(), category);
        }
    }
}

fn schedule(state: &Rc<State>) {
    if state.scheduled.replace(true) {
        return;
    }
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade() {
            state.scheduled.set(false);
            apply(&state);
        }
    });
}

fn apply(state: &Rc<State>) {
    let Some(key) = StoreKey::parse(&state.profile.borrow()) else {
        state.drop_view();
        *state.built.borrow_mut() = None;
        return state.emit("failed", json!({ "url": "", "message": "This profile's data store is not valid." }));
    };
    let unfiltered = state.unfiltered.get();
    let youtube = state.youtube.get().then(|| shield::groups::YOUTUBE_GROUP.to_owned());
    let lists = state.lists.borrow().iter().cloned().chain(youtube).filter(|g| !unfiltered || !shield::groups::pausable(g)).collect();
    let level = state.level.get();
    let wanted = Build { key, level, lists, inspectable: state.inspectable.get() && level != ShieldLevel::Lockdown };
    if state.built.borrow().as_ref() == Some(&wanted) {
        if state.web().is_some() {
            load(state, None);
        }
        return;
    }
    let current = state.page.borrow().as_ref().map(Url::to_string);
    state.drop_view();
    *state.built.borrow_mut() = Some(wanted.clone());
    let weak = Rc::downgrade(state);
    filters::with_filters(wanted.level, wanted.lists.clone(), move |result| {
        let Some(state) = weak.upgrade() else { return };
        if state.built.borrow().as_ref() != Some(&wanted) {
            return;
        }
        match result {
            Ok(filters) => {
                build(&state, &wanted, &filters);
                load(&state, current);
            }
            Err(e) => {
                *state.built.borrow_mut() = None;
                state.emit("failed", json!({ "url": "", "message": format!("Shield rules did not compile: {e}") }));
            }
        }
    });
}

fn build(state: &Rc<State>, wanted: &Build, filters: &[UserContentFilter]) {
    let Some(session) = session::acquire(&wanted.key) else {
        *state.built.borrow_mut() = None;
        return state.emit("failed", json!({ "url": "", "message": "The profile's data store could not be opened." }));
    };
    let content = UserContentManager::new();
    filters.iter().for_each(|f| content.add_filter(f));
    scripts::apply_styles(state, &content);
    scripts::apply_page_scripts(state, &content);
    *state.content.borrow_mut() = Some(content.clone());
    *state.zap_filter.borrow_mut() = None;
    apply_zaps(state);
    ext::attach(state);
    let web = WebView::builder().network_session(&session).user_content_manager(&content).build();
    if let Some(settings) = WebViewExt::settings(&web) {
        settings.set_enable_javascript(wanted.level.policy().javascript);
        settings.set_javascript_can_open_windows_automatically(false);
        settings.set_enable_developer_extras(wanted.inspectable);
    }
    let swipes = |web: &WebView, on: bool| WebViewExt::settings(web).into_iter().for_each(|s| s.set_enable_back_forward_navigation_gestures(on));
    web.connect_map(move |web| swipes(web, true));
    web.connect_unmap(move |web| swipes(web, false));
    web.set_hexpand(true);
    web.set_vexpand(true);
    if state.zoom.get() > 0.0 {
        web.set_zoom_level(state.zoom.get());
    }
    web.set_is_muted(state.muted.get());
    connect(state, &web);
    state.note_processes();
    attach_when_sized(state, &web);
    *state.view.borrow_mut() = Some(web);
}

fn attach_when_sized(state: &Rc<State>, web: &WebView) {
    let weak = Rc::downgrade(state);
    let web = web.clone();
    state.container().add_tick_callback(move |container, _| {
        let current = weak.upgrade().is_some_and(|s| s.web().as_ref() == Some(&web));
        if current && (container.width() == 0 || container.height() == 0) {
            return glib::ControlFlow::Continue;
        }
        if current {
            container.append(&web);
        }
        glib::ControlFlow::Break
    });
}

fn swap_group(name: &str, old: &[UserContentFilter], new: &[UserContentFilter]) {
    let views: Vec<Rc<State>> = VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).collect());
    for state in views {
        let uses = state.built.borrow().as_ref().is_some_and(|b| b.lists.iter().any(|l| l == name));
        let Some(content) = state.content.borrow().clone().filter(|_| uses) else { continue };
        old.iter().filter_map(UserContentFilter::identifier).for_each(|id| content.remove_filter_by_id(&id));
        new.iter().for_each(|f| content.add_filter(f));
    }
}

fn apply_zaps(state: &Rc<State>) {
    let Some(content) = state.content.borrow().clone() else { return };
    let zaps: Vec<(String, String)> = scripts::unlocked(state, &state.zaps.borrow()).cloned().collect();
    let json = shield::lists::compile_zaps(zaps.iter().map(|(h, s)| (h.as_str(), s.as_str())));
    let id = json.as_ref().map(|j| format!("tarn-zaps-{}", &shield::groups::hex(shield::lists::fingerprint(&[j]))[..16]));
    if *state.zap_filter.borrow() == id {
        return;
    }
    if let Some(old) = state.zap_filter.replace(id.clone()) {
        content.remove_filter_by_id(&old);
    }
    let (Some(json), Some(id)) = (json, id) else { return };
    let weak = Rc::downgrade(state);
    filters::compile_one(id.clone(), json, move |filter| {
        let Some(state) = weak.upgrade() else { return };
        if *state.zap_filter.borrow() == Some(id)
            && let Some(content) = state.content.borrow().as_ref()
        {
            content.add_filter(&filter);
        }
    });
}

fn load(state: &Rc<State>, current: Option<String>) {
    let Some(web) = state.web() else { return };
    let wanted = state.load.borrow().clone();
    let (target, file) = match wanted {
        Some((url, seq)) if state.loaded_seq.get() != Some(seq) => {
            state.loaded_seq.set(Some(seq));
            *state.link_url.borrow_mut() = state.load_link.get().then(|| url.clone());
            (Some(url), state.load_file.get())
        }
        _ => {
            let opened = current.as_ref().is_some_and(|c| state.opened_files.borrow().contains(c));
            (current, opened)
        }
    };
    let Some(target) = target else { return };
    let Ok(url) = Url::parse(&target) else {
        return state.emit("blocked", json!({ "url": target, "reason": "nav_policy" }));
    };
    if let GateAction::Warn(category) | GateAction::Notice(category) = nav_policy::gate(state.site(&url).blocked, true, true, false) {
        return state.listed(url.as_str(), category);
    }
    state.silenced.set(false);
    *state.shell_url.borrow_mut() = Some(url.to_string());
    state.shell_file.set(file);
    if file && url.scheme() == "file" {
        state.opened_files.borrow_mut().insert(url.to_string());
    }
    web.load_uri(url.as_str());
}

fn navigation_uri(decision: &webkit6::PolicyDecision) -> Option<(webkit6::NavigationAction, String)> {
    let action = decision.downcast_ref::<NavigationPolicyDecision>()?.navigation_action()?;
    let uri = action.request()?.uri()?.to_string();
    Some((action, uri))
}

fn decide(state: &Rc<State>, web: &WebView, decision: &webkit6::PolicyDecision, kind: PolicyDecisionType) -> bool {
    match kind {
        PolicyDecisionType::NavigationAction => decide_navigation(state, web, decision),
        PolicyDecisionType::NewWindowAction => {
            if let Some((action, uri)) = navigation_uri(decision) {
                open_popup(state, &uri, action.is_user_gesture(), is_link(&action), true, is_peek(&action), false);
            }
            decision.ignore();
            true
        }
        PolicyDecisionType::Response => decide_response(state, decision),
        _ => false,
    }
}

fn decide_response(state: &State, decision: &webkit6::PolicyDecision) -> bool {
    let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else { return false };
    let attachment = response
        .response()
        .and_then(|r| r.http_headers())
        .and_then(|h| h.one("Content-Disposition"))
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("attachment"));
    match (attachment || !response.is_mime_type_supported(), response.is_main_frame_main_resource()) {
        (false, _) => ext::hold(state, response),
        (true, true) => {
            decision.download();
            true
        }
        (true, false) => {
            decision.ignore();
            true
        }
    }
}

fn is_link(action: &webkit6::NavigationAction) -> bool {
    action.navigation_type() == NavigationType::LinkClicked
}

fn is_peek(action: &webkit6::NavigationAction) -> bool {
    is_link(action) && action.mouse_button() != 2 && action.modifiers() & SHIFT_MASK != 0 && action.modifiers() & CONTROL_MASK == 0
}

/// WebKit's Open Link in New Window opens a tab, so it says so, and Open Link in Split View
/// follows it, through the same pop-up checks.
fn link_menu(state: &std::rc::Weak<State>, menu: &webkit6::ContextMenu, hit: &webkit6::HitTestResult) {
    use webkit6::{ContextMenuAction, ContextMenuItem};
    let Some(link) = hit.link_uri().filter(|_| hit.context_is_link()).map(|u| u.to_string()) else { return };
    let items = menu.items();
    let Some(at) = items.iter().position(|item| item.stock_action() == ContextMenuAction::OpenLinkInNewWindow) else { return };
    menu.remove(&items[at]);
    menu.insert(&ContextMenuItem::from_stock_action_with_label(ContextMenuAction::OpenLinkInNewWindow, "Open Link in New Tab"), at as i32);
    let split = gio::SimpleAction::new("open-link-in-split-view", None);
    let state = state.clone();
    split.connect_activate(move |_, _| {
        if let Some(s) = state.upgrade() {
            open_popup(&s, &link, true, true, true, false, true);
        }
    });
    menu.insert(&ContextMenuItem::from_gaction(&split, "Open Link in Split View", None), at as i32 + 1);
}

/// `by_page`: a window the page opens, which its site's pop-up rule and the domain sets judge; a
/// link the user opens in a new tab is judged when that tab loads it. `split` puts the tab beside
/// the page in split view.
fn open_popup(state: &Rc<State>, uri: &str, gesture: bool, link: bool, by_page: bool, peek: bool, split: bool) {
    let Ok(url) = Url::parse(uri) else { return };
    let page = state.page.borrow().clone();
    let verdict = policy::popup(&state.levels.borrow(), state.level.get(), &url, page.as_ref());
    match verdict {
        Verdict::Allow { .. } if gesture && by_page && !nav_policy::popup_allowed(opener(state, page.as_ref()), Some(state.site(&url))) => {
            state.emit("blocked", json!({ "url": url.as_str(), "reason": "shield", "popup": true }))
        }
        Verdict::Allow { .. } if gesture && state.admit_popup() => {
            state.emit(if peek { "peek" } else { "popup" }, json!({ "url": url.as_str(), "link": link, "split": split }))
        }
        Verdict::External if gesture && state.admit_prompt() => state.emit("external", json!({ "url": url.as_str(), "origin": state.origin() })),
        Verdict::Blocked(reason) => state.emit("blocked", json!({ "url": url.as_str(), "reason": reason, "popup": true })),
        Verdict::Allow { .. } if !gesture => state.shielded("popup", url.as_str()),
        _ => {}
    }
}

/// The opening page's verdict. WebKitGTK does not say which frame opened a window, so a site's
/// "Allow" counts as "Default": it must not cover the ad frames the site embeds.
fn opener(state: &State, page: Option<&Url>) -> Site {
    let site = page.map_or(Site::new(state.level.get()), |p| state.site(p));
    Site { popups: if site.popups == PopupRule::Allow { PopupRule::Default } else { site.popups }, ..site }
}

fn decide_navigation(state: &Rc<State>, web: &WebView, decision: &webkit6::PolicyDecision) -> bool {
    let Some((action, uri)) = navigation_uri(decision) else {
        decision.ignore();
        return true;
    };
    #[cfg(test)]
    state.events.borrow_mut().push(("decided".to_owned(), json!({ "url": uri })));
    let nav_type = action.navigation_type();
    let gesture = action.is_user_gesture();
    let link = matches!(nav_type, NavigationType::LinkClicked | NavigationType::FormSubmitted);
    let peek = gesture && is_peek(&action);
    if link && gesture && (peek || action.mouse_button() == 2 || action.modifiers() & CONTROL_MASK != 0) {
        open_popup(state, &uri, true, nav_type == NavigationType::LinkClicked, false, peek, false);
        decision.ignore();
        return true;
    }
    let from_shell = state.shell_url.borrow_mut().take_if(|s| *s == uri).is_some();
    let shell_file = from_shell && state.shell_file.take();
    let upgraded = state.upgraded.borrow_mut().take_if(|(s, _, _)| *s == uri);
    let step = matches!(nav_type, NavigationType::BackForward | NavigationType::Reload) && state.user_step.take();
    let file_open = shell_file || (step && state.opened_files.borrow().contains(&uri));
    let Ok(url) = Url::parse(&uri) else {
        decision.ignore();
        return true;
    };
    let page_link = state.link_url.borrow_mut().take_if(|s| *s == uri).is_some();
    let redirect = action.is_redirect();
    let main_frame = from_shell || step || gesture || upgraded.is_some() || (redirect && state.main_pending.get());
    let site = state.site(&url);
    match nav_policy::gate(site.blocked, main_frame, from_shell, false) {
        GateAction::Allow => {}
        GateAction::Warn(category) | GateAction::Notice(category) => {
            state.listed(&uri, category);
            decision.ignore();
            return true;
        }
        GateAction::Cancel => {
            decision.ignore();
            return true;
        }
    }
    let store = engine::nav_policy::StoreNav { redirect, shell_load: from_shell, page_link, clicked: nav_type == NavigationType::LinkClicked && gesture };
    if engine::nav_policy::store_link_asks(store)
        && let Some(id) = engine::tarn_ext::store_install(&url)
    {
        state.emit(engine::tarn_ext::STORE_EVENT, json!({ "id": id }));
        decision.ignore();
        return true;
    }
    let page = state.page.borrow().clone();
    let nav = Navigation {
        url: &url,
        site_filtered: site.filtered,
        filtered: !state.unfiltered.get(),
        from_shell,
        user_link: (link && gesture) || upgraded.as_ref().is_some_and(|(_, link, _)| *link),
        user_step: step || upgraded.as_ref().is_some_and(|(_, _, step)| *step),
        page: page.as_ref(),
        upgraded: upgraded.is_some(),
        likely_subframe: !from_shell && !gesture && nav_type == NavigationType::Other,
        file_open,
    };
    let verdict = policy::navigation(&state.levels.borrow(), state.level.get(), &nav);
    match verdict {
        Verdict::Allow { javascript } => {
            let form = matches!(nav_type, NavigationType::FormSubmitted | NavigationType::FormResubmitted);
            let get = action.request().and_then(|r| r.http_method()).is_none_or(|m| m.eq_ignore_ascii_case("GET"));
            if form && !get {
                *state.posted.borrow_mut() = Some(url.clone());
            }
            if !javascript && let Some(settings) = WebViewExt::settings(web) {
                settings.set_enable_javascript(false);
            }
            if main_frame {
                state.main_pending.set(true);
            }
            return false;
        }
        Verdict::Blocked(reason) => {
            if reason == "local_network" {
                state.shielded("local_network", &uri);
            }
            state.emit("blocked", json!({ "url": uri, "reason": reason }))
        }
        Verdict::Rebuild => state.emit("blocked", json!({ "url": uri, "reason": "rebuild" })),
        Verdict::External => {
            if from_shell || state.admit_prompt() {
                let origin = if from_shell { String::new() } else { state.origin() };
                state.emit("external", json!({ "url": uri, "origin": origin }));
            }
        }
        Verdict::Upgrade(target) => {
            state.shielded("https_upgrade", target.as_str());
            *state.upgraded.borrow_mut() = Some((target.to_string(), nav.user_link, nav.from_shell || nav.user_step));
            if from_shell {
                *state.shell_url.borrow_mut() = Some(target.to_string());
            }
            let web = web.clone();
            glib::idle_add_local_once(move || web.load_uri(target.as_str()));
        }
    }
    decision.ignore();
    true
}

fn prompt(state: &Rc<State>, pending: Pending, data: JsonValue) {
    if state.prompts.borrow().len() >= policy::MAX_PROMPTS {
        return decline(pending);
    }
    let id = state.next_prompt.get() + 1;
    state.next_prompt.set(id);
    let mut data = data;
    data["id"] = json!(id);
    state.prompts.borrow_mut().insert(id, pending);
    state.emit("prompt", data);
}

fn may_prompt(state: &Rc<State>) -> bool {
    if state.silenced.get() {
        return false;
    }
    let admitted = state.admit_prompt();
    if !admitted {
        state.silenced.set(true);
    }
    admitted
}

fn script_dialog(state: &Rc<State>, dialog: &ScriptDialog) -> bool {
    let kind = match dialog.dialog_type() {
        ScriptDialogType::Alert => "alert",
        ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm => "confirm",
        ScriptDialogType::Prompt => "text",
        _ => return false,
    };
    if !may_prompt(state) {
        if kind == "confirm" {
            dialog.confirm_set_confirmed(false);
        }
        return true;
    }
    let message: String = dialog.message().unwrap_or_default().chars().take(4096).collect();
    let default = (kind == "text").then(|| dialog.prompt_get_default_text().unwrap_or_default().to_string());
    let data = json!({ "kind": kind, "origin": state.origin(), "message": message, "default": default });
    prompt(state, Pending::Dialog(dialog.clone()), data);
    true
}

fn permission(state: &Rc<State>, request: &PermissionRequest) -> bool {
    let kind = if let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() {
        if media.is_for_video_device() { "camera" } else { "microphone" }
    } else if request.is::<GeolocationPermissionRequest>() {
        "location"
    } else {
        request.deny();
        return true;
    };
    let page_level = state.page.borrow().as_ref().map(|p| state.levels.borrow().level(p));
    if state.level.get() == ShieldLevel::Lockdown || page_level == Some(ShieldLevel::Lockdown) || !may_prompt(state) {
        request.deny();
        return true;
    }
    let Some(web) = state.web() else {
        request.deny();
        return true;
    };
    let (origin, commit, request) = (state.origin(), state.commits.get(), request.clone());
    let weak = Rc::downgrade(state);
    web.call_async_javascript_function("return window.length;", None, Some(scripts::WORLD), None, None::<&gio::Cancellable>, move |frames| {
        let Some(state) = weak.upgrade().filter(|s| !s.silenced.get() && s.commits.get() == commit) else {
            return request.deny();
        };
        let top = frames.ok().is_some_and(|v| v.is_number() && v.to_int32() == 0);
        let data = json!({ "kind": kind, "origin": origin, "topOrigin": if top { origin.as_str() } else { "" } });
        prompt(&state, Pending::Permission(request), data);
    });
    true
}

fn authenticate(state: &Rc<State>, web: &WebView, request: &AuthenticationRequest) -> bool {
    let http = matches!(
        request.scheme(),
        AuthenticationScheme::HttpBasic | AuthenticationScheme::HttpDigest | AuthenticationScheme::Ntlm | AuthenticationScheme::Negotiate
    );
    let challenge = request.security_origin().map(|o| o.to_str().to_string()).and_then(|o| Url::parse(&o).ok());
    let page = web.uri().and_then(|u| Url::parse(&u).ok());
    let Some(challenge) = challenge.filter(|_| http && !request.is_for_proxy()) else {
        request.cancel();
        return true;
    };
    let origin = policy::origin(&challenge).unwrap_or_default();
    let failures = {
        let mut map = state.auth_failures.borrow_mut();
        let count = map.entry(origin.clone()).or_default();
        if request.is_retry() {
            *count += 1;
        }
        *count
    };
    if !policy::may_ask_credentials(&challenge, page.as_ref(), failures) || !may_prompt(state) {
        request.cancel();
        return true;
    }
    request.set_can_save_credentials(false);
    let data = json!({
        "kind": "credentials",
        "origin": origin,
        "realm": request.realm().unwrap_or_default().to_string(),
        "insecure": challenge.scheme() != "https",
    });
    prompt(state, Pending::Credentials(request.clone()), data);
    true
}

fn answer(state: &Rc<State>, args: &JsonValue) {
    if args["silence"].as_bool() == Some(true) {
        state.silenced.set(true);
        return state.decline_all();
    }
    let Some(pending) = args["id"].as_u64().and_then(|id| state.prompts.borrow_mut().remove(&id)) else { return };
    if !args["accept"].as_bool().unwrap_or(false) {
        return decline(pending);
    }
    match pending {
        Pending::Dialog(dialog) => {
            match dialog.dialog_type() {
                ScriptDialogType::Prompt => dialog.prompt_set_text(args["text"].as_str().unwrap_or_default()),
                ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm => dialog.confirm_set_confirmed(true),
                _ => {}
            }
            dialog.close();
        }
        Pending::Permission(request) => request.allow(),
        Pending::Credentials(request) => {
            let user = args["user"].as_str().unwrap_or_default();
            let password = args["password"].as_str().unwrap_or_default();
            request.authenticate(Some(&Credential::new(user, password, CredentialPersistence::None)));
        }
    }
}

fn save_pdf(state: &Rc<State>, web: &WebView, args: &JsonValue) {
    let Some(path) = args["path"].as_str().map(std::path::PathBuf::from).filter(|p| p.is_absolute()) else { return };
    let settings = gtk::PrintSettings::new();
    settings.set_printer(&glib::dgettext(Some("gtk40"), "Print to File"));
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&webkit6::gio::File::for_path(&path).uri()));
    let operation = webkit6::PrintOperation::builder().web_view(web).print_settings(&settings).build();
    let tab = state.tab.borrow().clone();
    let url = web.uri().map(|u| policy::without_credentials(&u)).unwrap_or_default();
    let held = Rc::new(RefCell::new(Some(operation.clone())));
    let (failed, failed_tab, failed_url, failed_path) = (held.clone(), tab.clone(), url.clone(), path.clone());
    operation.connect_failed(move |_, error| {
        if failed.take().is_some() {
            session::report_saved(&failed_tab, &failed_url, &failed_path, Err(error.message().to_owned()));
        }
    });
    operation.connect_finished(move |_| {
        if held.take().is_some() {
            session::report_saved(&tab, &url, &path, Ok(()));
        }
    });
    operation.print();
}

fn find(state: &Rc<State>, args: &JsonValue) {
    let Some(web) = state.web() else { return };
    let Some(finder) = web.find_controller() else { return };
    let text = args["text"].as_str().unwrap_or_default().to_owned();
    if text.is_empty() {
        clear_find(state);
        return state.emit("found", json!({ "matched": 0 }));
    }
    let backwards = args["backwards"].as_bool().unwrap_or(false);
    if *state.find_text.borrow() == text {
        if backwards {
            finder.search_previous()
        } else {
            finder.search_next()
        }
        return;
    }
    let mut options = FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND;
    if backwards {
        options |= FindOptions::BACKWARDS;
    }
    finder.search_finish();
    *state.find_text.borrow_mut() = text.clone();
    finder.count_matches(&text, options.bits(), FIND_MAX);
    finder.search(&text, options.bits(), FIND_MAX);
}

/// `clear_find`: unmarks the matches a `find` left; the last one stays selected, as in Safari.
fn clear_find(state: &State) {
    if let Some(finder) = state.web().and_then(|web| web.find_controller()) {
        finder.search_finish();
    }
    state.find_text.borrow_mut().clear();
}

fn mute(state: &State, muted: bool) {
    state.muted.set(muted);
    if let Some(web) = state.web() {
        web.set_is_muted(muted);
    }
}

fn snapshot(state: &Rc<State>, web: &WebView) {
    let weak = Rc::downgrade(state);
    web.snapshot(webkit6::SnapshotRegion::Visible, webkit6::SnapshotOptions::NONE, None::<&webkit6::gio::Cancellable>, move |result| {
        let Some(state) = weak.upgrade() else { return };
        let png = result.ok().and_then(|texture| scaled_png(&texture, SNAPSHOT_WIDTH));
        state.emit("snapshot", json!({ "png": png.map(|b| format!("data:image/png;base64,{}", glib::base64_encode(&b))) }));
    });
}

fn scaled_png(texture: &webkit6::gdk::Texture, max_width: i32) -> Option<glib::Bytes> {
    let (width, height) = (texture.width(), texture.height());
    if width <= 0 || height <= 0 {
        return None;
    }
    if width <= max_width {
        return Some(texture.save_to_png_bytes());
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    texture.download(&mut pixels, width as usize * 4);
    let (scaled, w, h) = scale_down(&pixels, width as usize, height as usize, max_width as usize);
    let small =
        webkit6::gdk::MemoryTexture::new(w as i32, h as i32, webkit6::gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(scaled), w * 4);
    Some(small.save_to_png_bytes())
}

fn scale_down(pixels: &[u8], width: usize, height: usize, max_width: usize) -> (Vec<u8>, usize, usize) {
    let w = max_width.clamp(1, width);
    let h = (height * w / width).max(1);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = (y * height / h) * width;
        for x in 0..w {
            let at = (row + x * width / w) * 4;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    (out, w, h)
}

/// `snapshotRect`: a PNG of an area `browser_core::page_ops::snapshot_area` accepts, cut from the
/// visible page, or the whole document when it fits `SNAPSHOT_MAX_SIDE` (WebKitGTK cannot clip a
/// full-document snapshot, so a longer one answers none); under Lockdown a full page is the visible one.
fn snapshot_rect(state: &Rc<State>, web: &WebView, args: &JsonValue) {
    use engine::view_policy::{SNAPSHOT_MAX_SIDE, SnapshotArea};
    let seq = args["seq"].as_u64().unwrap_or_default();
    let weak = Rc::downgrade(state);
    let answer = move |png: Option<glib::Bytes>| {
        if let Some(state) = weak.upgrade() {
            state.emit("snapshotRect", json!({ "seq": seq, "png": png.map(|b| format!("data:image/png;base64,{}", glib::base64_encode(&b))) }));
        }
    };
    let css_width = f64::from(web.width()) / web.zoom_level();
    let whole = |texture: webkit6::gdk::Texture| texture.save_to_png_bytes();
    match browser_core::page_ops::snapshot_area(args) {
        None => answer(None),
        Some(SnapshotArea::Rect { x, y, width, height }) => {
            web.snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |result| {
                answer(result.ok().and_then(|texture| {
                    let bounds = pixel_rect([x, y, width, height], f64::from(texture.width()) / css_width, texture.width(), texture.height())?;
                    Some(cropped_png(&texture, bounds))
                }))
            })
        }
        Some(SnapshotArea::FullPage) if scripts::locked(state) => {
            web.snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |result| answer(result.ok().map(whole)))
        }
        Some(SnapshotArea::FullPage) => {
            let view = web.clone();
            web.call_async_javascript_function(
                engine::page_scripts::DOCUMENT_SIZE,
                None,
                Some(scripts::WORLD),
                None,
                None::<&gio::Cancellable>,
                move |result| {
                    let size = result.ok().filter(|v| v.is_string()).and_then(|v| serde_json::from_str::<[f64; 4]>(&v.to_str()).ok());
                    if !size.is_some_and(|[_, _, w, h]| w <= SNAPSHOT_MAX_SIDE && h <= SNAPSHOT_MAX_SIDE) {
                        return answer(None);
                    }
                    view.snapshot(SnapshotRegion::FullDocument, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |result| answer(result.ok().map(whole)));
                },
            );
        }
    }
}

/// The texture pixels `[x0, y0, x1, y1]` a CSS-pixel `[x, y, width, height]` covers at `per_css`
/// texture pixels each, inside the texture; `None` when nothing of it is.
fn pixel_rect([x, y, width, height]: [f64; 4], per_css: f64, texture_width: i32, texture_height: i32) -> Option<[usize; 4]> {
    if !per_css.is_finite() || per_css <= 0. {
        return None;
    }
    let edge = |v: f64, max: i32| (v * per_css).clamp(0., f64::from(max.max(0))) as usize;
    let (x0, y0) = (edge(x, texture_width), edge(y, texture_height));
    let (x1, y1) = (edge((x + width).ceil(), texture_width), edge((y + height).ceil(), texture_height));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

fn cropped_png(texture: &webkit6::gdk::Texture, [x0, y0, x1, y1]: [usize; 4]) -> glib::Bytes {
    let width = texture.width() as usize;
    let mut pixels = vec![0u8; width * texture.height() as usize * 4];
    texture.download(&mut pixels, width * 4);
    let (w, h) = (x1 - x0, y1 - y0);
    let rows = crop(&pixels, width, [x0, y0, x1, y1]);
    webkit6::gdk::MemoryTexture::new(w as i32, h as i32, webkit6::gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(rows), w * 4)
        .save_to_png_bytes()
}

fn crop(pixels: &[u8], width: usize, [x0, y0, x1, y1]: [usize; 4]) -> Vec<u8> {
    (y0..y1).flat_map(|y| &pixels[(y * width + x0) * 4..(y * width + x1) * 4]).copied().collect()
}

fn screenshot(state: &Rc<State>, web: &WebView, args: &JsonValue) {
    let seq = args["seq"].as_u64().unwrap_or_default();
    let name = engine::downloads::sanitize_filename(args["name"].as_str().unwrap_or_default());
    let weak = Rc::downgrade(state);
    let clipboard = web.clipboard();
    web.snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |result| {
        let saved = result.map_err(|_| "WebKit could not capture the page".to_owned()).and_then(|texture| {
            clipboard.set_texture(&texture);
            let dir = session::downloads_dir();
            std::fs::create_dir_all(&dir).and_then(|()| browser_core::capture::save_new(&dir, &name, &texture.save_to_png_bytes())).map_err(|e| e.to_string())
        });
        let Some(state) = weak.upgrade() else { return };
        state.emit(
            "screenshot",
            match saved {
                Ok(path) => json!({ "seq": seq, "path": path.to_string_lossy() }),
                Err(message) => json!({ "seq": seq, "message": message }),
            },
        );
    });
}

fn media_changed(state: &Weak<State>, web: &WebView) {
    let capturing = |state| state != MediaCaptureState::None;
    let playing = web.is_playing_audio() || capturing(web.camera_capture_state()) || capturing(web.microphone_capture_state());
    if let Some(s) = state.upgrade() {
        s.emit("playing", json!({ "playing": playing }));
    }
}

fn connect(state: &Rc<State>, web: &WebView) {
    let weak = Rc::downgrade(state);
    let w = weak.clone();
    web.connect_mouse_target_changed(move |_, hit, _| {
        if let Some(s) = w.upgrade() {
            let link = hit.link_uri().map(|u| u.chars().take(engine::view_policy::MAX_HOVERED_LINK).collect::<String>());
            s.emit("hover", json!({ "url": link }));
        }
    });
    let w = weak.clone();
    web.connect_decide_policy(move |web, decision, kind| w.upgrade().is_some_and(|s| decide(&s, web, decision, kind)));
    let w = weak.clone();
    web.connect_load_changed(move |web, event| {
        let Some(s) = w.upgrade() else { return };
        match event {
            LoadEvent::Started | LoadEvent::Redirected => {
                s.failed.set(false);
                s.note_processes();
                ext::started(&s);
                s.emit("started", json!({}));
            }
            LoadEvent::Committed => {
                s.main_pending.set(false);
                ext::committed(&s, web);
                let page = web.uri().and_then(|u| Url::parse(&u).ok());
                s.user_step.set(false);
                s.decline_all();
                s.commits.set(s.commits.get() + 1);
                *s.page.borrow_mut() = page.clone();
                if let Some(url) = page.as_ref().filter(|u| policy::needs_stricter_view(&s.levels.borrow(), s.level.get(), u)) {
                    web.stop_loading();
                    return s.emit("blocked", json!({ "url": url.as_str(), "reason": "rebuild" }));
                }
                let settings = WebViewExt::settings(web);
                let enabled = settings.as_ref().is_some_and(|settings| settings.enables_javascript());
                if let (Some(url), Some(settings)) = (page.as_ref(), settings)
                    && policy::javascript_restored(&s.levels.borrow(), s.level.get(), url, enabled)
                {
                    settings.set_enable_javascript(true);
                    if s.posted.take().as_ref() != Some(url) {
                        web.stop_loading();
                        return web.reload();
                    }
                }
                s.posted.take();
                s.silenced.set(false);
                s.auth_failures.borrow_mut().clear();
                s.find_text.borrow_mut().clear();
                s.claim_process();
                s.emit("committed", s.page_state(web));
            }
            LoadEvent::Finished => {
                s.user_step.set(false);
                if !s.failed.get() {
                    s.emit("finished", s.page_state(web));
                }
            }
            _ => {}
        }
    });
    let w = weak.clone();
    web.connect_title_notify(move |web| {
        if let Some(s) = w.upgrade() {
            s.emit("title", json!({ "title": policy::title(&web.title().unwrap_or_default()) }));
        }
    });
    let w = weak.clone();
    web.connect_uri_notify(move |web| {
        let Some(s) = w.upgrade().filter(|_| !web.is_loading() && web.uri().is_some_and(|u| !u.is_empty())) else { return };
        ext::moved(&s, web);
        s.user_step.set(false);
        *s.page.borrow_mut() = web.uri().and_then(|u| Url::parse(&u).ok());
        s.emit("finished", s.page_state(web));
    });
    let w = weak.clone();
    web.connect_load_failed(move |_, _, uri, err| {
        if let Some(s) = w.upgrade() {
            s.failed.set(true);
            s.main_pending.set(false);
            if !err.matches(NetworkError::Cancelled) && !err.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
                s.emit("failed", json!({ "url": uri, "message": err.message() }));
            } else {
                s.emit("stopped", json!({}));
            }
        }
        true
    });
    let w = weak.clone();
    web.connect_load_failed_with_tls_errors(move |_, uri, _, flags| {
        if let Some(s) = w.upgrade() {
            s.failed.set(true);
            let message = format!("The site's certificate is not trusted ({flags:?}); the page was not loaded.");
            s.emit("failed", json!({ "url": uri, "message": message }));
        }
        true
    });
    let w = weak.clone();
    web.connect_web_process_terminated(move |_, reason| {
        if let Some(s) = w.upgrade().filter(|_| reason != WebProcessTerminationReason::TerminatedByApi) {
            s.emit("crashed", JsonValue::Null);
        }
    });
    let w = weak.clone();
    web.connect_script_dialog(move |_, dialog| w.upgrade().is_some_and(|s| script_dialog(&s, dialog)));
    let w = weak.clone();
    web.connect_permission_request(move |_, request| {
        if !w.upgrade().is_some_and(|s| permission(&s, request)) {
            request.deny();
        }
        true
    });
    let w = weak.clone();
    web.connect_authenticate(move |web, request| {
        if !w.upgrade().is_some_and(|s| authenticate(&s, web, request)) {
            request.cancel();
        }
        true
    });
    let w = weak.clone();
    web.connect_enter_fullscreen(move |web| {
        if let Some(s) = w.upgrade() {
            let window_full_screen = web.root().and_downcast::<gtk::Window>().is_some_and(|window| window.is_fullscreen());
            s.emit("fullscreen", json!({ "on": true, "windowFullScreen": window_full_screen }));
        }
        false
    });
    let w = weak.clone();
    web.connect_leave_fullscreen(move |_| {
        if let Some(s) = w.upgrade() {
            s.emit("fullscreen", json!({ "on": false }));
        }
        false
    });
    let w = weak.clone();
    web.connect_is_playing_audio_notify(move |web| media_changed(&w, web));
    let w = weak.clone();
    web.connect_camera_capture_state_notify(move |web| media_changed(&w, web));
    let w = weak.clone();
    web.connect_microphone_capture_state_notify(move |web| media_changed(&w, web));
    let w = weak.clone();
    web.connect_create(move |_, action| {
        let action = action.clone();
        let uri = action.request().and_then(|r| r.uri()).map(|u| u.to_string());
        if let (Some(s), Some(uri)) = (w.upgrade(), uri) {
            open_popup(&s, &uri, action.is_user_gesture(), is_link(&action), true, is_peek(&action), false);
        }
        None
    });
    let w = weak.clone();
    web.connect_context_menu(move |_, menu, hit| {
        link_menu(&w, menu, hit);
        false
    });
    if let Some(finder) = web.find_controller() {
        let w = weak.clone();
        finder.connect_counted_matches(move |_, count| {
            if let Some(s) = w.upgrade() {
                s.emit("found", json!({ "matched": count }));
            }
        });
        finder.connect_failed_to_find_text(move |_| {
            if let Some(s) = weak.upgrade() {
                s.emit("found", json!({ "matched": 0 }));
            }
        });
    }
}

fn hosts(value: &JsonValue) -> HashSet<String> {
    value.as_array().into_iter().flatten().filter_map(|h| h.as_str()).map(|h| h.trim_end_matches('.').to_ascii_lowercase()).collect()
}

fn set_blocking(state: &Rc<State>, value: &JsonValue) {
    *state.rules.paused.borrow_mut() = hosts(&value["paused"]);
    *state.rules.exceptions.borrow_mut() = hosts(&value["exceptions"]);
    *state.rules.popups.borrow_mut() = value["popups"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(origin, rule)| match rule.as_str() {
            Some("allow") => Some((origin.clone(), PopupRule::Allow)),
            Some("block") => Some((origin.clone(), PopupRule::Block)),
            _ => None,
        })
        .collect();
    let mut categories: Vec<Category> = value["sets"].as_array().into_iter().flatten().filter_map(|c| c.as_str().and_then(Category::parse)).collect();
    categories.retain(|c| browser_core::site::SET_SWITCHES.iter().any(|(s, ..)| s == c));
    categories.sort();
    categories.dedup();
    filters::want_sets(categories);
}

#[cfg(test)]
mod display_tests;
#[cfg(test)]
mod ext_display_tests;

pub struct TarnWebView(Rc<State>);

impl TarnWebView {
    pub fn new(context: ComponentContext) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        let state = Rc::new(State { context: Some(context), container: Some(container), ..State::default() });
        // A split's pane takes the focus when its page is clicked; the page gets the click too.
        let pressed = gtk::GestureClick::new();
        pressed.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&state);
        pressed.connect_pressed(move |_, _, _, _| {
            if let Some(state) = weak.upgrade() {
                state.emit("pressed", JsonValue::Null);
            }
        });
        state.container().add_controller(pressed);
        VIEWS.with_borrow_mut(|views| {
            views.retain(|v| v.strong_count() > 0);
            views.push(Rc::downgrade(&state));
        });
        TarnWebView(state)
    }
}

impl Drop for TarnWebView {
    fn drop(&mut self) {
        self.0.drop_view();
    }
}

impl NativeComponent for TarnWebView {
    fn widget(&self) -> gtk::Widget {
        self.0.container().clone().upcast()
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        let state = &self.0;
        let null = JsonValue::Null;
        let value = value.unwrap_or(&null);
        match name {
            "tab" => return *state.tab.borrow_mut() = value.as_str().unwrap_or_default().to_owned(),
            "burstsOf" => {
                let opener = value.as_str().filter(|tab| !tab.is_empty() && *tab != *state.tab.borrow());
                let shared = opener.and_then(|tab| {
                    VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).find(|s| *s.tab.borrow() == tab).map(|s| s.bursts.borrow().clone()))
                });
                if let Some(bursts) = shared {
                    *state.bursts.borrow_mut() = bursts;
                }
                return;
            }
            "profile" => *state.profile.borrow_mut() = value.as_str().unwrap_or_default().to_owned(),
            "level" => state.level.set(value.as_str().and_then(ShieldLevel::parse).unwrap_or(ShieldLevel::Lockdown)),
            "inspectable" => state.inspectable.set(value.as_bool() == Some(true)),
            "levels" => {
                *state.levels.borrow_mut() = Levels::parse(value, state.level.get());
                if let Some(content) = state.content.borrow().as_ref() {
                    scripts::apply_styles(state, content);
                    scripts::apply_page_scripts(state, content);
                }
                apply_zaps(state);
            }
            "lists" => {
                let mut lists: Vec<String> = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                    .filter(|v| shield::groups::GROUPS.contains(v))
                    .map(str::to_owned)
                    .collect();
                lists.sort();
                lists.dedup();
                *state.lists.borrow_mut() = lists;
            }
            "styles" | "themeStyles" => {
                if name == "styles" {
                    *state.styles.borrow_mut() = scripts::styles(value);
                } else {
                    *state.theme_styles.borrow_mut() = scripts::theme_styles(value);
                }
                if let Some(content) = state.content.borrow().as_ref() {
                    scripts::apply_styles(state, content);
                }
                return;
            }
            "zaps" => {
                *state.zaps.borrow_mut() = scripts::zaps(value);
                return apply_zaps(state);
            }
            "youtube" => state.youtube.set(value.as_bool().unwrap_or(false)),
            "filtered" => state.unfiltered.set(value.as_bool() == Some(false)),
            "blocking" => return set_blocking(state, value),
            "sortDownloads" => return state.sort_downloads.set(value.as_bool().unwrap_or(false)),
            "muted" => return mute(state, value.as_bool().unwrap_or(false)),
            "load" => {
                *state.load.borrow_mut() = value["url"].as_str().zip(value["seq"].as_u64()).map(|(u, s)| (u.to_owned(), s));
                state.load_link.set(value["link"].as_bool() == Some(true));
                state.load_file.set(value["file"].as_bool() == Some(true));
            }
            "active" => {
                let active = value.as_bool().unwrap_or(false);
                state.container().set_visible(active);
                return;
            }
            _ => return,
        }
        schedule(state);
    }

    fn command(&mut self, name: &str, args: &JsonValue) {
        let state = &self.0;
        match name {
            "answer" => return answer(state, args),
            "find" => return find(state, args),
            "clearFind" => return clear_find(state),
            "downloadCancel" => return session::cancel(args["id"].as_str().unwrap_or_default()),
            "downloadKeep" => return session::keep(args["id"].as_str().unwrap_or_default(), args["keep"].as_bool() == Some(true)),
            "downloadPause" | "downloadResume" => return,
            _ if scripts::command(state, name, args) => return,
            _ => {}
        }
        let Some(web) = state.web() else { return };
        match name {
            "back" if web.can_go_back() => {
                state.user_step.set(true);
                web.go_back();
            }
            "forward" if web.can_go_forward() => {
                state.user_step.set(true);
                web.go_forward();
            }
            "reload" if web.uri().is_some_and(|u| !u.is_empty()) => {
                state.user_step.set(true);
                web.reload();
            }
            "reloadFromOrigin" if web.uri().is_some_and(|u| !u.is_empty()) => {
                state.user_step.set(true);
                web.reload_bypass_cache();
            }
            "back" | "forward" | "reload" | "reloadFromOrigin" => {}
            "clearCache" => session::clear_cache(&web),
            "stop" => web.stop_loading(),
            "focus" => {
                web.grab_focus();
            }
            "print" => {
                glib::idle_add_local_once(move || {
                    webkit6::PrintOperation::new(&web).run_dialog(web.root().and_downcast::<gtk::Window>().as_ref());
                });
            }
            "pdf" => save_pdf(state, &web, args),
            "snapshot" => snapshot(state, &web),
            "screenshot" => screenshot(state, &web, args),
            "snapshotRect" => snapshot_rect(state, &web, args),
            "zoom" => {
                let factor = policy::zoom_factor(args["factor"].as_f64().unwrap_or(1.0));
                state.zoom.set(factor);
                web.set_zoom_level(factor);
            }
            _ => eprintln!("tarn: unknown web view command {name}"),
        }
    }
}
