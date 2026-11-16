use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use store::Schedule;
use tarn_mobile::data::{Memory, Platform};
use tarn_mobile::scripts::{HISTORY_TEXT, SORT_DOWNLOADS, TransferState};
use tarn_mobile::shell::{Effect, Shell};

type TestShell = Shell<Memory>;

fn open() -> (tempfile::TempDir, TestShell) {
    let dir = tempfile::tempdir().unwrap();
    let shell = TestShell::open(dir.path());
    (dir, shell)
}

fn active(shell: &TestShell) -> TabKey {
    shell.model.active.expect("an active tab")
}

fn visit(shell: &mut TestShell, url: &str, title: &str) -> Vec<Effect> {
    let key = active(shell);
    shell.navigate(url);
    let page = json!({ "url": url, "title": title, "canGoBack": false, "canGoForward": false });
    shell.event(key, "committed", &page, 1);
    shell.event(key, "finished", &page, 2)
}

fn command<'a>(effects: &'a [Effect], name: &str) -> Option<&'a Value> {
    effects.iter().find_map(|e| match e {
        Effect::Command(_, n, args) if *n == name => Some(args),
        _ => None,
    })
}

fn seq(args: &Value) -> u64 {
    args["seq"].as_u64().unwrap()
}

#[test]
fn cookie_banner_lists_follow_their_switch() {
    let (_dir, shell) = open();
    assert!(shell.views()[0].lists.contains(&"annoyances".to_owned()));
    shell.set_filter_switch("block_annoyances", false);
    assert!(!shell.views()[0].lists.contains(&"annoyances".to_owned()));
}

#[test]
fn duplicates_close_as_one_entry_that_undo_brings_back() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    shell.new_tab(None);
    visit(&mut shell, "https://a.test/#x", "A");
    shell.new_tab(None);
    visit(&mut shell, "https://b.test/", "B");
    shell.new_tab(None);
    visit(&mut shell, "https://b.test/", "B");
    assert_eq!(shell.tab_count(), 4);
    shell.close_duplicates();
    assert_eq!(shell.tab_count(), 2);
    assert_eq!(shell.notice.as_deref(), Some("2 tabs closed"));
    assert!(shell.can_undo());
    assert!(shell.undo_close());
    assert_eq!(shell.tab_count(), 4);
    shell.close_duplicates();
    shell.notice = Some("something else".into());
    assert!(!shell.can_undo(), "Undo belongs to its own toast");
}

#[test]
fn clean_links_drop_tracking_parameters_and_count_them() {
    let (_dir, mut shell) = open();
    let id = shell.profile_id().unwrap();
    let counted = |shell: &TestShell| shell.store().shield_week(id).unwrap().iter().map(|d| d.tracking_params).sum::<i64>();
    visit(&mut shell, "https://a.test/p?id=3&utm_source=x&fbclid=y", "A");
    assert_eq!(counted(&shell), 2, "the typed address loses its two");
    let copied = shell.copy_clean_link().unwrap();
    assert_eq!(copied, "https://a.test/p?id=3");
    assert_eq!(shell.notice.as_deref(), Some("Link copied, 2 tracking parameters removed"));
    assert_eq!(counted(&shell), 4, "the copied link of the committed page loses its two");
    visit(&mut shell, "https://a.test/p?id=3", "A");
    shell.copy_clean_link().unwrap();
    assert_eq!(counted(&shell), 4, "a page already clean counts nothing more");
}

#[test]
fn site_css_dark_mode_and_zaps_reach_the_component_and_reload_the_host() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let first = shell.views()[0].load.clone().unwrap().seq;
    assert!(shell.set_site_dark(true));
    assert!(shell.set_site_css("body {\n color: red }"));
    let view = shell.views()[0].clone();
    let sheet = &view.styles["a.test"];
    assert!(sheet.starts_with(engine::page_scripts::DARK_CSS) && sheet.ends_with("body {  color: red }"));
    assert!(view.load.unwrap().seq > first, "the host's tab loads again with its new sheet");
    assert!(!shell.set_site_css(&"a".repeat(64 * 1024 + 1)));

    let picked = shell.pick();
    let args = command(&picked, "pick").unwrap().clone();
    let key = active(&shell);
    shell.event(key, "picked", &json!({ "seq": seq(&args) + 1, "url": "https://a.test/", "selector": "#ad" }), 3);
    assert_eq!(shell.site_zaps(), 0, "an answer to another request is dropped");
    shell.event(key, "picked", &json!({ "seq": seq(&args), "url": "https://other.test/", "selector": "#ad" }), 3);
    assert_eq!(shell.site_zaps(), 0, "a selector for another host is dropped");
    shell.pick();
    let args = shell.picking.unwrap().1;
    shell.event(key, "picked", &json!({ "seq": args, "url": "https://a.test/", "selector": "#ad" }), 3);
    assert_eq!(shell.site_zaps(), 1);
    assert_eq!(shell.views()[0].zaps["a.test"], vec!["#ad".to_owned()]);
    shell.clear_site_zaps();
    assert!(shell.views()[0].zaps.is_empty());
    assert!(!shell.views()[0].sort_downloads);
    shell.set_setting_on(SORT_DOWNLOADS, true);
    assert!(shell.views()[0].sort_downloads);
}

#[test]
fn private_spaces_never_send_or_keep_zaps() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    shell.pick();
    let seq = shell.picking.unwrap().1;
    shell.event(active(&shell), "picked", &json!({ "seq": seq, "url": "https://a.test/", "selector": "#ad" }), 3);
    shell.enter_private();
    visit(&mut shell, "https://a.test/", "A");
    assert!(shell.pick().is_empty());
    assert!(shell.views()[0].zaps.is_empty());
}

#[test]
fn the_reader_probes_after_a_load_and_parses_only_the_current_pages_article() {
    let (_dir, mut shell) = open();
    let effects = visit(&mut shell, "https://a.test/story", "Story");
    let probe = command(&effects, "readArticle").unwrap().clone();
    assert_eq!(probe["probe"], json!(true));
    let key = active(&shell);
    shell.event(key, "article", &json!({ "seq": seq(&probe), "url": "https://a.test/other", "text": "1" }), 3);
    assert!(!shell.active_page().unwrap().article, "an answer for another page is dropped");
    let effects = visit(&mut shell, "https://a.test/story", "Story");
    let probe = command(&effects, "readArticle").unwrap().clone();
    shell.event(key, "article", &json!({ "seq": seq(&probe), "url": "https://a.test/story", "text": "1" }), 3);
    assert!(shell.active_page().unwrap().article);

    let open = shell.open_reader();
    let args = command(&open, "readArticle").unwrap().clone();
    assert_eq!(args["probe"], json!(false));
    let article = json!({ "t": "Story", "b": "Ann", "p": [["h", "Part"], ["p", "Words"]] }).to_string();
    shell.event(key, "article", &json!({ "seq": seq(&args), "url": "https://a.test/story", "text": article }), 3);
    let reader = shell.reader.clone().unwrap();
    assert_eq!(reader.article.title, "Story");
    assert_eq!(reader.spoken(), "Story\n\nPart\n\nWords");
    shell.event(key, "article", &json!({ "seq": seq(&args), "url": "https://a.test/story", "text": article }), 3);
    let again = shell.open_reader();
    let args = command(&again, "readArticle").unwrap().clone();
    shell.reader = None;
    shell.event(key, "article", &json!({ "seq": seq(&args), "url": "https://a.test/story", "text": "{" }), 3);
    assert!(shell.reader.is_none());
    assert_eq!(shell.notice.as_deref(), Some("This page has no article to read."));
    shell.reader = Some(reader);
    shell.event(key, "committed", &json!({ "url": "https://a.test/next", "title": "", "canGoBack": true, "canGoForward": false }), 4);
    assert!(shell.reader.is_none(), "a navigation closes the reader");
}

struct PageWorld(Memory);

impl Platform for PageWorld {
    fn store_id(&self) -> String {
        self.0.store_id()
    }

    fn page_world_scripts() -> bool {
        true
    }
}

impl browser_core::WebsiteData for PageWorld {
    type RuleLists = ();

    fn persistent(uuid: &str, own: &str, sink: std::rc::Rc<dyn Fn(engine::Event)>) -> Option<Self> {
        Memory::persistent(uuid, own, sink).map(PageWorld)
    }

    fn ephemeral(sink: std::rc::Rc<dyn Fn(engine::Event)>) -> Option<Self> {
        Memory::ephemeral(sink).map(PageWorld)
    }

    fn wipe_persistent(_: &str) {}

    fn import_cookies(&self, cookies: Vec<engine::cookies::Cookie>, now: i64, done: Box<dyn FnOnce(usize, usize)>) {
        self.0.import_cookies(cookies, now, done);
    }
}

#[test]
fn page_world_platforms_run_no_script_a_page_did_not_ask_for() {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = Shell::<PageWorld>::open(dir.path());
    let key = shell.model.active.unwrap();
    shell.navigate("https://a.test/story");
    let page = json!({ "url": "https://a.test/story", "title": "Story", "canGoBack": false, "canGoForward": false });
    shell.event(key, "committed", &page, 1);
    assert!(shell.event(key, "finished", &page, 2).is_empty(), "no probe after a load");
    assert!(shell.offers_reader(), "the reader is offered without a probe");
    assert_eq!(command(&shell.open_reader(), "readArticle").unwrap()["probe"], json!(false));
}

#[test]
fn page_text_is_kept_only_when_switched_on_and_never_in_private_spaces() {
    let (_dir, mut shell) = open();
    let effects = visit(&mut shell, "https://a.test/", "A");
    assert!(command(&effects, "readText").is_none());
    shell.set_setting_on(HISTORY_TEXT, true);
    let effects = visit(&mut shell, "https://a.test/", "A");
    let args = command(&effects, "readText").unwrap().clone();
    assert_eq!(args["maxChars"], json!(store::MAX_HISTORY_TEXT));
    shell.event(active(&shell), "pageText", &json!({ "seq": seq(&args), "url": "https://a.test/", "text": "quokka marsupial" }), 3);
    assert_eq!(shell.search_history("quokka").len(), 1);
    assert!(shell.filter_history("quokka").is_empty(), "typing filters titles and addresses; the text search follows a pause");
    assert!(shell.search_history("qu").is_empty(), "the text search starts at three characters");
    shell.set_setting_on(HISTORY_TEXT, false);
    assert!(shell.search_history("quokka").is_empty(), "switching off forgets the text");

    shell.set_setting_on(HISTORY_TEXT, true);
    shell.enter_private();
    let effects = visit(&mut shell, "https://a.test/", "A");
    assert!(command(&effects, "readText").is_none());
}

#[test]
fn find_in_tabs_lists_names_at_once_and_text_rows_above_them() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "Quokka facts");
    shell.new_tab(None);
    visit(&mut shell, "https://b.test/", "Other");
    let effects = shell.find_in_tabs("quokka");
    assert_eq!(effects.len(), 2, "every live tab with a page searches");
    let rows = &shell.find_tabs.rows;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].snippet.is_none());
    let seq = shell.find_tabs.seq;
    let b = active(&shell);
    shell.event(b, "textFound", &json!({ "seq": seq, "url": "https://b.test/", "snippets": ["a quokka here"] }), 3);
    shell.event(b, "textFound", &json!({ "seq": seq, "url": "https://b.test/", "snippets": [], "matched": 4 }), 3);
    shell.event(b, "textFound", &json!({ "seq": seq - 1, "url": "https://b.test/", "snippets": ["late"] }), 3);
    let rows = &shell.find_tabs.rows;
    assert_eq!(rows[0].snippet.as_deref(), Some("a quokka here"));
    assert_eq!(rows[1].snippet.as_deref(), Some("4 matches on this page"));
    assert_eq!(rows.len(), 3);
}

#[test]
fn shield_events_count_once_per_page_and_skip_local_popups() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let key = active(&shell);
    for _ in 0..3 {
        shell.event(key, "shielded", &json!({ "kind": "https_upgrade", "url": "https://a.test/" }), 3);
    }
    shell.event(key, "shielded", &json!({ "kind": "popup", "url": "http://localhost/" }), 3);
    shell.event(key, "shielded", &json!({ "kind": "popup", "url": "https://a.test/" }), 3);
    shell.event(key, "shielded", &json!({ "kind": "local_network", "url": "https://a.test/" }), 3);
    let id = shell.profile_id().unwrap();
    let week = shell.store().shield_week(id).unwrap();
    let sum = |f: fn(&store::ShieldDay) -> i64| week.iter().map(f).sum::<i64>();
    assert_eq!((sum(|d| d.https_upgrades), sum(|d| d.popups_blocked), sum(|d| d.local_blocked)), (1, 1, 1));
    visit(&mut shell, "https://a.test/2", "A");
    shell.event(key, "shielded", &json!({ "kind": "https_upgrade", "url": "https://a.test/" }), 3);
    assert_eq!(shell.store().shield_week(id).unwrap().iter().map(|d| d.https_upgrades).sum::<i64>(), 2);
}

#[test]
fn the_shield_chip_counts_what_shield_stopped_on_the_open_site() {
    let (_dir, mut shell) = open();
    assert_eq!(shell.site_shield_total(), 0, "no page, no count");
    visit(&mut shell, "https://a.test/", "A");
    let key = active(&shell);
    shell.event(key, "shielded", &json!({ "kind": "https_upgrade", "url": "https://a.test/" }), 3);
    shell.event(key, "shielded", &json!({ "kind": "local_network", "url": "https://a.test/" }), 3);
    assert_eq!(shell.site_shield_total(), 2);
    visit(&mut shell, "https://b.test/", "B");
    assert_eq!(shell.site_shield_total(), 0, "another site has its own count");
}

#[test]
fn a_burner_tab_has_its_own_store_keeps_no_records_and_is_released_when_burned() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let normal = shell.views()[0].profile.clone();
    shell.open_burner();
    assert!(shell.in_burner());
    let burner = shell.views()[0].profile.clone();
    assert!(burner.starts_with("burner:"), "{burner}");
    shell.enter_private();
    let private = shell.views()[0].profile.clone();
    assert!(!shell.in_burner(), "leaving the burner burns it");
    assert!(shell.core.profiles.borrow().values().all(|rt| !burner.ends_with(rt.data.store_id().trim_start_matches("private:"))), "its store is released");
    assert_ne!(burner.trim_start_matches("burner:"), private.trim_start_matches("private:"));
    shell.close_private();

    shell.open_burner();
    let second = shell.views()[0].profile.clone();
    assert_ne!(second, burner, "every burner gets a new store");
    visit(&mut shell, "https://burned.test/", "Burned");
    shell.event(active(&shell), "download", &json!({ "id": "1", "state": "finished", "name": "x.txt", "path": "/x" }), 3);
    shell.close_tab(active(&shell));
    assert!(!shell.in_burner());
    assert_eq!(shell.views()[0].profile, normal, "closing the burner tab returns to the space it came from");
    assert!(shell.history().iter().all(|h| h.url != "https://burned.test/"));
    assert!(shell.downloads().is_empty());
    assert!(shell.spaces().iter().all(|s| !s.burner));
}

#[test]
fn a_burner_keeps_one_tab_and_leaves_nothing_to_reopen() {
    let (_dir, mut shell) = open();
    shell.open_burner();
    visit(&mut shell, "https://burned.test/", "Burned");
    let key = active(&shell);
    assert_eq!(shell.new_tab(Some("https://next.test/")), Some(key), "new tabs load in the burner's own tab");
    assert_eq!(shell.tab_count(), 1);
    shell.close_duplicates();
    shell.close_private();
    assert!(!shell.reopen_closed(), "nothing of the burner is left to reopen");
}

#[test]
fn downloads_outlive_their_tab_and_reach_history() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let key = active(&shell);
    shell.event(key, "download", &json!({ "id": "1", "state": "started", "name": "a.zip", "pausable": true }), 3);
    shell.event(key, "download", &json!({ "id": "1", "state": "paused", "done": 5 }), 3);
    let other = shell.new_tab(None).unwrap();
    shell.close_tab(key);
    assert_eq!(shell.transfers.len(), 1, "closing the tab keeps its download");
    let resume = shell.transfer_command("1", "downloadResume", false);
    assert_eq!(resume, vec![Effect::Command(other, "downloadResume", json!({ "id": "1" }))], "commands go through a view still shown");
    assert!(shell.new_profile("Other"));
    shell.event(key, "download", &json!({ "id": "1", "state": "finished", "name": "a.zip", "path": "/a.zip" }), 4);
    assert!(shell.transfers.is_empty());
    assert!(shell.notice.as_deref().is_some_and(|n| n.contains("a.zip")));
    assert!(shell.downloads().is_empty(), "not kept in the profile shown when it ended");
    shell.choose_space(0, 5);
    assert_eq!(shell.downloads().iter().map(|d| d.url.as_str()).collect::<Vec<_>>(), ["a.zip"], "kept in the profile it started in");
    let unknown = TabKey { private: false, id: 9_999 };
    shell.event(unknown, "download", &json!({ "id": "2", "state": "failed", "name": "b.zip", "message": "gone" }), 6);
    assert_eq!(shell.downloads().len(), 2, "an event from a view the shell no longer knows is still kept");
}

#[test]
fn tabs_move_between_spaces_of_one_profile() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let key = active(&shell);
    shell.new_space("Work");
    let work = shell.model.space_ix;
    shell.choose_space(0, 1);
    let targets = shell.move_targets(key);
    assert_eq!(targets, vec![(work, "Work".to_owned())]);
    assert!(shell.move_tab(key, work));
    shell.choose_space(work, 2);
    assert!(shell.model.tabs.iter().any(|t| t.url == "https://a.test/"));
}

#[test]
fn schedules_switch_spaces_unless_switched_by_hand_within_the_hour() {
    let (_dir, mut shell) = open();
    shell.new_space("Work");
    let work = shell.model.space_ix;
    assert!(shell.set_schedule(work, Schedule { days: 0b011_1110, start: 9, end: 17 }));
    assert_eq!(shell.schedule(work).start, 9);
    shell.choose_space(0, 1_000);
    assert!(!shell.follow_schedule(1, 10, 2_000), "held after a manual switch");
    assert!(shell.follow_schedule(1, 10, 1_000 + browser_core::schedule::MANUAL_HOLD));
    assert_eq!(shell.model.space_ix, work);
    shell.enter_private();
    shell.choose_space(0, 0);
    shell.enter_private();
    assert!(!shell.follow_schedule(1, 10, i64::MAX / 2), "never leaves the private space");
}

#[test]
fn ruled_hosts_open_link_popups_in_their_space() {
    let (_dir, mut shell) = open();
    shell.new_space("Work");
    let work = shell.model.space_ix;
    shell.choose_space(0, 1);
    visit(&mut shell, "https://docs.test/", "Docs");
    assert!(shell.set_site_rule(Some(work)));
    assert_eq!(shell.site_rule().unwrap().space, Some(work));
    shell.new_tab(None);
    visit(&mut shell, "https://a.test/", "A");
    shell.event(active(&shell), "popup", &json!({ "url": "https://docs.test/script" }), 3);
    assert_eq!(shell.model.space_ix, 0, "a script's pop-up keeps its opener's space");
    assert_eq!(shell.views().iter().filter_map(|v| v.load.clone()).next_back().unwrap().opener.as_deref(), Some("https://a.test/"));
    shell.event(active(&shell), "committed", &json!({ "url": "https://docs.test/script", "title": "" }), 3);
    shell.event(active(&shell), "popup", &json!({ "url": "https://docs.test/x", "link": true }), 3);
    assert_eq!(shell.model.space_ix, work);
    let load = shell.views().iter().filter_map(|v| v.load.clone()).find(|l| l.url == "https://docs.test/x").unwrap();
    assert_eq!(load.opener, None, "a routed link opens as a fresh tab");
}

#[test]
fn snapshots_save_and_restore_a_space() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    assert!(shell.save_snapshot("Trip", 5));
    let saved = shell.snapshots();
    assert_eq!(saved.len(), 1);
    assert!(shell.restore_snapshot(&saved[0]));
    assert_eq!(shell.model.current_space().unwrap().space.name, "Trip");
    assert_eq!(shell.model.tabs.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), ["https://a.test/"]);
    shell.delete_snapshot(&saved[0]);
    assert!(shell.snapshots().is_empty());
}

#[test]
fn reading_list_keeps_pages_in_its_folder() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/story", "Story");
    assert!(shell.save_for_later(5));
    visit(&mut shell, "https://a.test/two", "Two");
    assert!(shell.save_for_later(6));
    let list = shell.reading_list();
    assert_eq!(list.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["Story", "Two"]);
    shell.remove_from_reading_list(list[0].id);
    assert_eq!(shell.reading_list().len(), 1);
}

#[test]
fn downloads_report_progress_pause_and_flagged_files() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let key = active(&shell);
    shell.event(key, "download", &json!({ "id": "7", "state": "started", "name": "a.zip", "pausable": true }), 3);
    shell.event(key, "download", &json!({ "id": "7", "state": "progress", "done": 10, "total": 100 }), 3);
    assert_eq!((shell.transfers[0].done, shell.transfers[0].total), (10, Some(100)));
    let pause = shell.transfer_command("7", "downloadPause", false);
    assert_eq!(command(&pause, "downloadPause"), Some(&json!({ "id": "7" })));
    shell.event(key, "download", &json!({ "id": "7", "state": "paused", "done": 12 }), 3);
    assert_eq!(shell.transfers[0].state, TransferState::Paused);
    shell.event(key, "download", &json!({ "id": "7", "state": "resumed" }), 3);
    assert_eq!(shell.transfers[0].state, TransferState::Saving);
    shell.event(key, "download", &json!({ "id": "7", "state": "flagged", "name": "a.zip", "host": "bad.test" }), 3);
    assert_eq!(shell.transfers[0].state, TransferState::Flagged("bad.test".into()));
    let keep = shell.transfer_command("7", "downloadKeep", false);
    assert_eq!(command(&keep, "downloadKeep"), Some(&json!({ "id": "7", "keep": false })));
    assert!(shell.transfers.is_empty());
}

#[test]
fn the_qr_code_and_share_take_only_web_pages() {
    let (_dir, mut shell) = open();
    assert!(shell.qr_code().is_none());
    visit(&mut shell, "https://a.test/page", "A");
    let (code, host) = shell.qr_code().unwrap();
    assert_eq!(host, "a.test");
    assert!(code.len() > 21);
}

#[test]
fn a_new_tab_reusing_a_closed_tabs_id_gets_its_own_view() {
    let (_dir, mut shell) = open();
    visit(&mut shell, "https://a.test/", "A");
    let closed = active(&shell);
    let born = shell.views().iter().find(|v| v.key == closed).map(|v| v.born);
    shell.close_tab(closed);
    let reused = active(&shell);
    assert_eq!(reused, closed, "the store hands the freed id to the next tab");
    let now = shell.views().iter().find(|v| v.key == reused).map(|v| v.born);
    assert!(born.is_some() && now.is_some() && born != now, "a fresh web view, not the closed tab's page");
}
