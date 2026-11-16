pub mod filters;
pub use engine::view_policy as policy;
mod scripts;
pub mod session;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use nsh_core::JsonValue;
use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::{ComponentContext, NativeComponent};
use serde_json::json;
use shield::ShieldLevel;
use url::Url;
use webkit6::prelude::*;
use webkit6::{
    AuthenticationRequest, AuthenticationScheme, Credential, CredentialPersistence, Download, FindOptions, GeolocationPermissionRequest, LoadEvent,
    MediaCaptureState, NavigationPolicyDecision, NavigationType, NetworkError, PermissionRequest, PolicyDecisionType, PolicyError, ResponsePolicyDecision,
    ScriptDialog, ScriptDialogType, UserContentFilter, UserContentManager, UserMediaPermissionRequest, WebProcessTerminationReason, WebView, glib,
};

use self::policy::{Burst, Levels, Navigation, Verdict};
use crate::data::StoreKey;

pub const NAME: &str = "tarn.webview";
const FIND_MAX: u32 = 1000;
const CONTROL_MASK: u32 = 1 << 2;

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
    levels: RefCell<Levels>,
    lists: RefCell<Vec<String>>,
    load: RefCell<Option<(String, u64)>>,
    loaded_seq: Cell<Option<u64>>,
    shell_url: RefCell<Option<String>>,
    upgraded: RefCell<Option<(String, bool, bool)>>,
    user_step: Cell<bool>,
    posted: RefCell<Option<Url>>,
    page: RefCell<Option<Url>>,
    failed: Cell<bool>,
    scheduled: Cell<bool>,
    silenced: Cell<bool>,
    prompts: RefCell<HashMap<u64, Pending>>,
    next_prompt: Cell<u64>,
    prompt_burst: RefCell<Burst>,
    popup_burst: RefCell<Burst>,
    download_burst: RefCell<Burst>,
    auth_failures: RefCell<HashMap<String, u32>>,
    find_text: RefCell<String>,
    zoom: Cell<f64>,
    content: RefCell<Option<UserContentManager>>,
    styles: RefCell<Vec<(String, String)>>,
    theme_styles: RefCell<Vec<(String, String)>>,
    zaps: RefCell<Vec<(String, String)>>,
    zap_filter: RefCell<Option<String>>,
    sort_downloads: Cell<bool>,
    processes: RefCell<std::collections::HashSet<i32>>,
    process: Cell<Option<i32>>,
    #[cfg(test)]
    events: RefCell<Vec<(String, JsonValue)>>,
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

    fn admit(&self, burst: &RefCell<Burst>, max: usize) -> bool {
        burst.borrow_mut().admit(max, Instant::now())
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
        self.content.replace(None);
        if let Some(old) = self.view.replace(None) {
            old.stop_loading();
            self.container().remove(&old);
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

pub fn adopt_download(download: &Download) {
    let owner = download.web_view();
    let state = VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).find(|s| owner.is_some() && s.web() == owner));
    match state {
        Some(state) => session::track(download, &*state),
        None => download.cancel(),
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
    let wanted = Build { key, level: state.level.get(), lists: state.lists.borrow().clone() };
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
    *state.content.borrow_mut() = Some(content.clone());
    *state.zap_filter.borrow_mut() = None;
    apply_zaps(state);
    let web = WebView::builder().network_session(&session).user_content_manager(&content).build();
    if let Some(settings) = WebViewExt::settings(&web) {
        settings.set_enable_javascript(wanted.level.policy().javascript);
        settings.set_javascript_can_open_windows_automatically(false);
        settings.set_enable_developer_extras(false);
    }
    web.set_hexpand(true);
    web.set_vexpand(true);
    if state.zoom.get() > 0.0 {
        web.set_zoom_level(state.zoom.get());
    }
    connect(state, &web);
    state.note_processes();
    state.container().append(&web);
    *state.view.borrow_mut() = Some(web);
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
    let target = match wanted {
        Some((url, seq)) if state.loaded_seq.get() != Some(seq) => {
            state.loaded_seq.set(Some(seq));
            Some(url)
        }
        _ => current,
    };
    let Some(target) = target else { return };
    let Ok(url) = Url::parse(&target) else {
        return state.emit("blocked", json!({ "url": target, "reason": "nav_policy" }));
    };
    state.silenced.set(false);
    *state.shell_url.borrow_mut() = Some(url.to_string());
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
                open_popup(state, &uri, action.is_user_gesture(), is_link(&action));
            }
            decision.ignore();
            true
        }
        PolicyDecisionType::Response => decide_response(decision),
        _ => false,
    }
}

fn decide_response(decision: &webkit6::PolicyDecision) -> bool {
    let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else { return false };
    let attachment = response
        .response()
        .and_then(|r| r.http_headers())
        .and_then(|h| h.one("Content-Disposition"))
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("attachment"));
    match (attachment || !response.is_mime_type_supported(), response.is_main_frame_main_resource()) {
        (false, _) => false,
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

fn open_popup(state: &Rc<State>, uri: &str, gesture: bool, link: bool) {
    let Ok(url) = Url::parse(uri) else { return };
    let page = state.page.borrow().clone();
    let verdict = policy::popup(&state.levels.borrow(), state.level.get(), &url, page.as_ref());
    match verdict {
        Verdict::Allow { .. } if gesture && state.admit(&state.popup_burst, policy::POPUP_BURST) => {
            state.emit("popup", json!({ "url": url.as_str(), "link": link }))
        }
        Verdict::External if gesture && state.admit(&state.prompt_burst, policy::PROMPT_BURST) => {
            state.emit("external", json!({ "url": url.as_str(), "origin": state.origin() }))
        }
        Verdict::Blocked(reason) => state.emit("blocked", json!({ "url": url.as_str(), "reason": reason, "popup": true })),
        Verdict::Allow { .. } if !gesture => state.shielded("popup", url.as_str()),
        _ => {}
    }
}

fn decide_navigation(state: &Rc<State>, web: &WebView, decision: &webkit6::PolicyDecision) -> bool {
    let Some((action, uri)) = navigation_uri(decision) else {
        decision.ignore();
        return true;
    };
    let nav_type = action.navigation_type();
    let gesture = action.is_user_gesture();
    let link = matches!(nav_type, NavigationType::LinkClicked | NavigationType::FormSubmitted);
    if link && gesture && (action.mouse_button() == 2 || action.modifiers() & CONTROL_MASK != 0) {
        open_popup(state, &uri, true, nav_type == NavigationType::LinkClicked);
        decision.ignore();
        return true;
    }
    let from_shell = state.shell_url.borrow_mut().take_if(|s| *s == uri).is_some();
    let upgraded = state.upgraded.borrow_mut().take_if(|(s, _, _)| *s == uri);
    let step = matches!(nav_type, NavigationType::BackForward | NavigationType::Reload) && state.user_step.take();
    let Ok(url) = Url::parse(&uri) else {
        decision.ignore();
        return true;
    };
    let page = state.page.borrow().clone();
    let nav = Navigation {
        url: &url,
        from_shell,
        user_link: (link && gesture) || upgraded.as_ref().is_some_and(|(_, link, _)| *link),
        user_step: step || upgraded.as_ref().is_some_and(|(_, _, step)| *step),
        page: page.as_ref(),
        upgraded: upgraded.is_some(),
        likely_subframe: !from_shell && !gesture && nav_type == NavigationType::Other,
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
            if from_shell || state.admit(&state.prompt_burst, policy::PROMPT_BURST) {
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
    let admitted = state.admit(&state.prompt_burst, policy::PROMPT_BURST);
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
    let origin = state.origin();
    let data = json!({ "kind": kind, "origin": origin, "topOrigin": "" });
    prompt(state, Pending::Permission(request.clone()), data);
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

fn find(state: &Rc<State>, args: &JsonValue) {
    let Some(web) = state.web() else { return };
    let Some(finder) = web.find_controller() else { return };
    let text = args["text"].as_str().unwrap_or_default().to_owned();
    if text.is_empty() {
        finder.search_finish();
        state.find_text.borrow_mut().clear();
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
    web.connect_decide_policy(move |web, decision, kind| w.upgrade().is_some_and(|s| decide(&s, web, decision, kind)));
    let w = weak.clone();
    web.connect_load_changed(move |web, event| {
        let Some(s) = w.upgrade() else { return };
        match event {
            LoadEvent::Started => {
                s.failed.set(false);
                s.note_processes();
            }
            LoadEvent::Committed => {
                let page = web.uri().and_then(|u| Url::parse(&u).ok());
                s.user_step.set(false);
                s.decline_all();
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
        s.user_step.set(false);
        *s.page.borrow_mut() = web.uri().and_then(|u| Url::parse(&u).ok());
        s.emit("finished", s.page_state(web));
    });
    let w = weak.clone();
    web.connect_load_failed(move |_, _, uri, err| {
        if let Some(s) = w.upgrade() {
            s.failed.set(true);
            if !err.matches(NetworkError::Cancelled) && !err.matches(PolicyError::FrameLoadInterruptedByPolicyChange) {
                s.emit("failed", json!({ "url": uri, "message": err.message() }));
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
            open_popup(&s, &uri, action.is_user_gesture(), is_link(&action));
        }
        None
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

#[cfg(test)]
mod display_tests;

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
            "profile" => *state.profile.borrow_mut() = value.as_str().unwrap_or_default().to_owned(),
            "level" => state.level.set(value.as_str().and_then(ShieldLevel::parse).unwrap_or(ShieldLevel::Lockdown)),
            "levels" => {
                *state.levels.borrow_mut() = Levels::parse(value, state.level.get());
                if let Some(content) = state.content.borrow().as_ref() {
                    scripts::apply_styles(state, content);
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
            "sortDownloads" => return state.sort_downloads.set(value.as_bool().unwrap_or(false)),
            "load" => {
                *state.load.borrow_mut() = value["url"].as_str().zip(value["seq"].as_u64()).map(|(u, s)| (u.to_owned(), s));
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
            "downloadCancel" => return session::cancel(args["id"].as_str().unwrap_or_default()),
            "downloadKeep" | "downloadPause" | "downloadResume" => return,
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
            "back" | "forward" | "reload" => {}
            "stop" => web.stop_loading(),
            "zoom" => {
                let factor = policy::zoom_factor(args["factor"].as_f64().unwrap_or(1.0));
                state.zoom.set(factor);
                web.set_zoom_level(factor);
            }
            _ => eprintln!("tarn: unknown web view command {name}"),
        }
    }
}
