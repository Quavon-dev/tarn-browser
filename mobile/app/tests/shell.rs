use serde_json::{Value, json};
use shield::ShieldLevel;
use tarn_mobile::data::{Memory, Platform};
use tarn_mobile::events::Answer;
use tarn_mobile::shell::{Effect, Shell};

type TestShell = Shell<Memory>;

fn add_space<D: Platform>(shell: &mut Shell<D>, name: &str) -> bool {
    let mut draft = shell.space_draft(None);
    draft.name = name.into();
    shell.save_space(&draft)
}

fn add_profile<D: Platform>(shell: &mut Shell<D>, name: &str) -> bool {
    let mut draft = shell.space_draft(None);
    draft.profile = None;
    draft.profile_name = name.into();
    draft.name = "Home".into();
    shell.save_space(&draft)
}

fn open() -> (tempfile::TempDir, TestShell) {
    let dir = tempfile::tempdir().unwrap();
    let shell = TestShell::open(dir.path());
    (dir, shell)
}

fn active(shell: &TestShell) -> browser_core::tabs::TabKey {
    shell.model.active.expect("an active tab")
}

fn finish(shell: &mut TestShell, url: &str, title: &str) {
    let key = active(shell);
    let page = json!({ "url": url, "title": title, "canGoBack": true, "canGoForward": false });
    shell.event(key, "committed", &page, 1);
    shell.event(key, "finished", &page, 2);
}

#[test]
fn a_fresh_app_shows_one_empty_tab_without_a_load() {
    let (_dir, shell) = open();
    assert_eq!(shell.tab_count(), 1);
    let views = shell.views();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].load, None);
    assert_eq!(views[0].level, ShieldLevel::Standard);
    assert!(!views[0].profile.starts_with("private:"));
}

#[test]
fn the_address_bar_loads_urls_and_searches_with_a_new_seq_each_time() {
    let (_dir, mut shell) = open();
    shell.navigate("example.com");
    let first = shell.views()[0].load.clone().unwrap();
    assert_eq!(first.url, "https://example.com/");
    shell.navigate("example.com");
    let again = shell.views()[0].load.clone().unwrap();
    assert_eq!(again.url, first.url);
    assert!(again.seq > first.seq, "the same URL loads again");
    shell.set_search_engine(1);
    shell.navigate("rust lifetimes");
    assert_eq!(shell.views()[0].load.clone().unwrap().url, "https://www.startpage.com/sp/search?query=rust+lifetimes");
}

#[test]
fn rejected_and_external_input_never_loads() {
    let (_dir, mut shell) = open();
    shell.navigate("javascript:alert(1)");
    assert!(shell.notice.is_some());
    assert_eq!(shell.views()[0].load, None);
    shell.navigate("mailto:a@example.com");
    assert_eq!(shell.external.as_ref().map(|(_, u)| u.as_str()), Some("mailto:a@example.com"));
    assert_eq!(shell.confirm_external(false), vec![]);
    assert!(shell.external.is_none());
    shell.navigate("mailto:a@example.com");
    assert_eq!(shell.confirm_external(true), vec![Effect::Open("mailto:a@example.com".into())]);
}

#[test]
fn a_hand_off_names_the_page_that_asked_and_none_for_a_typed_one() {
    let (_dir, mut shell) = open();
    shell.navigate("https://news.test/");
    finish(&mut shell, "https://news.test/", "News");
    let key = active(&shell);
    shell.event(key, "external", &json!({ "url": "mailto:a@example.com", "origin": "https://news.test" }), 1);
    assert_eq!(shell.external_origin, "https://news.test");
    shell.confirm_external(false);
    shell.navigate("mailto:b@example.com");
    assert_eq!(shell.external_origin, "", "the user typed it");
}

#[test]
fn history_feeds_suggestions_and_titles_follow() {
    let (_dir, mut shell) = open();
    shell.navigate("https://docs.rs/");
    finish(&mut shell, "https://docs.rs/", "Docs.rs");
    shell.event(active(&shell), "title", &json!({ "title": "Docs.rs — crates" }), 3);
    shell.navigate("https://example.com/");
    finish(&mut shell, "https://example.com/", "Example Domain");
    let found = shell.suggestions("doc", "docs").0;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Docs.rs — crates");
    assert!(shell.suggestions("", " ").0.is_empty());
    assert_eq!(shell.history().len(), 2);
    assert_eq!(shell.model.active_tab().unwrap().title, "Example Domain");
}

#[test]
fn only_addresses_the_user_opened_complete() {
    let (_dir, mut shell) = open();
    shell.navigate("https://docs.rs/");
    finish(&mut shell, "https://docs.rs/", "Docs.rs");
    finish(&mut shell, "https://evil.test/", "Planted");
    finish(&mut shell, "https://evil.test/", "Planted");
    shell.navigate("https://example.com/");
    finish(&mut shell, "https://example.com/", "Example");
    let suffix = |shell: &TestShell, typed: &str, value: &str| shell.suggestions(typed, value).1.map(|c| c.suffix);
    assert_eq!(suffix(&shell, "d", "do").as_deref(), Some("cs.rs"));
    assert_eq!(suffix(&shell, "e", "ev"), None, "a page led there, however often");
    assert_eq!(suffix(&shell, "e", "ex").as_deref(), Some("ample.com"));
    shell.navigate("https://paypal.com/");
    finish(&mut shell, "https://paypal.com.evil.test/", "Redirected");
    assert_eq!(suffix(&shell, "p", "pa"), None, "a redirect to another host, though its tab is open");
}

#[test]
fn tabs_open_close_and_reopen() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.test/");
    let first = active(&shell);
    let second = shell.new_tab(Some("https://b.test/")).unwrap();
    assert_eq!(shell.tab_count(), 2);
    assert_eq!(active(&shell), second);
    finish(&mut shell, "https://b.test/page", "B");
    shell.close_tab(second);
    assert_eq!(shell.tab_count(), 1);
    assert_eq!(active(&shell), first);
    assert!(shell.reopen_closed());
    assert_eq!(shell.tab_count(), 2);
    let reopened = active(&shell);
    assert_ne!(reopened, first);
    assert_eq!(shell.views().iter().find(|v| v.key == reopened).unwrap().load.as_ref().unwrap().url, "https://b.test/page");
    shell.close_tab(first);
    shell.close_tab(reopened);
    assert_eq!(shell.tab_count(), 1, "closing the last tab leaves an empty one");
}

#[test]
fn an_untouched_new_tab_is_discarded_and_the_last_tab_returns() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.test/");
    let first = active(&shell);
    let fresh = shell.new_tab(None).unwrap();
    shell.discard_new_tab(fresh, first);
    assert_eq!(shell.tab_count(), 1);
    assert_eq!(active(&shell), first);
    assert!(!shell.reopen_closed(), "a discarded new tab is not a closed tab");

    let typed = shell.new_tab(None).unwrap();
    shell.navigate("https://b.test/");
    shell.discard_new_tab(typed, first);
    assert_eq!(shell.tab_count(), 2, "a new tab that loads something stays");
    assert_eq!(active(&shell), typed);
}

#[test]
fn only_recent_tabs_keep_a_view() {
    let (_dir, mut shell) = open();
    for n in 0..10 {
        shell.new_tab(Some(&format!("https://t{n}.test/")));
    }
    assert_eq!(shell.views().len(), tarn_mobile::shell::LIVE_TABS);
}

#[test]
fn the_private_space_uses_its_own_ephemeral_store_and_forgets_everything() {
    let (_dir, mut shell) = open();
    let normal = shell.views()[0].profile.clone();
    shell.enter_private();
    assert!(shell.model.private());
    let first_session = shell.views()[0].profile.clone();
    assert!(first_session.starts_with("private:"));
    shell.navigate("https://secret.test/");
    finish(&mut shell, "https://secret.test/", "Secret");
    assert_eq!(shell.history().len(), 1);
    shell.close_private();
    assert!(!shell.model.private());
    assert_eq!(shell.views()[0].profile, normal);
    assert!(shell.core.disk.history(1, 10).unwrap().iter().all(|h| !h.url.contains("secret")), "nothing reaches disk");
    shell.enter_private();
    assert_ne!(shell.views()[0].profile, first_session, "a new session gets a new store");
    assert!(shell.history().is_empty());
    assert_eq!(shell.tab_count(), 1);
}

#[test]
fn spaces_and_profiles_have_their_own_tabs_and_stores() {
    let (_dir, mut shell) = open();
    let personal = shell.views()[0].profile.clone();
    assert!(add_space(&mut shell, "Work"));
    assert_eq!(shell.views()[0].profile, personal, "a space of the same profile shares its store");
    assert!(add_profile(&mut shell, "Banking"));
    let banking = shell.views()[0].profile.clone();
    assert_ne!(banking, personal);
    let names: Vec<(String, String)> = shell.spaces().into_iter().map(|s| (s.name, s.profile)).collect();
    assert_eq!(names, [("Home".into(), "Personal".into()), ("Work".into(), "Personal".into()), ("Home".into(), "Banking".into())]);
    shell.switch_space(0);
    assert_eq!(shell.views()[0].profile, personal);
}

#[test]
fn space_drafts_create_edit_and_add_a_profile_only_when_saved() {
    let (_dir, mut shell) = open();
    let mut draft = shell.space_draft(None);
    assert_eq!(draft.name, "Space 2");
    draft.name = "Work".into();
    draft.icon = "code".into();
    assert!(shell.save_space(&draft));
    let work = shell.model.space_ix;
    assert_eq!(shell.model.spaces[work].space.name, "Work", "a new space is switched to");

    let mut edit = shell.space_draft(Some(work));
    edit.name = "Home".into();
    assert!(!shell.save_space(&edit), "a name taken in the profile is refused");
    edit.name = "Code".into();
    edit.color = "blue".into();
    assert_eq!(shell.model.spaces[work].space.name, "Work", "nothing changes before Save");
    assert!(shell.save_space(&edit));
    assert_eq!((shell.model.spaces[work].space.name.as_str(), shell.model.spaces[work].space.color.as_str()), ("Code", "blue"));

    let mut fresh = shell.space_draft(None);
    fresh.profile = None;
    fresh.profile_name = "Banking".into();
    fresh.profile_color = "green".into();
    assert!(shell.save_space(&fresh));
    let names: Vec<(String, String)> = shell.spaces().into_iter().map(|s| (s.name, s.profile)).collect();
    assert_eq!(names, [("Home".into(), "Personal".into()), ("Code".into(), "Personal".into()), ("Space 3".into(), "Banking".into())]);
    assert_eq!(shell.model.current_space().map(|s| s.space.name.as_str()), Some("Space 3"));
}

#[test]
fn site_and_default_levels_reach_the_component() {
    let (_dir, mut shell) = open();
    shell.navigate("https://news.test/");
    finish(&mut shell, "https://news.test/a", "News");
    assert_eq!(shell.site_level(), Some(ShieldLevel::Standard));
    assert!(shell.set_site_level(Some(ShieldLevel::Strict)));
    assert_eq!(shell.site_level(), Some(ShieldLevel::Strict));
    assert_eq!(shell.views()[0].levels.get("news.test").map(String::as_str), Some("strict"));
    assert!(shell.set_site_level(None));
    assert_eq!(shell.views()[0].levels.keys().collect::<Vec<_>>(), ["*"], "only the profile's own level is left");
    assert_eq!(shell.views()[0].lists, ["ads", "cosmetic", "trackers", "malware", "annoyances"]);
    let store = shell.views()[0].profile.clone();
    assert!(shell.set_default_level(ShieldLevel::Lockdown));
    let view = &shell.views()[0];
    assert_eq!(view.level, ShieldLevel::Lockdown);
    assert_eq!(view.levels.get("*").map(String::as_str), Some("lockdown"));
    assert!(view.profile.starts_with("private:"), "Lockdown keeps website data in memory");
    assert!(shell.set_default_level(ShieldLevel::Standard));
    assert_eq!(shell.views()[0].profile, store);
}

#[test]
fn settings_persist() {
    let dir = tempfile::tempdir().unwrap();
    {
        let shell = TestShell::open(dir.path());
        shell.set_search_engine(2);
        assert!(shell.filter_switch("block_ads"));
        shell.set_filter_switch("block_ads", false);
    }
    let shell = TestShell::open(dir.path());
    assert_eq!(shell.search_engine(), 2);
    assert!(!shell.filter_switch("block_ads"));
}

#[test]
fn clear_data_empties_history_and_downloads() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.test/");
    finish(&mut shell, "https://a.test/", "A");
    shell.event(active(&shell), "download", &json!({ "state": "finished", "name": "a.pdf", "path": "/Documents/Downloads/a.pdf" }), 5);
    assert_eq!(shell.downloads().len(), 1);
    assert_eq!(shell.downloads()[0].path, "/Documents/Downloads/a.pdf");
    shell.clear_data();
    assert!(shell.history().is_empty() && shell.downloads().is_empty());
}

#[test]
fn prompts_queue_answer_and_remember_permissions() {
    let (_dir, mut shell) = open();
    let key = active(&shell);
    let alert = json!({ "id": 7, "kind": "text", "origin": "https://a.test", "message": "Name?", "default": "x" });
    assert_eq!(shell.event(key, "prompt", &alert, 1), vec![]);
    assert_eq!(shell.prompts.len(), 1);
    let answer = Answer { accept: true, text: Some("Ada".into()), user: None, password: None, remember: false };
    assert_eq!(shell.answer(answer), vec![Effect::Command(key, "answer", json!({ "id": 7, "accept": true, "text": "Ada" }))]);
    let camera = json!({ "id": 8, "kind": "camera", "origin": "https://a.test" });
    shell.event(key, "prompt", &camera, 2);
    let deny = Answer { accept: false, text: None, user: None, password: None, remember: true };
    shell.answer(deny);
    let again = json!({ "id": 9, "kind": "camera", "origin": "https://a.test" });
    assert_eq!(shell.event(key, "prompt", &again, 3), vec![Effect::Command(key, "answer", json!({ "id": 9, "accept": false }))]);
    assert!(shell.prompts.is_empty(), "a remembered decision is not asked again");
    shell.event(key, "prompt", &json!({ "id": 10, "kind": "camera", "origin": "https://b.test" }), 4);
    assert_eq!(shell.prompts.len(), 1, "other sites still ask");
    shell.close_tab(key);
    assert!(shell.prompts.is_empty(), "a closed tab's prompts go with it");
}

#[test]
fn a_crashed_tab_reloads_once_until_a_page_finishes() {
    let (_dir, mut shell) = open();
    let key = active(&shell);
    assert_eq!(shell.event(key, "crashed", &Value::Null, 1_000), vec![Effect::Command(key, "reload", Value::Null)]);
    assert_eq!(shell.event(key, "crashed", &Value::Null, 3_600_000), vec![], "a second crash before the page finished stays");
    assert!(shell.active_page().unwrap().error.is_some());
    shell.event(key, "finished", &json!({ "url": "https://a.test/", "title": "A" }), 3_600_001);
    assert_eq!(shell.event(key, "crashed", &Value::Null, 3_600_002), vec![Effect::Command(key, "reload", Value::Null)]);
}

#[test]
fn popups_open_tabs_and_failures_show() {
    let (_dir, mut shell) = open();
    shell.navigate("https://opener.test/");
    finish(&mut shell, "https://opener.test/page", "Opener");
    let key = active(&shell);
    shell.event(key, "popup", &json!({ "url": "https://pop.test/" }), 1);
    assert_eq!(shell.tab_count(), 2);
    let load = shell.views().into_iter().find(|v| Some(v.key) == shell.model.active).unwrap().load.unwrap();
    assert_eq!(load.url, "https://pop.test/");
    assert_eq!(load.opener.as_deref(), Some("https://opener.test/page"), "a popup is judged from its opener's page");
    let pop = active(&shell);
    shell.event(pop, "failed", &json!({ "url": "https://pop.test/", "message": "offline" }), 2);
    assert_eq!(shell.active_page().unwrap().error.as_deref(), Some("offline"));
    assert!(shell.dismiss_error().is_empty());
    assert_eq!(shell.active_page().unwrap().error, None);
    shell.event(pop, "blocked", &json!({ "url": "http://192.168.1.1/", "reason": "local_network" }), 3);
    assert!(shell.notice.as_deref().unwrap().contains("local network"));
    shell.event(pop, "found", &json!({ "matched": 3 }), 4);
    assert_eq!(shell.found, Some(3));
    assert_eq!(shell.zoom(9.0), vec![Effect::Command(pop, "zoom", json!({ "factor": 3.0 }))]);
}

#[test]
fn open_anyway_loads_a_refused_local_address_as_a_typed_load_once() {
    let (_dir, mut shell) = open();
    shell.navigate("https://evil.test/");
    finish(&mut shell, "https://evil.test/", "Evil");
    let key = active(&shell);
    shell.event(key, "blocked", &json!({ "url": "http://192.168.1.1/admin", "reason": "local_network" }), 2);
    assert_eq!(shell.notice.as_deref(), Some("An address on your local network was blocked: 192.168.1.1/admin"));
    assert!(shell.can_open_anyway());
    let before = shell.views()[0].load.clone().unwrap().seq;
    assert!(shell.open_anyway());
    let load = shell.views()[0].load.clone().unwrap();
    assert_eq!((load.url.as_str(), load.opener), ("http://192.168.1.1/admin", None), "a shell load without an opener is the user's own");
    assert!(load.seq > before);
    assert!(!shell.open_anyway(), "the offer is used once");

    shell.event(key, "blocked", &json!({ "url": "http://u:p@10.0.0.1/", "reason": "local_network" }), 3);
    assert!(!shell.can_open_anyway(), "no offer for an address with credentials");
    shell.event(key, "blocked", &json!({ "url": "http://10.0.0.1/", "reason": "local_network" }), 4);
    shell.event(key, "committed", &json!({ "url": "https://next.test/" }), 5);
    assert!(!shell.can_open_anyway(), "the offer goes with the next page");
    assert_eq!(shell.notice, None);
    shell.event(key, "blocked", &json!({ "url": "http://10.0.0.1/", "reason": "local_network" }), 6);
    shell.notice = Some("Tab closed".into());
    assert!(!shell.open_anyway(), "another notice replaced the offer");
}

#[test]
fn shield_rule_and_popup_blocks_never_offer_open_anyway() {
    let (_dir, mut shell) = open();
    shell.navigate("https://news.test/");
    finish(&mut shell, "https://news.test/", "News");
    let key = active(&shell);
    shell.event(key, "blocked", &json!({ "url": "https://tracker.example/pixel", "reason": "shield" }), 2);
    assert_eq!(shell.notice.as_deref(), Some("Shield blocked this address."));
    assert!(!shell.can_open_anyway());
    assert!(!shell.open_anyway());
    shell.event(key, "blocked", &json!({ "url": "http://192.168.1.1/", "reason": "shield", "popup": true }), 3);
    assert!(!shell.can_open_anyway(), "a pop-up the page opened is never the user's own load");
    shell.event(key, "blocked", &json!({ "url": "https://tracker.example/pixel", "reason": "local_network" }), 4);
    assert!(!shell.can_open_anyway(), "only an address on the local network");
}

#[test]
fn a_space_round_trip_reloads_the_page_the_tab_shows_and_drops_its_prompts() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.test/");
    finish(&mut shell, "https://b.test/later", "B");
    let key = active(&shell);
    shell.event(key, "prompt", &json!({ "id": 1, "kind": "alert", "origin": "https://b.test" }), 3);
    assert!(add_space(&mut shell, "Work"));
    assert!(shell.prompts.is_empty(), "a hidden tab's view is gone, and its prompts with it");
    shell.switch_space(0);
    assert_eq!(active(&shell), key);
    assert_eq!(shell.views()[0].load.as_ref().unwrap().url, "https://b.test/later");
}

#[test]
fn an_evicted_tab_loses_its_prompts() {
    let (_dir, mut shell) = open();
    let first = active(&shell);
    shell.event(first, "prompt", &json!({ "id": 1, "kind": "alert", "origin": "https://a.test" }), 1);
    for n in 0..tarn_mobile::shell::LIVE_TABS {
        shell.new_tab(Some(&format!("https://t{n}.test/")));
    }
    assert!(shell.prompts.is_empty());
}

#[test]
fn closing_the_private_space_keeps_lockdown_profiles_stores() {
    let (_dir, mut shell) = open();
    shell.set_default_level(ShieldLevel::Lockdown);
    let lockdown = shell.views()[0].profile.clone();
    shell.enter_private();
    shell.close_private();
    assert_eq!(shell.views()[0].profile, lockdown);
}

#[test]
fn background_tabs_cannot_take_over_the_screen_or_the_external_prompt() {
    let (_dir, mut shell) = open();
    let background = active(&shell);
    let front = shell.new_tab(Some("https://front.test/")).unwrap();
    shell.event(background, "prompt", &json!({ "id": 1, "kind": "alert", "origin": "https://bg.test" }), 1);
    assert!(shell.active_prompt().is_none(), "a background tab's prompt waits");
    let ok = Answer { accept: true, text: None, user: None, password: None, remember: false };
    assert_eq!(shell.answer(ok.clone()), vec![], "and is not answered from another tab");
    shell.event(background, "external", &json!({ "url": "evil:x", "origin": "https://evil.test" }), 2);
    assert!(shell.external.is_none());
    shell.event(front, "external", &json!({ "url": "mailto:a@b.test" }), 3);
    shell.event(front, "external", &json!({ "url": "evil:swap" }), 4);
    assert_eq!(shell.external.as_ref().map(|(_, u)| u.as_str()), Some("mailto:a@b.test"), "a pending choice is never replaced");
    shell.activate(background);
    assert_eq!(shell.answer(ok), vec![Effect::Command(background, "answer", json!({ "id": 1, "accept": true }))]);
}

#[test]
fn a_tab_queues_at_most_eight_prompts() {
    let (_dir, mut shell) = open();
    let key = active(&shell);
    for id in 0..8 {
        assert_eq!(shell.event(key, "prompt", &json!({ "id": id, "kind": "alert", "origin": "https://a.test" }), 1), vec![]);
    }
    assert_eq!(
        shell.event(key, "prompt", &json!({ "id": 8, "kind": "alert", "origin": "https://a.test" }), 1),
        vec![Effect::Command(key, "answer", json!({ "id": 8, "accept": false }))]
    );
}

#[test]
fn embedded_frames_permissions_are_never_remembered() {
    let (_dir, mut shell) = open();
    let key = active(&shell);
    let framed = json!({ "id": 1, "kind": "camera", "origin": "https://widget.test", "topOrigin": "https://site.test" });
    shell.event(key, "prompt", &framed, 1);
    assert!(!shell.active_prompt().unwrap().rememberable());
    shell.answer(Answer { accept: true, text: None, user: None, password: None, remember: true });
    let again = json!({ "id": 2, "kind": "camera", "origin": "https://widget.test" });
    assert_eq!(shell.event(key, "prompt", &again, 2), vec![], "nothing was stored for the widget");
}

#[test]
fn the_top_pages_own_permission_is_remembered_when_the_component_names_it() {
    let (_dir, mut shell) = open();
    let key = active(&shell);
    let own = json!({ "id": 1, "kind": "location", "origin": "https://site.test", "topOrigin": "https://site.test" });
    shell.event(key, "prompt", &own, 1);
    assert!(shell.active_prompt().unwrap().rememberable());
    shell.answer(Answer { accept: true, text: None, user: None, password: None, remember: true });
    let again = json!({ "id": 2, "kind": "location", "origin": "https://site.test", "topOrigin": "https://site.test" });
    assert_eq!(shell.event(key, "prompt", &again, 2), vec![Effect::Command(key, "answer", json!({ "id": 2, "accept": true }))]);
}

#[test]
fn clear_data_gives_lockdown_and_private_profiles_a_fresh_store() {
    let (_dir, mut shell) = open();
    shell.set_default_level(ShieldLevel::Lockdown);
    let before = shell.views()[0].profile.clone();
    shell.clear_data();
    let after = shell.views()[0].profile.clone();
    assert_ne!(before, after);
    assert_eq!(before.rsplit_once(':').unwrap().0, after.rsplit_once(':').unwrap().0, "same profile, new store");
    shell.enter_private();
    let private = shell.views()[0].profile.clone();
    shell.clear_data();
    assert_ne!(shell.views()[0].profile, private);
}

#[test]
fn an_external_choice_goes_away_when_another_tab_is_shown() {
    let (_dir, mut shell) = open();
    let first = active(&shell);
    let second = shell.new_tab(Some("https://b.test/")).unwrap();
    shell.event(second, "external", &json!({ "url": "tel:123" }), 1);
    assert!(shell.external.is_some());
    shell.activate(first);
    assert!(shell.external.is_none());
}

thread_local! {
    static HANDED_OFF: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

macro_rules! test_platform {
    ($name:ident, $private_process:expr) => {
        struct $name(String);

        impl browser_core::WebsiteData for $name {
            type RuleLists = ();

            fn persistent(uuid: &str, _: &str, _: std::rc::Rc<dyn Fn(engine::Event)>) -> Option<Self> {
                if $private_process { None } else { Some($name(uuid.to_ascii_lowercase())) }
            }

            fn ephemeral(downloads: std::rc::Rc<dyn Fn(engine::Event)>) -> Option<Self> {
                if $private_process {
                    Some($name("private:0123456789abcdef0123456789abcdef".into()))
                } else {
                    Memory::ephemeral(downloads).map(|m| $name(m.store_id()))
                }
            }

            fn wipe_persistent(_: &str) {}

            fn import_cookies(&self, cookies: Vec<engine::cookies::Cookie>, _: i64, done: Box<dyn FnOnce(usize, usize)>) {
                done(0, cookies.len());
            }
        }

        impl Platform for $name {
            fn store_id(&self) -> String {
                self.0.clone()
            }

            fn private_process() -> bool {
                $private_process
            }

            fn open_private_space() -> Option<bool> {
                HANDED_OFF.with(|h| h.borrow_mut().push("open"));
                Some(true)
            }

            fn close_private_space() -> Option<bool> {
                if !$private_process {
                    return None;
                }
                HANDED_OFF.with(|h| h.borrow_mut().push("close"));
                Some(true)
            }
        }
    };
}

test_platform!(MainProcess, false);
test_platform!(PrivateProcess, true);

#[test]
fn a_private_space_in_another_process_is_opened_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = Shell::<MainProcess>::open(dir.path());
    shell.enter_private();
    assert!(!shell.model.private(), "the main process stays where it was");
    assert!(shell.spaces().iter().all(|s| !s.private));
    assert_eq!(HANDED_OFF.with(|h| h.take()), vec!["open"]);
}

#[test]
fn the_private_process_shows_only_the_private_space_and_closes_through_the_platform() {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = Shell::<PrivateProcess>::open(dir.path());
    assert!(shell.model.private());
    assert!(shell.views()[0].profile.starts_with("private:"));
    assert!(shell.spaces().iter().all(|s| s.private));
    assert!(!add_profile(&mut shell, "Work"), "no persistent profile in the private process");
    shell.close_private();
    assert!(shell.model.private());
    assert_eq!(HANDED_OFF.with(|h| h.take()), vec!["close"]);
}

#[test]
fn the_private_process_reaches_no_other_space_and_offers_no_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = Shell::<PrivateProcess>::open(dir.path());
    let here = shell.model.space_ix;
    let others: Vec<usize> = (0..shell.model.spaces.len()).filter(|&ix| !shell.model.spaces[ix].private).collect();
    assert!(!others.is_empty(), "the model still lists the normal spaces");
    for ix in others {
        shell.choose_space(ix, 1);
        assert_eq!(shell.model.space_ix, here, "space {ix} is not reachable from the private process");
    }
    for step in [browser_core::sidebar::Step::Next, browser_core::sidebar::Step::Previous] {
        assert_eq!(shell.neighbour_space(step), None);
    }
    assert!(!shell.can_add_space());
    assert!(Shell::<MainProcess>::open(tempfile::tempdir().unwrap().path()).can_add_space());
}

#[test]
fn every_copy_link_says_so_and_copies_only_web_links() {
    let (_dir, mut shell) = open();
    shell.navigate("data:text/html,hi");
    let data = active(&shell);
    assert_eq!(shell.copy_link_of(data, false), None, "a data: page has no link to copy");
    assert_eq!(shell.copy_link_of(data, true), None);
    shell.new_tab(None);
    shell.navigate("https://example.com/?utm_source=x");
    finish(&mut shell, "https://example.com/?utm_source=x", "Example");
    let web = active(&shell);
    shell.notice = None;
    assert_eq!(shell.copy_link(false).as_deref(), Some("https://example.com/?utm_source=x"));
    assert_eq!(shell.notice.as_deref(), Some("Link copied"));
    shell.notice = None;
    assert_eq!(shell.copy_link_of(web, true).as_deref(), Some("https://example.com/"));
    assert!(shell.notice.as_deref().is_some_and(|n| n.starts_with("Link copied")));
}

#[test]
fn closing_a_pinned_tab_or_tile_unloads_it_and_keeps_it() {
    let (_dir, mut shell) = open();
    shell.navigate("https://example.com/");
    let pinned = active(&shell);
    shell.toggle_pin(pinned);
    shell.new_tab(None);
    shell.navigate("https://example.org/");
    let tile = active(&shell);
    browser_core::sidebar::toggle_tile(&mut shell.model, &shell.core, tile);
    for key in [pinned, tile] {
        shell.activate(key);
        shell.close_tab(key);
        assert!(shell.model.tab(key).is_some(), "a pinned tab or tile stays in the sidebar");
        assert!(!shell.model.lifecycle.live().any(|k| k == key), "its page is unloaded");
    }
}

#[test]
fn closing_several_tabs_says_how_many_and_undoes_together() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.test/");
    let keep = active(&shell);
    for url in ["https://b.test/", "https://c.test/"] {
        shell.new_tab(None);
        shell.navigate(url);
    }
    let others = browser_core::sidebar::other_unpinned(&shell.model, keep);
    assert_eq!(others.len(), 2);
    shell.close_many(&others);
    assert_eq!(shell.notice.as_deref(), Some("2 tabs closed"));
    assert!(shell.can_undo());
    assert!(shell.undo_close());
    assert_eq!(shell.model.tabs.len(), 3, "Undo brings both back");
    shell.close_many(&[]);
    assert_eq!(shell.model.tabs.len(), 3);
}

#[test]
fn zoom_steps_from_the_page_and_resets() {
    let (_dir, mut shell) = open();
    shell.navigate("https://example.com/");
    let key = active(&shell);
    shell.zoom_by(0.1);
    shell.zoom_by(0.1);
    assert!((shell.pages[&key].zoom - 1.2).abs() < 1e-9);
    shell.zoom_by(0.0);
    assert!((shell.pages[&key].zoom - 1.0).abs() < 1e-9);
}

#[test]
fn a_finished_load_that_never_committed_leaves_the_tab_where_it_was() {
    let (_dir, mut shell) = open();
    shell.navigate("https://public.test/redirect");
    let key = active(&shell);
    let refused = json!({ "url": "http://localhost:8771/local", "title": "", "canGoBack": false, "canGoForward": false });
    shell.event(key, "finished", &refused, 1);
    assert_eq!(shell.active_url(), "https://public.test/redirect");
    assert!(!shell.model.active_tab().unwrap().url.contains("localhost"), "the refused URL is not kept for a later reload");
}

#[test]
fn a_popup_tab_is_judged_from_the_committed_opener_and_saves_no_url_before_it_commits() {
    let (_dir, mut shell) = open();
    shell.navigate("https://public.test/");
    finish(&mut shell, "https://public.test/", "Public");
    shell.navigate("http://192.168.1.1/");
    let opener = active(&shell);
    shell.event(opener, "popup", &json!({ "url": "https://public.test/redirect" }), 3);
    let load = shell.views().into_iter().find(|v| Some(v.key) == shell.model.active).unwrap().load.unwrap();
    assert_eq!(load.opener.as_deref(), Some("https://public.test/"), "a load that has not committed is not the opener");
    assert_eq!(shell.model.active_tab().unwrap().url, "", "a restored popup tab must not reload the redirect as a typed load");
}

#[test]
fn clear_data_in_a_private_process_says_website_data_goes_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = Shell::<PrivateProcess>::open(dir.path());
    shell.clear_data();
    assert!(shell.notice.as_deref().unwrap().contains("when the private space closes"));
}

#[test]
fn a_finished_load_after_a_commit_keeps_a_same_document_url() {
    let (_dir, mut shell) = open();
    shell.navigate("https://app.test/");
    let key = active(&shell);
    shell.event(key, "committed", &json!({ "url": "https://app.test/", "title": "App" }), 1);
    shell.event(key, "finished", &json!({ "url": "https://app.test/inbox", "title": "App" }), 2);
    let page = shell.active_page().unwrap();
    assert_eq!(page.url, "https://app.test/inbox");
    assert!(!page.loading);
}

#[test]
fn only_a_commit_saves_a_tab_url() {
    let (_dir, mut shell) = open();
    shell.navigate("https://public.test/");
    finish(&mut shell, "https://public.test/", "Public");
    let opener = active(&shell);
    shell.event(opener, "popup", &json!({ "url": "https://public.test/redirect" }), 3);
    let pop = active(&shell);
    shell.event(pop, "title", &json!({ "title": "Forbidden" }), 4);
    shell.event(pop, "finished", &json!({ "url": "https://public.test/redirect", "title": "" }), 5);
    assert_eq!(shell.model.active_tab().unwrap().url, "", "title and finished before a commit save nothing");
    shell.close_tab(pop);
    assert!(shell.reopen_closed());
    assert!(
        shell.views().iter().all(|v| v.load.as_ref().is_none_or(|l| l.url != "https://public.test/redirect")),
        "a reopened popup does not reload its redirect as a typed load"
    );
}

#[test]
fn a_page_that_has_not_committed_opens_no_popup() {
    let (_dir, mut shell) = open();
    shell.navigate("https://public.test/");
    let key = active(&shell);
    shell.event(key, "popup", &json!({ "url": "https://public.test/redirect" }), 1);
    assert_eq!(shell.tab_count(), 1);
}

#[test]
fn a_failed_load_keeps_its_error_when_the_error_page_finishes() {
    let (_dir, mut shell) = open();
    shell.navigate("https://nowhere.test/");
    let key = active(&shell);
    shell.event(key, "failed", &json!({ "url": "https://nowhere.test/", "message": "host not found" }), 1);
    shell.event(key, "finished", &json!({ "url": "https://nowhere.test/", "title": "" }), 2);
    assert_eq!(shell.active_page().unwrap().error.as_deref(), Some("host not found"));
    assert_eq!(shell.model.active_tab().unwrap().url, "");
    shell.event(key, "committed", &json!({ "url": "https://nowhere.test/", "title": "Webpage not available" }), 3);
    let page = shell.active_page().unwrap();
    assert_eq!(page.error.as_deref(), Some("host not found"), "Android commits its own error page after failed");
    assert!(!page.loading);
}

#[test]
fn views_carry_the_themes_website_sheets_but_never_in_private() {
    let (_dir, mut shell) = open();
    let css = "@tarn-theme { name: Site; } :root { --tarn-accent: #224466 } @tarn-site *.example.org { a { color: #ff0000 } }";
    let id = browser_core::theme_library::add(&shell.core.disk, css, 1).unwrap();
    shell.apply_theme();
    assert!(shell.views()[0].theme_styles.is_empty(), "an unverified theme's website styles start off");
    browser_core::theme_library::set_page_css(&shell.core.disk, id, true).unwrap();
    shell.apply_theme();
    assert_eq!(shell.views()[0].theme_styles, [("*.example.org".to_owned(), "a{color:#ff0000;}".to_owned())]);
    let profile = shell.profile_id().unwrap();
    browser_core::theme_library::set_pages(&shell.core.disk, profile, false).unwrap();
    assert!(shell.views()[0].theme_styles.is_empty(), "the profile turned theme styles off");
    browser_core::theme_library::set_pages(&shell.core.disk, profile, true).unwrap();
    shell.enter_private();
    assert!(shell.model.private());
    assert!(shell.views()[0].theme_styles.is_empty(), "never in a private space");
}

#[test]
fn split_panes_stay_live_and_a_closed_pane_focuses_its_neighbour() {
    let (_dir, mut shell) = open();
    shell.set_wide(true);
    let a = active(&shell);
    let b = shell.new_tab(Some("https://b.example/")).unwrap();
    let c = shell.new_tab(Some("https://c.example/")).unwrap();
    shell.activate(a);
    shell.split_with(c);
    assert_eq!(shell.model.active, Some(c));
    assert_eq!(shell.shown_split().map(|s| s.panes().to_vec()), Some(vec![a, c]));
    assert_eq!(shell.pane_share(a), Some((0., 0., 0.5, 1.)));
    assert_eq!(shell.pane_share(b), None);
    for _ in 0..tarn_mobile::shell::LIVE_TABS {
        shell.new_tab(Some("https://more.example/"));
    }
    shell.activate(c);
    let live: Vec<_> = shell.views().iter().map(|v| v.key).collect();
    assert!(live.contains(&a) && live.contains(&c), "both panes of the split on screen are live");
    shell.close_tab(c);
    assert_eq!(shell.model.active, Some(a), "the pane beside it takes the focus, not the tab before it");
    assert!(!shell.splits.contains(a), "a split left with one pane ends");
    shell.split_with(b);
    shell.leave_split(b);
    assert_eq!((shell.model.active, shell.shown_split()), (Some(b), None));
}

#[test]
fn a_lone_tab_splits_with_a_copy_of_its_page() {
    let (_dir, mut shell) = open();
    let blank = active(&shell);
    assert_eq!(shell.model.tabs.len(), 1);
    shell.split_with(blank);
    assert!(shell.shown_split().is_none() && shell.model.tabs.len() == 1, "a blank tab has no page to copy");
    let a = shell.new_tab(Some("https://a.example/")).unwrap();
    shell.close_tab(blank);
    shell.activate(a);
    assert_eq!(shell.model.tabs.len(), 1);
    shell.split_with(a);
    let panes = shell.shown_split().map(|s| s.panes().to_vec()).unwrap_or_default();
    assert_eq!((panes.len(), panes[0]), (2, a));
    assert_eq!(shell.model.tab(panes[1]).map(|t| t.url.clone()), shell.model.tab(a).map(|t| t.url.clone()));
}

#[test]
fn closing_the_private_space_ends_its_splits_even_when_their_panes_sleep() {
    let (_dir, mut shell) = open();
    shell.set_wide(true);
    shell.enter_private();
    let a = shell.new_tab(Some("https://a.example/")).unwrap();
    let b = shell.new_tab(Some("https://b.example/")).unwrap();
    shell.activate(a);
    shell.split_with(b);
    assert!(shell.splits.contains(a) && a.private);
    let c = shell.new_tab(Some("https://c.example/")).unwrap();
    shell.activate(c);
    for _ in 0..tarn_mobile::shell::LIVE_TABS {
        shell.new_tab(Some("https://more.example/"));
    }
    shell.close_private();
    assert!(!shell.splits.contains(a) && !shell.splits.contains(b), "no private pane outlives its session");
}

#[test]
fn a_press_on_a_page_focuses_its_pane_only_in_the_split_on_screen() {
    let (_dir, mut shell) = open();
    let a = active(&shell);
    let b = shell.new_tab(Some("https://b.example/")).unwrap();
    let c = shell.new_tab(Some("https://c.example/")).unwrap();
    shell.activate(a);
    shell.split_with(b);
    shell.event(a, "pressed", &Value::Null, 1);
    assert_eq!(shell.model.active, Some(b), "a phone shows one pane, so a press focuses nothing");
    shell.set_wide(true);
    shell.event(a, "pressed", &Value::Null, 1);
    assert_eq!(shell.model.active, Some(a));
    shell.event(c, "pressed", &Value::Null, 1);
    assert_eq!(shell.model.active, Some(a), "a tab outside the split is not focused by its page");
}

#[test]
fn same_document_history_moves_back_and_forward() {
    let (_dir, mut shell) = open();
    shell.navigate("https://a.example/");
    let key = active(&shell);
    let page = json!({ "url": "https://a.example/", "title": "A", "canGoBack": false, "canGoForward": false });
    shell.event(key, "committed", &page, 1);
    shell.event(key, "history", &json!({ "canGoBack": true, "canGoForward": false }), 2);
    assert!(shell.active_page().is_some_and(|p| p.back && !p.forward));
    shell.event(key, "history", &json!({ "canGoBack": false, "canGoForward": true }), 3);
    assert!(shell.active_page().is_some_and(|p| !p.back && p.forward));
}

#[test]
fn commands_with_nothing_to_act_on_are_not_offered() {
    let (_dir, mut shell) = open();
    assert!(shell.duplicate_tabs().is_empty() && !shell.zappable());
    finish(&mut shell, "https://a.example/", "A");
    assert!(shell.zappable());
    shell.new_tab(Some("https://a.example/"));
    assert_eq!(shell.duplicate_tabs().len(), 1, "one of the two tabs on the same page");
    shell.enter_private();
    finish(&mut shell, "https://a.example/", "A");
    assert!(!shell.zappable(), "a private space keeps no hidden elements");
}

#[test]
fn views_share_the_profile_props_read_once() {
    let (_dir, mut shell) = open();
    shell.new_tab(Some("https://b.example/"));
    let props = shell.profile_props();
    assert_eq!(shell.views_from(props.as_ref()), shell.views());
    assert!(shell.views_from(None).is_empty());
}

#[test]
fn waking_a_sleeping_pane_keeps_the_live_panes_views() {
    let (_dir, mut shell) = open();
    shell.set_wide(true);
    let a = active(&shell);
    let b = shell.new_tab(Some("https://b.example/")).unwrap();
    shell.activate(a);
    shell.split_with(b);
    shell.activate(a);
    for _ in 1..tarn_mobile::shell::LIVE_TABS {
        shell.new_tab(Some("https://more.example/"));
    }
    let before = shell.model.lifecycle.born(a);
    assert!(before.is_some() && shell.model.lifecycle.born(b).is_none(), "a is still live, b sleeps");
    shell.activate(a);
    assert_eq!(shell.model.lifecycle.born(a), before, "waking b does not renumber a's view");
    assert!(shell.model.lifecycle.born(b).is_some());
}

#[test]
fn store_links_install_only_from_a_profile_tab_outside_lockdown() {
    let (_dir, mut shell) = open();
    finish(&mut shell, "https://example.com/", "E");
    let key = active(&shell);
    assert!(shell.store_link_allowed(key));
    assert!(shell.set_site_level(Some(ShieldLevel::Lockdown)));
    assert!(!shell.store_link_allowed(key), "from a Lockdown site");
    assert!(shell.set_site_level(None));
    assert!(shell.store_link_allowed(key));
    shell.navigate("https://extensions.tarnbrowser.com/install/helper");
    shell.event(key, browser_core::extensions::STORE_EVENT, &json!({ "id": "helper" }), 3);
    assert!(!shell.pages[&key].loading, "the cancelled load stops");
    shell.enter_private();
    assert!(!shell.store_link_allowed(active(&shell)), "never from the private space");
    assert!(!shell.store_link_allowed(key), "only from the space shown");
    shell.close_private();
    shell.open_burner();
    assert!(!shell.store_link_allowed(active(&shell)), "never from a burner tab");
    shell.burn();
    assert!(shell.set_default_level(ShieldLevel::Lockdown));
    assert!(!shell.store_link_allowed(active(&shell)), "never in a Lockdown profile");
}
