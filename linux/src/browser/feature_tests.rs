use super::*;
use crate::browser::pages::HISTORY_TEXT;
use crate::browser::spaces::SpaceCommand;
use browser_core::space_card::SpaceDraft;

fn browser() -> (tempfile::TempDir, Browser) {
    let dir = tempfile::tempdir().unwrap();
    let b = Browser::open(dir.path());
    (dir, b)
}

fn visit(b: &mut Browser, key: TabKey, url: &str) {
    b.on_event(key, "committed", &json!({ "url": url, "title": "T" }));
    b.on_event(key, "finished", &json!({ "url": url, "title": "T" }));
}

fn blocked_page(b: &Browser) -> Option<crate::browser::blocking::BlockedPage> {
    b.active_page().and_then(|p| p.blocked.clone())
}

fn asked(b: &Browser, name: &str) -> Vec<Value> {
    b.outbox.iter().filter(|(_, n, _)| *n == name).map(|(_, _, a)| a.clone()).collect()
}

#[test]
fn duplicates_close_with_one_undo_that_reopens_them() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    visit(&mut b, first, "https://a.test/x");
    let second = b.new_tab("https://a.test/x#top").unwrap();
    visit(&mut b, second, "https://a.test/x#top");
    let third = b.new_tab("https://a.test/x").unwrap();
    visit(&mut b, third, "https://a.test/x");
    assert_eq!(b.duplicate_tabs().len(), 2);
    b.close_duplicates();
    assert_eq!(b.model.tabs.len(), 1);
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("2 tabs closed"));
    b.take_toast_action();
    assert_eq!(b.model.tabs.len(), 3);
    assert!(b.toast.is_none());
}

#[test]
fn closing_the_active_tab_selects_the_neighbour_the_setting_names() {
    let (_dir, mut b) = browser();
    let tabs: Vec<TabKey> = (0..4).map(|n| b.new_tab(&format!("https://t{n}.test/")).unwrap()).collect();
    let middle = |b: &Browser| {
        b.model
            .tabs
            .iter()
            .map(|t| b.model.key(t))
            .find(|k| tabs.contains(k) && b.model.close_neighbour(*k, &[], false) != b.model.close_neighbour(*k, &[], true))
    };
    let mid = middle(&b).unwrap();
    let next = b.model.close_neighbour(mid, &[], false);
    b.activate(mid);
    b.close_tab(mid);
    assert_eq!(b.model.active, next, "Next by default");
    b.set_pref(browser_core::prefs::CLOSE_SELECTS_PREVIOUS, true);
    let mid = middle(&b).unwrap();
    let previous = b.model.close_neighbour(mid, &[], true);
    b.activate(mid);
    b.close_tab(mid);
    assert_eq!(b.model.active, previous);
}

#[test]
fn the_command_card_opens_what_was_typed_in_a_new_tab() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let count = b.model.tabs.len();
    b.navigate_in("https://first.test/", true);
    assert_eq!((b.model.active, b.model.tabs.len()), (Some(first), count), "a blank tab takes it");
    visit(&mut b, first, "https://kept.test/");
    b.navigate_in("https://new.test/", true);
    let opened = b.model.active.unwrap();
    assert_ne!(opened, first);
    assert_eq!(b.model.tabs.len(), count + 1);
    assert_eq!(b.pages[&first].url, "https://kept.test/", "the page it was opened over keeps its address");
    assert_eq!(b.pages[&opened].load.as_ref().map(|(url, _)| url.as_str()), Some("https://new.test/"));
}

#[test]
fn a_download_outlives_its_tab_and_is_cancelled_by_id() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    b.new_tab("https://b.test/");
    let path = "/home/u/Downloads/big.zip";
    b.on_download(key, &json!({ "id": "7", "state": "started", "name": "big.zip", "path": path }), true);
    b.close_tab(key);
    b.on_download(key, &json!({ "id": "7", "state": "progress", "done": 10, "total": 20 }), true);
    assert_eq!(b.downloads[0].done, 10);
    b.on_download(key, &json!({ "id": "7", "state": "finished", "name": "big.zip", "path": path, "url": "https://d.test/big.zip" }), true);
    assert_eq!(b.downloads[0].state, "finished");
    assert_eq!(b.core.disk.downloads(profile).unwrap()[0].path, path);
    assert!(b.download_owners.is_empty());
    b.cancel_download("8");
    assert_eq!(b.cancelled_downloads, ["8"]);
}

#[test]
fn a_flagged_download_waits_for_keep_or_delete() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    for id in ["4", "5"] {
        b.on_download(key, &json!({ "id": id, "state": "started", "name": "x.exe", "pausable": false }), true);
        b.on_download(key, &json!({ "id": id, "state": "flagged", "name": "x.exe", "host": "evil.test" }), true);
    }
    assert_eq!(
        b.toast.as_ref().map(|t| (t.tone, t.text.as_str())),
        Some((browser_core::toast::Tone::Warning, "x.exe comes from evil.test, a site on the malware list."))
    );
    assert!(b.downloads.iter().all(|d| d.state == "flagged" && d.message == "From evil.test, a site on the malware list. Not saved until you keep it."));
    assert_eq!(crate::snapshot::snapshot(&b, true).downloading, None, "a held download is not counted as saving");
    b.clear_downloads();
    assert_eq!(b.downloads.len(), 2, "Clear keeps downloads that wait for an answer");
    b.keep_download("4", true);
    assert!(b.kept_downloads.is_empty(), "Keep ignores clicks while the rows still move");
    b.downloads_moved -= browser_core::prompts::BYPASS_DELAY;
    b.on_download(key, &json!({ "id": "4", "state": "progress", "done": 0, "total": 9 }), true);
    b.keep_download("4", true);
    assert!(b.kept_downloads.is_empty(), "a progress bar appearing moves the rows");
    b.downloads_moved -= browser_core::prompts::BYPASS_DELAY;
    b.keep_download("4", true);
    assert_eq!(b.downloads.iter().find(|d| d.id == "4").map(|d| d.state.as_str()), Some("started"), "a kept row loses Keep and Delete at once");
    assert!(!browser_core::prompts::stable(b.downloads_moved, std::time::Instant::now()), "a kept row moves the rest");
    b.keep_download("5", false);
    assert_eq!(b.kept_downloads, [("4".to_owned(), true), ("5".to_owned(), false)]);
    assert_eq!(b.downloads.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["4"]);
    assert!(!b.download_owners.contains_key("5"));
    b.on_download(key, &json!({ "id": "4", "state": "started", "name": "x.exe", "path": "/d/x.exe", "pausable": false }), true);
    assert_eq!(b.downloads[0].state, "started");
    b.on_download(key, &json!({ "id": "4", "state": "finished", "name": "x.exe", "path": "/d/x.exe", "url": "https://evil.test/x.exe" }), true);
    assert_eq!(b.core.disk.downloads(profile).unwrap().len(), 1, "only the kept file is recorded");
}

#[test]
fn a_burned_tab_keeps_its_download_running_and_records_nothing() {
    let (_dir, mut b) = browser();
    b.open_burner();
    let key = b.model.active.unwrap();
    let burner_profile = b.model.profile().unwrap().1;
    b.favicons.load(&(true, burner_profile), || (vec![("a.test".to_owned(), std::sync::Arc::from("icon"))], Vec::new()));
    b.on_download(key, &json!({ "id": "9", "state": "started", "name": "a.zip", "path": "/d/a.zip" }), true);
    b.close_tab(key);
    assert!(!b.model.burner_shown());
    assert_eq!(b.released.len(), 1);
    assert!(b.favicons.icons(&(true, burner_profile)).next().is_none(), "burning forgets the burner's icons");
    b.on_download(key, &json!({ "id": "9", "state": "finished", "name": "a.zip", "path": "/d/a.zip", "url": "https://d.test/a.zip" }), true);
    assert_eq!(b.downloads[0].state, "finished");
    assert!(b.core.store(true).downloads(burner_profile).unwrap().is_empty());
    assert!(b.download_owners.is_empty());
}

#[test]
fn a_burner_tab_has_its_own_store_keeps_nothing_and_burns_on_close() {
    let (_dir, mut b) = browser();
    let home = b.model.space_ix;
    let profile = b.model.profile().unwrap().1;
    b.open_burner();
    assert!(b.model.burner_shown());
    let key = b.model.active.unwrap();
    let prop = b.store_prop(key);
    assert!(prop.starts_with("burner:") && prop.len() == 7 + 32, "{prop}");
    assert!(crate::data::StoreKey::parse(&prop).is_some());
    visit(&mut b, key, "https://burn.test/");
    let burner_profile = b.model.profile().unwrap().1;
    assert!(b.core.store(true).history(burner_profile, 10).unwrap().is_empty());
    b.close_tab(key);
    assert_eq!(b.model.space_ix, home);
    assert!(!b.model.burner_shown());
    assert_eq!(b.released.len(), 1);
    assert!(b.model.closed_tabs.newest().is_none(), "a burner tab never goes on the reopen stack");
    assert_eq!(b.model.profile().unwrap().1, profile);
}

#[test]
fn the_reader_opens_from_an_answer_for_the_page_on_screen() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://news.test/a");
    let probe = asked(&b, "readArticle");
    assert_eq!(probe.len(), 1);
    assert_eq!(probe[0]["probe"], true);
    let seq = probe[0]["seq"].as_u64().unwrap();
    b.on_event(key, "article", &json!({ "seq": seq, "url": "https://elsewhere.test/", "text": "1" }));
    assert!(b.articles.is_empty(), "an answer for another page is dropped");
    b.on_event(key, "article", &json!({ "seq": seq, "url": "https://news.test/a", "text": "1" }));
    assert!(b.articles.is_empty(), "each sequence number answers once");
    b.outbox.clear();
    b.toggle_reader();
    let read = asked(&b, "readArticle");
    let seq = read[0]["seq"].as_u64().unwrap();
    let article = json!({ "t": "Headline", "b": "By A", "p": [["h", "Part"], ["p", "Body text."]] }).to_string();
    b.on_event(key, "article", &json!({ "seq": seq, "url": "https://news.test/a", "text": article }));
    let reader = b.reader.clone().unwrap();
    assert_eq!((reader.title.as_str(), reader.blocks.len()), ("Headline", 2));
    assert!(crate::snapshot::snapshot(&b, true).views.iter().all(|v| !v.active || b.reader.is_some()));
    b.on_event(key, "committed", &json!({ "url": "https://news.test/b" }));
    assert!(b.reader.is_none(), "a new page closes the reader");
    b.toggle_reader();
    let seq = asked(&b, "readArticle").last().unwrap()["seq"].as_u64().unwrap();
    b.on_event(key, "article", &json!({ "seq": seq, "url": "https://news.test/b", "text": "" }));
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("No article found on this page"));
}

#[test]
fn page_text_is_kept_only_when_switched_on_and_never_in_private() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://text.test/");
    assert!(asked(&b, "readText").is_empty(), "off by default");
    b.toggle_off_setting(HISTORY_TEXT);
    b.outbox.clear();
    visit(&mut b, key, "https://text.test/");
    let ask = asked(&b, "readText");
    assert_eq!(ask[0]["maxChars"], store::MAX_HISTORY_TEXT);
    let seq = ask[0]["seq"].as_u64().unwrap();
    b.on_event(key, "pageText", &json!({ "seq": seq, "url": "https://text.test/", "text": "the quick zebrafish" }));
    let profile = b.model.profile().unwrap().1;
    assert_eq!(b.core.disk.search_history_text(profile, "zebra", 5).unwrap().len(), 1);
    assert_eq!(b.cached_text_matches("ze"), Some(Vec::new()), "under three characters there is no text search");
    assert_eq!(b.cached_text_matches("zebra"), None, "not searched yet, so the field waits");
    assert_eq!(b.search_text_matches("zebra", 6).len(), 1);
    assert_eq!(b.cached_text_matches("zebra").map(|f| f.len()), Some(1), "the matches are kept for the query");
    b.toggle_off_setting(HISTORY_TEXT);
    assert!(b.core.disk.search_history_text(profile, "zebra", 5).unwrap().is_empty(), "turning it off forgets the text");
    assert_eq!(b.cached_text_matches("zebra"), Some(Vec::new()));
    b.toggle_off_setting(HISTORY_TEXT);
    b.private_space();
    let private = b.model.active.unwrap();
    b.outbox.clear();
    visit(&mut b, private, "https://text.test/");
    assert!(asked(&b, "readText").is_empty());
}

#[test]
fn find_in_tabs_lists_text_matches_above_title_matches() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    visit(&mut b, first, "https://alpha.test/");
    let second = b.new_tab("https://beta.test/").unwrap();
    visit(&mut b, second, "https://beta.test/");
    b.outbox.clear();
    b.find_in_tabs("alpha");
    assert_eq!(b.find_rows.len(), 1);
    assert_eq!(b.find_rows[0].detail, "Title or address");
    let asks: Vec<(TabKey, Value)> = b.outbox.iter().filter(|(_, n, _)| *n == "findText").map(|(k, _, a)| (*k, a.clone())).collect();
    assert_eq!(asks.len(), 2);
    let (key, args) = asks.iter().find(|(k, _)| *k == second).unwrap();
    b.on_event(*key, "textFound", &json!({ "seq": args["seq"], "url": "https://beta.test/", "snippets": ["about alpha here"] }));
    assert_eq!(b.find_rows.len(), 2);
    assert_eq!(b.find_rows[0].label, "about alpha here");
    assert_eq!(b.find_rows[0].key, second);
    assert_eq!(b.open_find_row(second), "alpha");
    assert_eq!(b.model.active, Some(second));
}

#[test]
fn shield_counts_each_kind_once_per_page_and_skips_local_popups() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    b.on_event(key, "committed", &json!({ "url": "https://c.test/" }));
    for _ in 0..3 {
        b.on_event(key, "shielded", &json!({ "kind": "https_upgrade", "url": "https://c.test/" }));
    }
    b.on_event(key, "shielded", &json!({ "kind": "popup", "url": "http://192.168.1.1/" }));
    b.on_event(key, "shielded", &json!({ "kind": "local_network", "url": "http://10.0.0.1/" }));
    let stats = b.core.disk.shield_stats(profile).unwrap();
    assert_eq!((stats.https_upgrades, stats.local_blocked), (1, 1));
    let day = b.core.disk.shield_week(profile).unwrap().pop().unwrap();
    assert_eq!(day.popups_blocked, 0);
    b.on_event(key, "committed", &json!({ "url": "https://c.test/next" }));
    b.on_event(key, "shielded", &json!({ "kind": "popup", "url": "https://ads.test/" }));
    b.on_event(key, "shielded", &json!({ "kind": "https_upgrade", "url": "https://c.test/" }));
    assert_eq!(b.core.disk.shield_stats(profile).unwrap().https_upgrades, 2);
    assert_eq!(b.core.disk.shield_week(profile).unwrap().pop().unwrap().popups_blocked, 1);
}

#[test]
fn picked_elements_are_hidden_per_host_outside_private_spaces() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://zap.test/page");
    b.outbox.clear();
    b.zap_element();
    let seq = asked(&b, "pick")[0]["seq"].as_u64().unwrap();
    b.on_event(key, "picked", &json!({ "seq": seq, "url": "https://zap.test/page", "selector": "div.ad > p:nth-of-type(2)" }));
    assert_eq!(b.zapped(Some("zap.test")), [("zap.test".to_owned(), "div.ad > p:nth-of-type(2)".to_owned())]);
    assert_eq!(b.zaps(key)["zap.test"][0], "div.ad > p:nth-of-type(2)");
    b.zap_element();
    let seq = asked(&b, "pick").last().unwrap()["seq"].as_u64().unwrap();
    b.on_event(key, "picked", &json!({ "seq": seq, "url": "https://zap.test/page", "selector": "</style>" }));
    assert_eq!(b.zapped(None).len(), 1, "only the picker's shape is stored");
    b.unzap("zap.test", None);
    assert!(b.zapped(None).is_empty());
    b.private_space();
    let private = b.model.active.unwrap();
    assert_eq!(b.zaps(private), json!({}));
    assert!(!b.zappable());
}

#[test]
fn site_styles_carry_dark_mode_and_css_and_reload_the_host() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://style.test/");
    b.outbox.clear();
    b.toggle_site_dark();
    assert!(b.site_dark());
    assert_eq!(asked(&b, "reload").len(), 1);
    b.save_site_css("p { margin: 0 }").unwrap();
    let css = b.styles(key)["style.test"].as_str().unwrap().to_owned();
    assert!(css.starts_with(engine::page_scripts::DARK_CSS) && css.ends_with("p { margin: 0 }"));
    assert!(b.save_site_css(&"x".repeat(store::MAX_SITE_CSS + 1)).unwrap_err().contains("KiB"));
    assert_eq!(b.site_css(), "p { margin: 0 }");
}

#[test]
fn ruled_hosts_open_in_their_space_and_spaces_switch_on_schedule() {
    let (_dir, mut b) = browser();
    let home = b.model.space_ix;
    let home_id = b.model.current_space().unwrap().space.id;
    b.save_space(&b.new_space_draft());
    let work = b.model.space_ix;
    b.switch_space_by_user(home);
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://docs.test/");
    let work_id = b.model.spaces[work].space.id;
    b.route_site("docs.test", Some(work_id));
    assert_eq!(b.route_choices("docs.test").1, Some(work_id));
    b.on_event(key, "popup", &json!({ "url": "https://docs.test/signin", "link": false }));
    assert_eq!(b.model.space_ix, home, "a script pop-up keeps its opener");
    b.on_event(key, "popup", &json!({ "url": "https://docs.test/new", "link": true }));
    assert_eq!(b.model.space_ix, work);
    assert!(b.pages.values().any(|p| p.load.as_ref().is_some_and(|l| l.0 == "https://docs.test/new")));
    assert_eq!(b.space_rule_rows().len(), 1);
    b.switch_space_by_user(home);
    let before = b.model.tabs.len();
    b.new_tab(START_URL);
    b.navigate_in("https://docs.test/typed", false);
    assert_eq!(b.model.space_ix, work);
    b.switch_space_by_user(home);
    assert_eq!(b.model.tabs.len(), before, "the blank tab that was routed away is gone");
    b.switch_space_by_user(home);
    b.core.disk.set_space_schedule(work_id, store::Schedule { days: 0x7f, start: 0, end: 0 }).unwrap();
    b.model.reload_spaces(&b.core);
    b.schedule_tick();
    assert_eq!(b.model.space_ix, home, "a manual switch holds for an hour");
    b.switched_at = None;
    b.schedule_tick();
    assert_eq!(b.model.current_space().unwrap().space.id, work_id);
    assert_ne!(home_id, work_id);
}

#[test]
fn snapshots_restore_into_a_new_space_and_commands_switch_and_move() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://one.test/");
    b.new_tab("https://two.test/");
    b.save_snapshot();
    let saved = b.snapshots();
    assert_eq!(saved.len(), 1);
    let spaces = b.model.spaces.len();
    b.restore_snapshot(&saved[0], true);
    assert_eq!(b.model.spaces.len(), spaces + 1);
    assert_eq!(b.model.tabs.len(), 2);
    let commands = b.space_commands();
    let switch = commands.iter().find(|(l, _)| l == "Switch to Home").map(|(_, c)| *c).unwrap();
    assert!(commands.iter().any(|(l, c)| l == "Move Tab to Home" && matches!(c, SpaceCommand::Move(..))));
    b.run_space_command(switch);
    assert_eq!(b.model.current_space().unwrap().space.name, "Home");
}

#[test]
fn clean_links_drop_tracking_parameters_and_pages_go_to_the_reading_list() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://shop.test/item?id=4&utm_source=x&utm_medium=y");
    assert_eq!(b.copy_link(true).as_deref(), Some("https://shop.test/item?id=4"));
    assert_eq!(b.toast.as_ref().unwrap().text, "Link copied, 2 tracking parameters removed");
    assert_eq!(b.toast.as_ref().unwrap().tone, browser_core::toast::Tone::Success);
    let profile = b.model.profile().unwrap().1;
    assert_eq!(b.core.disk.shield_stats(profile).unwrap().tracking_params, 2);
    assert!(b.qr_code().is_some_and(|(code, host)| !code.is_empty() && host == "shop.test"));
    b.save_for_later();
    b.save_for_later();
    assert_eq!(b.folder_pages(crate::browser::tools::READING_LIST).len(), 1);
    assert_eq!(b.toast.as_ref().unwrap().text, "Saved to Reading List");
    let id = b.folder_pages(crate::browser::tools::READING_LIST)[0].id;
    b.remove_bookmark(id);
    assert!(b.folder_pages(crate::browser::tools::READING_LIST).is_empty());
}

#[test]
fn a_split_shows_its_panes_and_a_closed_pane_hands_the_focus_on() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    visit(&mut b, first, "https://a.test/");
    let second = b.new_tab("https://b.test/").unwrap();
    let third = b.new_tab("https://c.test/").unwrap();
    b.activate(first);
    b.open_split();
    assert_eq!(b.shown_split().map(|s| s.panes().to_vec()), Some(vec![first, third]), "the most recently used tab joins");
    assert_eq!(b.model.active, Some(third));
    b.split_with(second);
    let snap = crate::snapshot::snapshot(&b, true);
    let shown: Vec<_> = snap.views.iter().filter(|v| v.active).map(|v| (v.key, v.pane)).collect();
    assert_eq!(shown.len(), 3);
    assert!(shown.contains(&(first, Some(0))) && shown.contains(&(second, Some(2))));
    assert_eq!(snap.split.as_ref().map(|s| (s.panes.len(), s.dividers.len())), Some((3, 2)));
    let rows = crate::snapshot::rows(&b);
    assert_eq!(rows.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), [crate::snapshot::RowKind::Split(first)], "a split lists as one row");
    assert_eq!(rows[0].panes.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), [first, third, second].map(crate::snapshot::RowKind::Tab), "in layout order");
    assert!(rows[0].panes.iter().all(|r| r.split == Some(true)));
    assert!(crate::snapshot::snapshot(&b, false).split.is_none(), "a panel covers the split");

    b.close_tab(second);
    assert_eq!(b.model.active, Some(third), "the neighbouring pane takes the focus");
    b.on_event(first, "pressed", &Value::Null);
    assert_eq!(b.model.active, Some(first));
    b.swap_panes();
    assert_eq!(b.shown_split().unwrap().panes(), [third, first]);
    b.leave_split(first);
    assert!(b.shown_split().is_none() && b.model.active == Some(first), "a split left with one pane ends");
}

#[test]
fn a_split_out_of_sight_waits_and_its_edge_fades() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let second = b.new_tab("https://b.test/").unwrap();
    b.open_split();
    let other = b.new_tab("https://c.test/").unwrap();
    assert!(b.shown_split().is_none());
    let rows = crate::snapshot::rows(&b);
    let edge = |key| rows.iter().flat_map(|r| [r].into_iter().chain(&r.panes)).find(|r| r.kind == crate::snapshot::RowKind::Tab(key)).map(|r| r.split);
    assert_eq!((edge(first), edge(second), edge(other)), (Some(Some(false)), Some(Some(false)), Some(None)));
    b.on_event(first, "pressed", &Value::Null);
    assert_eq!(b.model.active, Some(other), "a hidden pane takes no clicks");
    b.activate(first);
    assert_eq!(b.shown_split().map(|s| s.panes().to_vec()), Some(vec![second, first]));
    b.toggle_split();
    assert!(b.shown_split().is_none() && b.splits.of(second).is_none());
}

#[test]
fn a_lone_tab_splits_with_a_copy_but_the_burner_says_it_needs_another() {
    let (_dir, mut b) = browser();
    let only = b.model.active.unwrap();
    visit(&mut b, only, "https://a.test/");
    b.open_split();
    let copy = b.model.active.unwrap();
    assert_ne!(copy, only);
    assert_eq!(b.model.tab(copy).map(|t| t.url.as_str()), Some("https://a.test/"));
    b.open_burner();
    b.open_split();
    assert!(b.shown_split().is_none());
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("Split View needs a second tab"));
}

#[test]
fn a_tab_dropped_on_a_grouped_row_joins_its_group_and_other_windows_reload() {
    let (_dir, mut b) = browser();
    let a = b.model.active.unwrap();
    let c = b.new_tab("https://c.test/").unwrap();
    let d = b.new_tab("https://d.test/").unwrap();
    let id = b.new_group(Some(a)).unwrap();
    let here = b.model.current_space().map(|s| (s.private, s.space.id));
    b.regrouped_space = None;
    b.move_tab_to(c, a, false);
    assert_eq!(b.model.tab(c).unwrap().group_id, Some(id));
    assert_eq!(b.regrouped_space, here, "other windows on this space reload its groups");
    b.regrouped_space = None;
    b.move_tab_to(a, c, false);
    assert_eq!(b.regrouped_space, None, "a reorder inside the group changes no group");
    b.move_tab_to(c, d, false);
    assert_eq!((b.model.tab(c).unwrap().group_id, b.regrouped_space), (None, here), "leaving the group reloads too");
    b.new_folder(None);
    let folder = b.model.folders[0].id;
    b.regrouped_space = None;
    b.move_to_folder(d, folder);
    assert_eq!(b.regrouped_space, None, "filing an ungrouped tab changes no group");
    b.move_to_folder(a, folder);
    assert_eq!((b.model.tab(a).unwrap().group_id, b.regrouped_space), (None, here), "filing a grouped tab takes it out of its group and reloads");
}

#[test]
fn a_dragged_tab_takes_the_place_of_the_row_it_lands_on() {
    let (_dir, mut b) = browser();
    let a = b.model.active.unwrap();
    let c = b.new_tab("https://c.test/").unwrap();
    let d = b.new_tab("https://d.test/").unwrap();
    let order = |b: &Browser| b.model.ephemeral_keys();
    assert_eq!(order(&b), [a, c, d]);
    b.move_tab_to(d, a, false);
    assert_eq!(order(&b), [d, a, c], "the upper half of a row puts it above");
    b.move_tab_to(d, c, true);
    assert_eq!(order(&b), [a, c, d], "the lower half puts it below");
    b.move_tab_to(d, a, true);
    assert_eq!(order(&b), [a, d, c], "below a row above it");
    b.move_tab_to(a, c, false);
    assert_eq!(order(&b), [d, a, c], "above a row below it");
    b.pin_toggle(a);
    b.move_tab_to(c, a, false);
    assert_eq!(b.model.tab(c).unwrap().kind, TabKind::Pinned, "a row of another zone takes the tab into it");
    b.move_tab_to(d, d, true);
    assert_eq!(order(&b), [d]);
}

#[test]
fn the_space_card_creates_edits_and_adds_a_profile() {
    let (_dir, mut b) = browser();
    let home = b.model.space_ix;
    let mut draft = b.new_space_draft();
    draft.name = "Work".into();
    draft.color = "blue".into();
    draft.icon = "code".into();
    assert!(b.save_space(&draft));
    assert!(!b.save_space(&draft), "a taken name is refused");
    assert_ne!(b.model.space_ix, home, "the new space is shown");
    let work = b.model.current_space().unwrap().clone();
    assert_eq!((work.space.name.as_str(), work.space.color.as_str(), work.space.icon.as_str()), ("Work", "blue", "code"));
    b.switch_space_by_user(home);
    let mut edit = SpaceDraft::edit(&work);
    edit.name = "Code".into();
    edit.schedule.days = 0b011_1110;
    assert!(b.save_space(&edit));
    assert_eq!(b.model.space_ix, home, "an edit keeps the shown space");
    let saved = b.model.spaces.iter().find(|s| s.space.id == work.space.id).unwrap();
    assert_eq!((saved.space.name.as_str(), saved.space.schedule), ("Code", store::Schedule { days: 0b011_1110, start: 9, end: 17 }));
    let mut fresh = b.new_space_draft();
    fresh.profile = None;
    fresh.profile_name = "Second".into();
    fresh.profile_color = "green".into();
    assert!(b.save_space(&fresh));
    let second = b.core.disk.profiles().unwrap().into_iter().find(|p| p.name == "Second").unwrap();
    assert_eq!(second.color, "green");
    assert_eq!(b.model.current_space().unwrap().space.profile_id, second.id, "the new profile's space is shown");
}

#[test]
fn spaces_and_folders_are_renamed_and_folders_nest() {
    let (_dir, mut b) = browser();
    let mut draft = SpaceDraft::edit(b.model.current_space().unwrap());
    draft.name = "  Work  ".into();
    b.save_space(&draft);
    assert_eq!(b.model.current_space().unwrap().space.name, "Work");
    draft.name = " ".into();
    assert!(!b.save_space(&draft));
    assert_eq!(b.model.current_space().unwrap().space.name, "Work", "a blank name is ignored");
    b.new_folder(None);
    let parent = b.model.folders[0].id;
    b.new_folder(Some(parent));
    let child = b.model.folders.iter().find(|f| f.id != parent).unwrap();
    assert_eq!(child.parent_id, Some(parent));
    assert!(b.model.expanded.contains(&parent));
    b.rename_folder(parent, "Reading");
    assert_eq!(b.model.folders.iter().find(|f| f.id == parent).unwrap().name, "Reading");
}

#[test]
fn a_typed_search_forgets_the_last_count_and_a_step_keeps_it() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.on_event(key, "found", &json!({ "matched": 0 }));
    b.find("x", false);
    assert_eq!(b.pages[&key].found, Some(0));
    b.find_typed("xy");
    assert_eq!(b.pages[&key].found, None);
}

#[test]
fn site_icons_load_at_start_fetch_inside_the_tab_and_go_with_history() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    b.core.disk.set_favicon(profile, "b.test", b"png").unwrap();
    let icons = b.site_icons(&Default::default()).unwrap();
    assert_eq!(icons.find("www.b.test").as_deref(), Some(engine::favicon::data_uri(b"png").as_str()));
    assert!(b.site_icons(&icons).is_none(), "unchanged");
    visit(&mut b, key, "https://b.test/");
    assert!(asked(&b, "readIconLinks").is_empty(), "a fresh stored icon is not fetched again");
    visit(&mut b, key, "http://a.test/");
    assert!(asked(&b, "readIconLinks").is_empty(), "only https");
    for _ in 0..engine::favicon::MAX_TRIES {
        b.outbox.clear();
        visit(&mut b, key, "https://a.test/x");
        let seq = asked(&b, "readIconLinks")[0]["seq"].as_u64().unwrap();
        b.on_event(key, "iconLinks", &json!({ "seq": seq, "url": "https://a.test/x", "text": "https://a.test/i.png\t32x32\nhttps://evil.test/p.png\t64x64" }));
        let fetch = asked(&b, "fetchIcon");
        assert_eq!(fetch[0]["urls"], json!(["https://a.test/i.png", "https://a.test/favicon.ico"]));
        assert_eq!(fetch[0]["page"], "https://a.test/x");
        b.on_icon_event(
            key,
            &json!({ "seq": fetch[0]["seq"], "url": "https://a.test/x", "text": "bm90IGFuIGltYWdl" }),
            engine::favicon::normalize("bm90IGFuIGltYWdl"),
        );
    }
    b.outbox.clear();
    visit(&mut b, key, "https://a.test/x");
    assert!(asked(&b, "readIconLinks").is_empty(), "gives up after MAX_TRIES failed fetches");
    assert!(b.core.disk.favicons(profile).unwrap().iter().all(|(h, _)| h == "b.test"));
    b.clear_history(browser_core::library::HistoryRange::All);
    let cleared = b.site_icons(&icons).unwrap();
    assert!(cleared.find("b.test").is_none());
    b.outbox.clear();
    visit(&mut b, key, "https://a.test/x");
    assert_eq!(asked(&b, "readIconLinks").len(), 1, "a history clear starts over");
    b.close_tab(key);
    let next = b.new_tab("https://a.test/x").unwrap();
    b.outbox.clear();
    visit(&mut b, next, "https://a.test/x");
    assert_eq!(asked(&b, "readIconLinks").len(), 1, "a closed tab's unanswered request does not hold the host");
}

#[test]
fn a_burner_tab_neither_fetches_nor_keeps_site_icons() {
    let (_dir, mut b) = browser();
    b.open_burner();
    let key = b.model.active.unwrap();
    let profile = (true, b.model.profile().unwrap().1);
    b.outbox.clear();
    visit(&mut b, key, "https://a.test/x");
    assert!(asked(&b, "readIconLinks").is_empty(), "a burner tab asks for no icons");
    let ticket = b.favicons.request(&profile, "a.test").unwrap();
    b.requests.insert(900, crate::browser::pages::Request::IconData(key, profile, "a.test".to_owned(), ticket));
    b.on_icon_event(key, &json!({ "seq": 900, "url": "https://a.test/x" }), Some(b"png".to_vec()));
    assert!(b.favicons.icons(&profile).next().is_none(), "a burner answer never enters memory");
    assert!(!b.favicons.is_requested(&profile, "a.test", ticket));
    assert!(b.site_icons(&Default::default()).is_none_or(|icons| icons.find("a.test").is_none()));
}

#[test]
fn the_suggestions_switch_turns_off_history_rows_completion_and_text_search() {
    let (_dir, mut b) = browser();
    let history = [store::HistoryEntry { url: "https://zebra.test/".into(), title: "Zebra".into(), visited_at: browser_core::now(), visits: 5, typed: 3 }];
    let (rows, completion) = b.address_edit("ze", "zeb", &history, 6);
    assert_eq!(rows, [("Zebra".to_owned(), "https://zebra.test/".to_owned())]);
    assert_eq!(completion.map(|c| c.suffix), Some("ra.test".to_owned()));
    b.toggle_off_setting(HISTORY_TEXT);
    assert_eq!(b.cached_text_matches("zebra"), None, "on by default, so the text is searched");
    b.toggle_setting(browser_core::settings::SUGGESTIONS);
    assert!(!b.suggestions());
    let (rows, completion) = b.address_edit("ze", "zeb", &history, 6);
    assert!(rows.is_empty() && completion.is_none());
    assert_eq!(b.cached_text_matches("zebra"), Some(Vec::new()));
    assert!(b.search_text_matches("zebra", 6).is_empty());
}

#[test]
fn search_shortcuts_are_off_until_switched_on_and_work_in_the_private_space() {
    let (_dir, mut b) = browser();
    let loaded = |b: &Browser| b.pages[&b.model.active.unwrap()].load.as_ref().map(|(url, _)| url.clone());
    assert!(!b.search_shortcuts());
    b.navigate_in("!yt cats", false);
    assert_eq!(loaded(&b).as_deref(), Some("https://duckduckgo.com/?q=%21yt+cats"));
    b.toggle_off_setting(browser_core::settings::SEARCH_SHORTCUTS);
    assert!(b.search_shortcuts());
    b.navigate_in("!yt cats", false);
    assert_eq!(loaded(&b).as_deref(), Some("https://www.youtube.com/results?search_query=cats"));
    b.private_space();
    b.navigate_in("cats !w", true);
    assert_eq!(loaded(&b).as_deref(), Some("https://en.wikipedia.org/w/index.php?search=cats"));
}

#[test]
fn the_tab_choices_set_the_live_limit_and_the_sleep_time() {
    use std::time::{Duration, Instant};
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let tabs: Vec<TabKey> = (0..5).map(|n| b.new_tab(&format!("https://t{n}.test/")).unwrap()).collect();
    b.on_event(tabs[0], "playing", &json!({ "playing": true }));
    let live = |b: &Browser, key| b.model.lifecycle.live().any(|k| k == key);
    let number = b.model.lifecycle.born(tabs[0]);
    b.set_live_tabs(0);
    b.apply_tab_choices(false, 1);
    assert_eq!(b.model.lifecycle.born(tabs[0]), number, "the playing tab keeps its view");
    assert_eq!(browser_core::settings::live_tabs(&b.core.disk), 3);
    assert!(!live(&b, first) && !live(&b, tabs[1]) && b.pages[&first].load.is_none(), "the least recent tabs sleep");
    assert!(live(&b, tabs[0]), "a playing tab stays awake");
    assert!(tabs[2..].iter().all(|k| live(&b, *k)));
    assert!(!b.set_sleep_after(0));
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(3 * 3_600), true);
    assert!(live(&b, tabs[2]), "Never keeps idle tabs awake, short of memory too");
    assert!(!b.set_sleep_after(9), "an unknown choice is ignored");
    assert!(b.set_sleep_after(1), "turning sleep on after Never counts idle times afresh");
    b.apply_tab_choices(true, 1);
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(9 * 60), false);
    assert!(live(&b, tabs[2]));
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(11 * 60), false);
    assert!(!live(&b, tabs[2]) && live(&b, tabs[4]), "after ten minutes, not the shown tab");
}

#[test]
fn background_tabs_over_the_memory_budget_sleep_heaviest_first() {
    use crate::memory::Usage;
    use std::time::{Duration, Instant};
    let (_dir, mut b) = browser();
    let pinned = b.model.active.unwrap();
    b.pin_toggle(pinned);
    let heavy = b.new_tab("https://heavy.test/").unwrap();
    let recent = b.new_tab("https://recent.test/").unwrap();
    let playing = b.new_tab("https://play.test/").unwrap();
    b.on_event(playing, "playing", &json!({ "playing": true }));
    let pane = b.new_tab("https://pane.test/").unwrap();
    let shown = b.new_tab("https://shown.test/").unwrap();
    b.split_with(pane);
    assert!(b.in_split(pane) && b.in_split(shown));
    let start = Instant::now();
    let sizes = [(pinned, 500), (heavy, 600), (recent, 300), (playing, 500), (pane, 5000), (shown, 100)];
    let usage = sizes.iter().enumerate().map(|(pid, (key, size))| {
        b.processes.insert(*key, pid as i32 + 100);
        (pid as i32 + 100, Usage { footprint: *size, cpu_ns: 0 })
    });
    let usage = usage.collect();
    b.samples.record(usage, start);
    let two_minutes_ago = start - Duration::from_secs(120);
    b.used.extend([pinned, heavy, playing, pane].map(|k| (k, two_minutes_ago)));
    b.used.insert(recent, start - Duration::from_secs(30));
    let live = |b: &Browser, key| b.model.lifecycle.live().any(|k| k == key);

    b.sleep_over_budget(start, 1500);
    assert!(!live(&b, heavy) && b.pages[&heavy].load.is_none(), "the heaviest idle tab sleeps");
    assert!([pinned, recent, playing, pane, shown].iter().all(|k| live(&b, *k)), "pinned, recent, playing and shown tabs stay awake");

    b.sleep_over_budget(start + Duration::from_secs(31 * 60), 1000);
    assert!(!live(&b, pinned), "a pinned tab sleeps after half an hour");
    assert!(live(&b, recent), "only as many sleep as the budget needs");
    assert!(live(&b, playing) && live(&b, pane) && live(&b, shown));
}

#[test]
fn sleep_after_never_turns_the_memory_budget_off() {
    use crate::memory::Usage;
    use std::time::{Duration, Instant};
    let (_dir, mut b) = browser();
    let heavy = b.new_tab("https://heavy.test/").unwrap();
    b.new_tab("https://shown.test/").unwrap();
    b.processes.insert(heavy, 100);
    b.samples.record([(100, Usage { footprint: 4 * engine::lifecycle::BACKGROUND_BUDGET, cpu_ns: 0 })].into(), Instant::now());
    b.used.insert(heavy, Instant::now() - Duration::from_secs(120));
    let live = |b: &Browser| b.model.lifecycle.live().any(|k| k == heavy);
    b.set_sleep_after(0);
    b.idle_tick(1);
    assert!(live(&b), "Never keeps the budget from sleeping tabs");
    b.set_sleep_after(2);
    b.idle_tick(1);
    assert!(!live(&b), "with a sleep time the budget applies");
}

#[test]
fn save_pdf_names_the_file_after_the_page_and_print_asks_the_active_tab() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.outbox.clear();
    b.print();
    assert!(b.pdf_name().is_none() && asked(&b, "print").is_empty(), "the start page has nothing to print");
    b.on_event(key, "committed", &json!({ "url": "https://news.example/a", "title": "Q3 / Q4 report" }));
    b.on_event(key, "finished", &json!({ "url": "https://news.example/a", "title": "Q3 / Q4 report" }));
    let url = "https://news.example/a".to_owned();
    assert_eq!(b.pdf_name(), Some((key, url.clone(), "Q3 - Q4 report.pdf".to_owned())));
    b.print();
    assert_eq!(asked(&b, "print").len(), 1);
    b.save_pdf(key, &url, std::path::Path::new("/d/Q3 - Q4 report.pdf"));
    assert_eq!(asked(&b, "pdf"), [json!({ "path": "/d/Q3 - Q4 report.pdf" })]);
    b.outbox.clear();
    visit(&mut b, key, "https://news.example/b");
    b.save_pdf(key, &url, std::path::Path::new("/d/x.pdf"));
    assert!(asked(&b, "pdf").is_empty(), "a tab that moved on is not saved under the old page's name");
    b.close_tab(key);
    b.save_pdf(key, "https://news.example/b", std::path::Path::new("/d/x.pdf"));
    assert!(asked(&b, "pdf").is_empty(), "a closed tab gets no command");
}

#[test]
fn tabs_are_bookmarked_once_filed_into_folders_and_reopened_from_them() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    assert_eq!(b.bookmark_tab(None), None, "the start page is not bookmarked");
    visit(&mut b, first, "https://a.test/");
    let id = b.bookmark_tab(None).unwrap();
    assert_eq!(b.bookmark_tab(Some(first)), Some(id));
    assert_eq!(b.bookmarks()[0].title, "T", "the page's current title is saved");
    let second = b.new_tab("https://b.test/").unwrap();
    visit(&mut b, second, "https://b.test/");
    let folder = b.bookmark_all_tabs().unwrap();
    let marks = b.bookmarks();
    assert_eq!(
        marks.iter().filter(|m| m.parent_id == Some(folder)).filter_map(|m| m.url.as_deref()).collect::<Vec<_>>(),
        ["https://a.test/", "https://b.test/"]
    );
    let sub = b.new_bookmark_folder(Some(folder)).unwrap();
    b.rename_bookmark(sub, "  Work  ");
    b.rename_bookmark(sub, " ");
    b.move_bookmark(id, Some(sub));
    b.move_bookmark(folder, Some(sub));
    let marks = b.bookmarks();
    let get = |id| marks.iter().find(|m| m.id == id).unwrap();
    assert_eq!(get(sub).title, "Work", "a blank name keeps the old one");
    assert_eq!(get(id).parent_id, Some(sub));
    assert_eq!(get(folder).parent_id, None, "a folder cannot move into its own subfolder");
    b.open_bookmark("https://a.test/");
    assert_eq!(b.model.active, Some(first), "an open page is switched to");
    let tabs = b.model.tabs.len();
    b.open_bookmark("javascript:alert(1)");
    assert_eq!(b.model.tabs.len(), tabs, "a bookmarklet is refused like a typed one");
    b.open_bookmark("https://c.test/");
    assert_eq!(b.model.tabs.len(), tabs + 1);
    assert_eq!(b.active_page().and_then(|p| p.typed.as_deref()), Some("https://c.test/"));
    b.delete_bookmark(folder);
    assert!(b.bookmarks().is_empty(), "a folder goes with everything in it");
}

fn group_row(b: &Browser, id: Id) -> crate::snapshot::Row {
    crate::snapshot::rows(b).into_iter().find(|r| r.kind == crate::snapshot::RowKind::Group(id)).unwrap()
}

fn pane_keys(row: &crate::snapshot::Row) -> Vec<crate::snapshot::RowKind> {
    row.panes.iter().map(|r| r.kind.clone()).collect()
}

#[test]
fn a_group_takes_a_listed_symbol_or_one_emoji_as_its_icon_and_keeps_it_when_reopened() {
    let (_dir, mut b) = browser();
    let a = b.new_tab("https://a.test/").unwrap();
    let id = b.new_group(Some(a)).unwrap();
    b.set_group_icon(id, "briefcase");
    assert_eq!(group_row(&b, id).icon, "briefcase");
    b.set_group_icon(id, "not a symbol");
    assert_eq!(group_row(&b, id).icon, "briefcase", "anything else changes nothing");
    assert!(b.toast.is_some(), "and says why");
    b.set_group_icon(id, "🚀");
    assert_eq!(group_row(&b, id).icon, "🚀");
    b.close_group(id);
    b.reopen();
    let reopened = b.model.groups.iter().find(|g| g.name == b.model.groups[0].name).map(|g| g.icon.clone());
    assert_eq!(reopened.as_deref(), Some("🚀"), "a reopened group keeps its icon");
}

#[test]
fn groups_hold_tabs_collapse_to_the_active_one_and_reopen_as_a_group() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let a = b.new_tab("https://a.test/").unwrap();
    let c = b.new_tab("https://c.test/").unwrap();
    let id = b.new_group(Some(a)).unwrap();
    b.set_tab_group(c, Some(id));
    assert_eq!(pane_keys(&group_row(&b, id)), [a, c].map(crate::snapshot::RowKind::Tab));
    let loose: Vec<_> = crate::snapshot::rows(&b).into_iter().filter(|r| matches!(r.kind, crate::snapshot::RowKind::Tab(_))).map(|r| r.kind).collect();
    assert_eq!(loose, [crate::snapshot::RowKind::Tab(first)], "grouped tabs show only in their card");
    b.toggle_group(id);
    let row = group_row(&b, id);
    assert!(!row.open);
    assert_eq!(pane_keys(&row), [crate::snapshot::RowKind::Tab(c)], "a collapsed group shows its active tab");
    b.rename_group(id, " Work ");
    b.set_group_color(id, "green");
    let row = group_row(&b, id);
    assert_eq!((row.label.as_str(), row.color), ("Work", 0x30d158));
    b.close_group(id);
    assert!(b.model.groups.is_empty() && b.model.tab(a).is_none() && b.model.tab(c).is_none());
    assert_eq!(b.toast.as_ref().unwrap().text, "Group closed");
    b.reopen();
    let group = b.model.groups[0].clone();
    assert_eq!((group.name.as_str(), group.color.as_str()), ("Work", "green"));
    assert_eq!(b.model.tabs.iter().filter(|t| t.group_id == Some(group.id)).count(), 2);
    let grouped = b.model.tabs.iter().find(|t| t.group_id == Some(group.id)).map(|t| b.model.key(t)).unwrap();
    b.close_tab(grouped);
    b.reopen();
    assert_eq!(b.model.tab(b.model.active.unwrap()).unwrap().group_id, Some(group.id), "a closed tab reopens in its group");
    b.regrouped_space = None;
    b.ungroup(group.id);
    assert!(b.model.groups.is_empty());
    assert!(b.model.tabs.iter().all(|t| t.group_id.is_none()));
    assert_eq!(b.regrouped_space, b.model.current_space().map(|s| (s.private, s.space.id)), "other windows on this space reload it");
    assert_eq!(b.forgotten_groups.last(), Some(&(false, group.id)), "and drop the group from their reopen stacks");
}

#[test]
fn a_dropped_tab_joins_the_group_it_lands_in() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let a = b.new_tab("https://a.test/").unwrap();
    let id = b.new_group(Some(a)).unwrap();
    b.move_tab_to(first, a, false);
    assert_eq!(b.model.tab(first).unwrap().group_id, Some(id), "dropped on a grouped tab");
    b.set_tab_group(first, None);
    b.pin_toggle(first);
    b.move_to_group(first, id);
    let tab = b.model.tab(first).unwrap();
    assert_eq!((tab.kind, tab.group_id), (TabKind::Ephemeral, Some(id)), "dropped on the group's header");
}

#[test]
fn clean_up_sorts_loose_tabs_into_groups_and_archives_unused_ones() {
    let (_dir, mut b) = browser();
    assert!(!crate::snapshot::snapshot(&b, true).cleanable);
    b.new_tab("https://github.com/tarn").unwrap();
    b.new_tab("https://gitlab.com/tarn").unwrap();
    assert!(crate::snapshot::snapshot(&b, true).cleanable);
    b.clean_up_tabs();
    assert_eq!(b.toast.as_ref().unwrap().text, "Sorted 2 tabs into groups");
    let group = b.model.groups[0].clone();
    assert_eq!((group.name.as_str(), group.color.as_str()), ("Development", "indigo"));
    assert!(!crate::snapshot::snapshot(&b, true).cleanable);
    b.clean_up_tabs();
    assert_eq!(b.toast.as_ref().unwrap().text, "Nothing to sort. Settings › Tabs can archive unused tabs too.");

    let old = b.new_tab("https://old.test/").unwrap();
    b.new_tab("https://new.test/").unwrap();
    b.core.disk.touch_tab(old.id, 1).unwrap();
    assert_eq!(b.archive_unused_tabs(), 0, "archiving is off by default");
    b.set_archive_after(2);
    assert_eq!(b.archive_after_index(), 2);
    assert!(b.model.tab(old).is_none(), "choosing a time archives right away");
    let archived = b.folder_pages(browser_core::closed::ARCHIVE_FOLDER);
    assert_eq!(archived.iter().map(|m| m.url.as_deref()).collect::<Vec<_>>(), [Some("https://old.test/")]);
    b.clean_up_tabs();
    assert_eq!(b.toast.as_ref().unwrap().text, "Already tidy");
}

#[test]
fn undo_sort_puts_the_tabs_back_and_removes_the_groups_clean_up_made() {
    let (_dir, mut b) = browser();
    let a = b.new_tab("https://github.com/tarn").unwrap();
    let c = b.new_tab("https://gitlab.com/tarn").unwrap();
    b.clean_up_tabs();
    let group = b.model.groups[0].clone();
    assert_eq!(group.icon, "code", "Clean Up gives the group its category's icon");
    assert!(matches!(b.toast.as_ref().unwrap().action, Some(crate::browser::ToastAction::UndoSort(_))));
    b.take_toast_action();
    assert!(b.model.groups.is_empty(), "the group it made goes once empty");
    assert!([a, c].iter().all(|k| b.model.tab(*k).is_some_and(|t| t.group_id.is_none())), "and the tabs are loose again");
    assert_eq!(b.forgotten_groups.last(), Some(&(false, group.id)), "and every window forgets the group");
}

#[test]
fn site_overrides_and_zoom_levels_are_listed_and_removed() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://a.test/");
    b.set_site_level(Some(ShieldLevel::Strict));
    b.zoom(1);
    assert_eq!(b.site_overrides(), [("a.test".to_owned(), ShieldLevel::Strict)]);
    assert_eq!(b.site_zooms(), [("a.test".to_owned(), 1.1)]);
    b.outbox.clear();
    b.remove_site_zoom(None);
    assert!(b.site_zooms().is_empty());
    assert_eq!(asked(&b, "zoom"), [json!({ "factor": 1.0 })], "the open page goes back to actual size");
    b.remove_site_override("a.test");
    assert!(b.site_overrides().is_empty());
    assert_eq!(b.level_for(key, "https://a.test/"), b.default_level());
}

#[test]
fn delete_everything_empties_the_profile_and_moves_its_store() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap();
    let old = b.store_prop(key);
    visit(&mut b, key, "https://a.test/");
    b.zoom(1);
    b.set_site_level(Some(ShieldLevel::Strict));
    b.new_tab("https://b.test/");
    let (name, json) = b.export_profile().unwrap();
    assert_eq!(name, "Tarn export Personal.json");
    assert!(json.contains("https://a.test/"));
    b.on_download(key, &json!({ "id": "5", "state": "started", "name": "a.zip", "path": "/d/a.zip" }), true);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let wipes = b.delete_profile_data();
    assert_eq!(wipes, [profile_data::Wipe { key: crate::data::StoreKey::parse(&old).unwrap(), remove: Some(old.clone()) }]);
    assert!(b.core.disk.history(profile.1, 10).unwrap().is_empty());
    assert!(b.site_zooms().is_empty() && b.site_overrides().is_empty());
    assert_eq!(b.model.profile(), Some(profile));
    assert_eq!(b.model.current_space().unwrap().space.name, "Home");
    assert_eq!(b.model.tabs.len(), 1);
    assert_ne!(b.store_prop(b.model.active.unwrap()), old, "the profile moved to a new website data store");
    b.on_download(key, &json!({ "id": "5", "state": "finished", "name": "a.zip", "path": "/d/a.zip", "url": "https://d.test/a.zip" }), true);
    assert!(b.core.disk.downloads(profile.1).unwrap().is_empty(), "a download started before the wipe is not recorded");
}

#[test]
fn delete_everything_in_the_private_space_clears_its_session() {
    let (_dir, mut b) = browser();
    b.private_space();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://a.test/");
    let prop = b.store_prop(key);
    let wipes = b.delete_profile_data();
    assert_eq!(wipes, [profile_data::Wipe { key: crate::data::StoreKey::parse(&prop).unwrap(), remove: None }]);
    assert!(b.model.current_space().unwrap().private);
    assert_eq!(b.store_prop(b.model.active.unwrap()), prop, "the private session stays, emptied");
    assert!(b.export_profile().is_err(), "private data is never exported");
}

#[test]
fn only_delete_everything_drops_finished_downloads_from_the_list() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let (private, profile) = b.model.profile().unwrap();
    b.on_download(key, &json!({ "id": "6", "state": "finished", "name": "b.zip", "path": "/d/b.zip", "url": "https://d.test/b.zip" }), true);
    b.forget_profile(private, profile);
    assert_eq!(b.downloads.len(), 1, "another window keeps its list");
    b.delete_profile_data();
    assert!(b.downloads.is_empty());
}

#[test]
fn a_dangerous_site_gets_the_warning_page_and_visit_anyway_waits_then_lasts_until_quit() {
    use browser_core::site::BYPASS_DELAY;
    use shield::Category;
    let (dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://news.test/");
    b.on_event(key, "blocked", &json!({ "url": "https://evil.test/x", "reason": "listed", "category": "malware" }));
    assert_eq!(blocked_page(&b).map(|p| (p.url, p.category)), Some(("https://evil.test/x".to_owned(), Category::Malware)));
    assert!(crate::snapshot::snapshot(&b, true).blocked.is_some(), "the warning page covers the tab");
    assert!(!b.pages[&key].loading);
    b.visit_anyway();
    assert!(blocked_page(&b).is_some(), "a click right after the page showed is ignored");
    b.visit_anyway_at(b.guard_since + BYPASS_DELAY);
    assert!(blocked_page(&b).is_none());
    assert_eq!(b.pages[&key].load.as_ref().map(|l| l.0.as_str()), Some("https://evil.test/x"), "loaded as the user's own navigation");
    assert_eq!(b.blocking(key)["exceptions"], json!(["evil.test"]));
    let other = b.new_tab("https://news.test/").unwrap();
    assert_eq!(b.blocking(other)["exceptions"], json!(["evil.test"]), "remembered for the profile");
    b.private_space();
    let private = b.model.active.unwrap();
    assert_eq!(b.blocking(private)["exceptions"], json!([]), "not for another profile");
    drop(b);
    let again = Browser::open(dir.path());
    assert_eq!(again.blocking(again.model.active.unwrap())["exceptions"], json!([]), "nothing is stored");
}

#[test]
fn adult_sites_have_no_way_past_and_go_back_keeps_the_page() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://news.test/");
    b.on_event(key, "blocked", &json!({ "url": "https://adult.test/", "reason": "listed", "category": "adult" }));
    b.visit_anyway_at(b.guard_since + std::time::Duration::from_secs(60));
    assert!(blocked_page(&b).is_some(), "Visit Anyway is for malware only");
    b.leave_blocked();
    assert!(blocked_page(&b).is_none());
    assert_eq!(b.pages[&key].url, "https://news.test/");
    b.on_event(key, "blocked", &json!({ "url": "https://adult.test/", "reason": "listed", "category": "adult" }));
    b.navigate_in("https://other.test/", false);
    assert!(blocked_page(&b).is_none(), "a new load leaves the warning page");
    b.on_event(key, "blocked", &json!({ "url": "https://ads.test/", "reason": "listed", "category": "popups" }));
    assert!(blocked_page(&b).is_none());
    assert!(matches!(&b.notice, Some(Notice::Open { message, .. }) if message == "Stopped a redirect to ads.test, a known pop-up ad site."));
}

#[test]
fn pausing_a_site_drops_its_ad_rules_and_the_privacy_list_turns_it_on_again() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.navigate_in("https://news.test/", false);
    visit(&mut b, key, "https://news.test/");
    let view = |b: &Browser| crate::snapshot::views(b, true).into_iter().find(|v| v.key == key).unwrap();
    assert!(view(&b).filtered);
    b.set_site_paused("news.test", true);
    assert!(b.site_paused("news.test") && !view(&b).filtered, "the tab reloads without ad, tracker and cookie-banner rules");
    assert_eq!(b.paused_sites(), ["news.test"]);
    assert_eq!(b.blocking(key)["paused"], json!(["news.test"]));
    let profile = b.model.profile().unwrap().1;
    assert_eq!(b.core.disk.paused_sites(profile).unwrap(), ["news.test"]);
    b.set_site_paused("news.test", false);
    assert!(view(&b).filtered && b.paused_sites().is_empty());
}

#[test]
fn pop_up_rules_and_permission_rows_belong_to_the_site_in_the_popover() {
    use engine::nav_policy::PopupRule;
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://news.test/a");
    b.set_site_popups(PopupRule::Block);
    assert_eq!(b.site_popups(), PopupRule::Block);
    assert_eq!(b.blocking(key)["popups"], json!({ "https://news.test": "block" }));
    b.set_site_popups(PopupRule::Default);
    assert_eq!(b.blocking(key)["popups"], json!({}));
    let profile = b.model.profile().unwrap().1;
    b.core.disk.set_permission(profile, "https://news.test", "microphone", "deny").unwrap();
    b.core.disk.set_permission(profile, "https://other.test", "camera", "allow").unwrap();
    let rows = b.site_permissions();
    assert_eq!(rows.iter().map(|r| (r.label.as_str(), r.decision.as_str())).collect::<Vec<_>>(), [("Microphone", "deny")]);
    b.set_site_permission("https://news.test", "microphone", Some("ask"));
    b.on_event(key, "prompt", &json!({ "id": 4, "kind": "microphone", "origin": "https://news.test" }));
    assert!(asked(&b, "answer").is_empty(), "Ask asks again");
    assert_eq!(b.pages[&key].prompts.len(), 1);
    b.set_site_permission("https://news.test", "microphone", None);
    assert!(b.site_permissions().is_empty());
}

#[test]
fn privacy_switches_choose_the_domain_sets_and_adult_starts_off() {
    use shield::Category;
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    assert_eq!(b.set_categories(), [Category::Malware, Category::Popups]);
    assert_eq!(b.blocking(key)["sets"], json!(["malware", "popups"]));
    b.toggle_set(Category::Adult);
    b.toggle_set(Category::Popups);
    assert_eq!(b.blocking(key)["sets"], json!(["malware", "adult"]));
    b.toggle_set(Category::Malware);
    assert!(!b.lists().contains(&"malware".to_owned()), "the malware switch also turns off its rule list");
}

#[test]
fn clearing_site_data_forgets_its_zaps_and_reports_what_went() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://news.test/");
    let profile = b.model.profile().unwrap().1;
    b.core.disk.zap_element(profile, "news.test", "div.ad").unwrap();
    assert!(b.clear_site("news.test").is_some(), "the tab's store is named");
    assert!(b.core.disk.zapped_elements(profile).unwrap().is_empty());
    b.site_cleared("news.test", 2);
    assert_eq!(b.notice, Some(Notice::Message("Cleared stored data for news.test.".into())));
    assert_eq!(asked(&b, "reload").len(), 1);
    b.site_cleared("news.test", 0);
    assert_eq!(b.notice, Some(Notice::Message("news.test had no stored data.".into())));
}

#[test]
fn the_library_searches_tabs_bookmarks_history_and_downloads() {
    let (_dir, mut b) = browser();
    let (private, profile) = b.model.profile().unwrap();
    let store = b.core.store(private);
    store.add_history(profile, "https://news.test/a", "Morning News", 1, false).unwrap();
    store.add_history(profile, "https://news.test/a", "Morning News", 2, false).unwrap();
    store.add_history(profile, "https://cook.test/", "Recipes", 3, false).unwrap();
    store.add_bookmark(profile, None, Some("https://news.test/b"), "Evening News", 4).unwrap();
    store.add_download(profile, "https://files.test/n", "/tmp/news-report.pdf", "finished", 5).unwrap();
    b.new_tab("https://news.test/live");
    let history = b.library_history();
    assert_eq!(history.len(), 2);

    let all = b.library_search("", &history);
    assert_eq!(all.tab_count, b.model.tabs.len());
    assert!(all.bookmarks.is_empty() && all.history.is_empty() && all.downloads.is_empty());

    let found = b.library_search("news", &history);
    assert_eq!(found.tabs.len(), 1);
    assert_eq!(found.bookmarks.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Evening News"]);
    assert_eq!(found.history.iter().map(|l| l.url.as_str()).collect::<Vec<_>>(), ["https://news.test/a"]);
    assert_eq!(found.downloads.len(), 1);
    assert!(b.library_search("zzzz", &history).is_empty());
}

#[test]
fn library_rows_reuse_an_open_tab_and_edit_downloads_and_snapshots() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    let two = b.new_tab("https://two.test/").unwrap();
    b.activate(first);
    let tabs = b.model.tabs.len();
    b.open_url("https://two.test/");
    assert_eq!((b.model.tabs.len(), b.model.active), (tabs, Some(two)));
    b.open_url("https://three.test/");
    assert_eq!(b.model.tabs.len(), tabs + 1);

    let (private, profile) = b.model.profile().unwrap();
    b.core.store(private).add_download(profile, "https://files.test/a", "/tmp/a.zip", "finished", 1).unwrap();
    b.core.store(private).add_download(profile, "https://files.test/b", "/tmp/b.zip", "finished", 2).unwrap();
    let gone = b.saved_downloads()[0].id;
    b.remove_download(gone);
    assert_eq!(b.saved_downloads().iter().map(|d| d.id).filter(|id| *id == gone).count(), 0);
    assert_eq!(b.saved_downloads().len(), 1);

    b.save_snapshot();
    let id = b.snapshots()[0].id;
    b.rename_snapshot(id, "  ");
    assert_ne!(b.snapshots()[0].name, "");
    b.rename_snapshot(id, " Trip ");
    assert_eq!(b.snapshots()[0].name, "Trip");
}

#[test]
fn a_shift_clicked_link_peeks_in_the_openers_profile_and_moves_into_a_tab() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://a.test/");
    b.on_event(key, "peek", &json!({ "url": "https://b.test/page", "link": true }));
    assert_eq!(b.model.tabs.len(), 1, "a peek is not a tab");
    let peek = crate::snapshot::snapshot(&b, true).link_peek.expect("the peek shows");
    assert_eq!((peek.view.key, peek.view.profile.clone(), peek.view.link), (key, b.store_prop(key), true));
    assert_eq!(peek.view.load.as_ref().map(|(u, _)| u.as_str()), Some("https://b.test/page"));
    let opener = format!("{}-{}", key.private, key.id);
    assert_eq!(peek.view.opener, opener, "the peek shares its opener's pop-up and prompt limits");
    assert_ne!(peek.tab, opener, "the peek is a view of its own");
    assert_eq!(crate::snapshot::tab_key(&peek.tab), Some(key), "its downloads belong to the opener");
    let tab = crate::snapshot::views(&b, true).into_iter().find(|v| v.key == key).expect("the opener's view");
    assert_eq!(
        (&peek.view.blocking, peek.view.filtered, peek.view.youtube, peek.view.inspectable, &peek.view.lists),
        (&tab.blocking, tab.filtered, tab.youtube, tab.inspectable, &tab.lists)
    );
    assert!(crate::snapshot::snapshot(&b, false).link_peek.is_none(), "hidden while a panel covers the page");
    b.on_peek_event("committed", &json!({ "url": "https://b.test/next", "title": "Next" }));
    assert_eq!(crate::snapshot::snapshot(&b, true).link_peek.map(|p| (p.host, p.title)), Some(("b.test".into(), "Next".into())));
    b.on_peek_event("prompt", &json!({ "id": 4, "kind": "alert", "origin": "https://b.test" }));
    assert_eq!(b.peek_outbox, vec![("answer", json!({ "id": 4, "accept": false }))], "the peek's dialogs are declined");
    b.link_peek_to_tab();
    assert!(b.link_peek.is_none());
    assert_eq!(b.model.tabs.len(), 2);
    let tab = b.model.active.unwrap();
    assert_ne!(tab, key);
    assert_eq!(b.pages[&tab].load.as_ref().map(|(u, _)| u.as_str()), Some("https://b.test/next"));
    assert!(b.pages[&tab].link, "the page's link, not the user's address");
}

#[test]
fn a_peek_reloads_at_a_stricter_level_and_closes_with_its_tab() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.on_event(key, "peek", &json!({ "url": "https://b.test/" }));
    let seq = b.link_peek.as_ref().unwrap().load.1;
    b.on_peek_event("blocked", &json!({ "url": "https://b.test/", "reason": "rebuild" }));
    assert!(b.link_peek.as_ref().unwrap().load.1 > seq, "loaded again in a rebuilt view");
    b.on_peek_event("peek", &json!({ "url": "https://c.test/" }));
    assert_eq!(b.link_peek.as_ref().map(|p| p.load.0.as_str()), Some("https://c.test/"));
    let other = b.new_tab("https://d.test/").unwrap();
    assert!(b.link_peek.is_none(), "switching tabs closes the peek");
    b.activate(key);
    b.on_event(key, "peek", &json!({ "url": "https://b.test/" }));
    b.close_tab(key);
    assert!(b.link_peek.is_none());
    assert_eq!(b.model.active, Some(other));
}

#[test]
fn compact_toolbar_and_auto_hide_sidebar_are_off_until_turned_on() {
    let (_dir, mut b) = browser();
    assert!(!b.compact_toolbar() && !b.auto_hide_sidebar());
    b.toggle_off_setting(COMPACT_TOOLBAR);
    b.toggle_off_setting(AUTO_HIDE_SIDEBAR);
    assert!(b.compact_toolbar() && b.auto_hide_sidebar());
    assert!(crate::snapshot::snapshot(&b, true).compact);
    assert_eq!(b.core.disk.setting(COMPACT_TOOLBAR).unwrap().as_deref(), Some("on"), "the key the macOS client reads");
}

#[test]
fn links_from_other_apps_open_in_new_tabs_and_load_as_a_pages_link() {
    let (_dir, mut b) = browser();
    let before = b.model.tabs.len();
    let args = ["https://a.test/?utm_source=x", "javascript:alert(1)", "file:///etc/passwd", "--new-window", "http://localhost:3000/", "http://192.168.1.1/"];
    b.open_app_links(&args.map(str::to_owned));
    assert_eq!(b.model.tabs.len(), before + 3, "only web addresses open");
    let link = |b: &Browser, url: &str| b.pages.values().find(|p| p.load.as_ref().is_some_and(|(u, _)| u == url)).map(|p| p.link);
    assert_eq!(link(&b, "https://a.test/"), Some(true), "tracking parameters go and the load is a page's link");
    assert_eq!(link(&b, "http://localhost:3000/"), Some(false), "localhost opens as typed by default");
    assert_eq!(link(&b, "http://192.168.1.1/"), Some(true));
    b.toggle_setting(browser_core::incoming::LOCALHOST_FROM_APPS);
    b.open_app_links(&["http://localhost:4000/".to_owned()]);
    assert_eq!(link(&b, "http://localhost:4000/"), Some(true));
}

#[test]
fn only_a_web_page_has_a_qr_code() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "file:///home/me/a.html");
    assert!(b.qr_code().is_none());
    assert_eq!(b.toast.as_ref().unwrap().text, "Only web pages have a QR code");
}

#[test]
fn open_tabs_are_offered_as_switch_to_tab_rows_across_the_profiles_spaces() {
    let (_dir, mut b) = browser();
    let docs = b.new_tab("https://docs.test/guide").unwrap();
    b.new_tab("https://news.test/").unwrap();
    let rows = b.tab_suggestions("docs");
    assert_eq!(rows.iter().map(|r| r.key).collect::<Vec<_>>(), [docs]);
    assert_eq!(rows[0].detail(), "Switch to Tab");
    assert!(b.tab_suggestions(" ").is_empty());
    assert!(b.tab_suggestions("news").is_empty(), "the current tab is not offered");
    let home = b.model.space_ix;
    b.save_space(&b.new_space_draft());
    let rows = b.tab_suggestions("docs.test/guide");
    assert_eq!(rows.iter().map(|r| (r.key, r.space.is_some())).collect::<Vec<_>>(), [(docs, true)]);
    assert!(rows[0].detail().starts_with("Switch to Tab · "));
    b.switch_to_tab(docs);
    assert_eq!((b.model.space_ix, b.model.active), (home, Some(docs)));
}

#[test]
fn page_commands_copy_the_title_or_a_markdown_link_and_close_or_sleep_tabs() {
    let (_dir, mut b) = browser();
    let key = b.new_tab("https://a.test/x").unwrap();
    b.on_event(key, "committed", &json!({ "url": "https://a.test/x", "title": "A [b]" }));
    assert_eq!(b.copy_page(true).as_deref(), Some(r"[A \[b\]](https://a.test/x)"));
    assert_eq!(b.copy_page(false).as_deref(), Some("A [b]"));
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("Title copied"));
    b.duplicate_tab(key);
    let copy = b.model.active.unwrap();
    assert_ne!(copy, key);
    assert!(b.pages[&copy].load.as_ref().is_some_and(|(u, _)| u == "https://a.test/x"));
    b.sleep_tabs(b.model.other_tabs(copy));
    assert!(b.model.lifecycle.live().all(|k| k == copy));
    b.close_all_tabs();
    assert_eq!(b.model.tabs.len(), 1, "a blank tab replaces them");
    assert_eq!(b.active_url(), None);
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("3 tabs closed"));
}

#[test]
fn a_screenshot_is_named_after_the_page_and_recorded_in_its_own_profile() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    b.on_event(key, "finished", &json!({ "url": "https://a.test/x", "title": "Q3 / Q4 report" }));
    b.take_screenshot();
    let requests = asked(&b, "screenshot");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["name"], "Q3 - Q4 report.png");
    let seq = requests[0]["seq"].clone();
    let other = b.new_tab("https://b.test/").unwrap();
    b.on_event(other, "screenshot", &json!({ "seq": seq, "path": "/d/elsewhere.png" }));
    assert!(b.core.disk.downloads(profile).unwrap().is_empty(), "only the tab asked answers");
    b.on_event(key, "screenshot", &json!({ "seq": seq, "path": "/d/Q3 - Q4 report.png" }));
    let saved = b.core.disk.downloads(profile).unwrap();
    assert_eq!((saved.len(), saved[0].path.as_str(), saved[0].url.as_str()), (1, "/d/Q3 - Q4 report.png", "https://a.test/x"));
    let toast = b.toast.clone().unwrap();
    assert_eq!((toast.text.as_str(), toast.action), ("Screenshot saved", Some(ToastAction::Reveal("/d/Q3 - Q4 report.png".into()))));
    b.on_event(key, "screenshot", &json!({ "seq": seq, "path": "/d/again.png" }));
    assert_eq!(b.core.disk.downloads(profile).unwrap().len(), 1, "one answer per request");

    b.activate(key);
    b.take_screenshot();
    let seq = asked(&b, "screenshot").last().unwrap()["seq"].clone();
    b.on_event(key, "screenshot", &json!({ "seq": seq, "message": "WebKit could not capture the page" }));
    assert_eq!(b.notice, Some(Notice::Message("Download failed: WebKit could not capture the page".into())));
}

#[test]
fn a_burner_screenshot_is_saved_but_not_recorded() {
    let (_dir, mut b) = browser();
    b.open_burner();
    let key = b.model.active.unwrap();
    let profile = b.model.profile().unwrap().1;
    b.take_screenshot();
    let seq = asked(&b, "screenshot")[0]["seq"].clone();
    b.on_event(key, "screenshot", &json!({ "seq": seq, "path": "/d/Page.png" }));
    assert!(b.core.store(true).downloads(profile).unwrap().is_empty());
    assert_eq!(b.toast.as_ref().map(|t| t.text.as_str()), Some("Screenshot saved"));
}

#[test]
fn a_full_screen_page_hides_the_shell_until_the_shell_needs_it() {
    use nsh_core::desktop::WindowMode;
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://video.test/");
    b.window_changed(WindowMode::Maximized);
    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    assert!(b.page_fullscreen.is_on());
    assert_eq!(b.window_request.take(), Some(WindowMode::FullScreen));
    b.window_changed(WindowMode::FullScreen);
    assert!(crate::snapshot::snapshot(&b, true).fullscreen);
    b.keep_page_fullscreen(false, false, false);
    assert!(b.page_fullscreen.is_on());
    b.on_event(key, "fullscreen", &json!({ "on": false }));
    assert!(!b.page_fullscreen.is_on());
    assert_eq!(b.window_request.take(), Some(WindowMode::Maximized), "the window goes back to how it was");

    b.window_changed(WindowMode::Maximized);
    for shell in [(true, false, false), (false, true, false), (false, false, true)] {
        b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
        b.window_request = None;
        b.keep_page_fullscreen(shell.0, shell.1, shell.2);
        assert!(!b.page_fullscreen.is_on(), "{shell:?}");
        assert_eq!(b.window_request.take(), Some(WindowMode::Maximized));
    }
    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    b.notice = Some(Notice::Message("m".into()));
    b.keep_page_fullscreen(false, false, false);
    assert!(!b.page_fullscreen.is_on(), "a notice brings the shell back");
    b.notice = None;

    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    b.new_tab("https://b.test/").unwrap();
    assert!(!b.page_fullscreen.is_on(), "another tab brings the shell back");
    b.window_request = None;
    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    assert!(!b.page_fullscreen.is_on(), "only the active tab fills the window");
    assert_eq!(b.window_request.take(), Some(WindowMode::Maximized), "WebKit's full-screen window is undone");

    b.activate(key);
    b.on_event(key, "prompt", &json!({ "id": 1, "kind": "camera", "origin": "https://video.test" }));
    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    assert!(!b.page_fullscreen.is_on(), "a pending prompt keeps the shell");
    assert_eq!(b.window_request.take(), Some(WindowMode::Maximized));
}

#[test]
fn full_screen_keys_and_pages_keep_a_window_that_was_already_full_screen() {
    use nsh_core::desktop::WindowMode;
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.toggle_full_screen();
    assert_eq!(b.window_request.take(), Some(WindowMode::FullScreen));
    b.window_changed(WindowMode::FullScreen);
    b.on_event(key, "fullscreen", &json!({ "on": true, "windowFullScreen": true }));
    assert!(b.page_fullscreen.is_on());
    b.window_request = None;
    b.toggle_full_screen();
    assert!(!b.page_fullscreen.is_on(), "the key first gives the shell back");
    assert_eq!(b.window_request.take(), Some(WindowMode::FullScreen), "the window stays in full screen");
    b.on_event(key, "fullscreen", &json!({ "on": false }));
    assert_eq!(b.window_request.take(), None);
    b.toggle_full_screen();
    assert_eq!(b.window_request.take(), Some(WindowMode::Normal));
}

#[test]
fn a_full_screen_page_alone_fills_a_split() {
    let (_dir, mut b) = browser();
    let first = b.model.active.unwrap();
    visit(&mut b, first, "https://a.test/");
    b.new_tab("https://b.test/").unwrap();
    b.activate(first);
    b.open_split();
    let active = b.model.active.unwrap();
    b.on_event(active, "fullscreen", &json!({ "on": true, "windowFullScreen": false }));
    let snap = crate::snapshot::snapshot(&b, true);
    assert!(snap.split.is_none());
    assert_eq!(snap.views.iter().filter(|v| v.active).map(|v| (v.key, v.pane)).collect::<Vec<_>>(), [(active, None)]);
}

#[test]
fn profiles_are_created_edited_switched_and_deleted_with_their_store_retired() {
    let (_dir, mut b) = browser();
    let first = b.current_profile().unwrap();
    assert!(!b.delete_profile(first), "the last profile stays");
    let new = browser_core::profiles::NewProfile {
        name: " Work ",
        color: "red",
        level: ShieldLevel::Strict,
        engine: browser_core::command::ENGINES[1].name,
        share: None,
        space: ["Desk", "green", "code"],
    };
    b.create_profile(&new).unwrap();
    let work = b.current_profile().unwrap();
    assert_ne!(work, first);
    let space = b.model.current_space().unwrap().space.clone();
    assert_eq!((space.name.as_str(), space.color.as_str(), space.icon.as_str()), ("Desk", "green", "code"));
    assert_eq!(b.core.disk.profile(work).unwrap().name, "Work");
    assert!(b.create_profile(&new).is_err(), "names are unique");
    assert_eq!(b.create_profile(&browser_core::profiles::NewProfile { name: " ", ..new }), Err("Name the profile.".to_owned()));
    std::fs::create_dir_all(b.extensions_root.join(work.to_string()).join("install")).unwrap();
    assert!(!b.edit_profile(work, "personal", "blue"), "taken by the first profile");
    assert!(b.edit_profile(work, "Office", "blue"));
    assert_eq!((b.core.disk.profile(work).unwrap().name.as_str(), b.core.disk.profile(work).unwrap().color.as_str()), ("Office", "blue"));
    let key = b.new_tab("https://work.test/").unwrap();
    let store = b.store_key().unwrap();
    b.switch_profile(first);
    assert_eq!(b.current_profile(), Some(first));
    b.switch_profile(work);
    assert!(b.delete_profile(work));
    assert!(b.core.disk.profile(work).is_err());
    assert!(!b.extensions_root.join(work.to_string()).exists(), "its extension files go too");
    assert!(!b.pages.contains_key(&key));
    assert_eq!(b.current_profile(), Some(first));
    assert!(b.released.contains(&store));
    let crate::data::StoreKey::Persistent(uuid) = store else { panic!("a saved profile's store") };
    assert_eq!(b.retired, std::slice::from_ref(&uuid));
    assert_eq!(browser_core::profiles::retired_stores(&b.core.disk), [uuid]);
}

#[test]
fn spaces_are_edited_deleted_and_switched_by_number() {
    let (_dir, mut b) = browser();
    let home = b.model.space_ix;
    let home_key = (false, b.model.spaces[home].space.id);
    let before = b.model.spaces.len();
    assert!(!b.model.can_delete_space(home), "a profile's last space stays");
    b.delete_space(home_key);
    assert_eq!(b.model.spaces.len(), before);
    let mut draft = b.new_space_draft();
    (draft.name, draft.color, draft.icon) = ("Work".into(), "red".into(), "briefcase".into());
    assert!(b.save_space(&draft));
    let work = b.model.space_ix;
    assert_ne!(work, home);
    let mut edit = SpaceDraft::edit(&b.model.spaces[home]);
    edit.name = " work".into();
    assert!(!b.save_space(&edit), "taken in this profile");
    assert!(!b.save_space(&SpaceDraft { editing: Some(999), name: "Gone".into(), ..edit.clone() }), "a space deleted meanwhile");
    (edit.name, edit.color, edit.icon) = ("Home".into(), "blue".into(), "star".into());
    assert!(b.save_space(&edit));
    let edited = &b.model.spaces[home].space;
    assert_eq!((edited.color.as_str(), edited.icon.as_str()), ("blue", "star"));
    assert_eq!(b.model.space_ix, work, "editing does not switch");
    b.space_number(home + 1);
    assert_eq!(b.model.space_ix, home);
    b.space_number(9);
    assert_eq!(b.model.space_ix, home, "no ninth space");
    let key = b.new_tab("https://gone.test/").unwrap();
    b.space_number(work + 1);
    let shown = b.model.active;
    b.delete_space(home_key);
    assert_eq!(b.model.spaces.len(), before);
    assert!(b.core.disk.tab(key.id).is_err() && !b.pages.contains_key(&key));
    assert_eq!(b.model.current_space().unwrap().space.name, "Work");
    assert_eq!(b.model.active, shown, "deleting another space keeps the tab shown");
}

#[test]
fn developer_tools_follow_the_switch_and_the_developer_profile() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://api.test/data.json");
    assert!(!b.devtools_on() && asked(&b, "readJson").is_empty());
    assert!(crate::snapshot::views(&b, true).iter().all(|v| !v.inspectable));
    b.set_developer(true);
    assert!(b.devtools_on() && crate::snapshot::views(&b, true).iter().all(|v| v.inspectable));
    visit(&mut b, key, "https://api.test/data.json");
    assert!(asked(&b, "clearCache").is_empty(), "only the Developer profile clears its cache");
    let seq = asked(&b, "readJson").first().and_then(|a| a["seq"].as_u64()).expect("a JSON read after the load");
    assert!(!b.json_request(key, &json!({ "seq": seq, "url": "https://evil.test/" })), "an answer for another page");
    visit(&mut b, key, "https://api.test/data.json");
    let seq = asked(&b, "readJson").last().and_then(|a| a["seq"].as_u64()).unwrap();
    assert!(b.json_request(key, &json!({ "seq": seq, "url": "https://api.test/data.json" })));
    assert!(!b.json_request(key, &json!({ "seq": seq, "url": "https://api.test/data.json" })), "an answer counts once");
    b.show_json(key, "https://api.test/old.json", devkit::json_view::Tree::parse("[]").unwrap());
    assert!(b.active_json().is_none(), "a tree for a page no longer shown");
    b.show_json(key, "https://api.test/data.json", devkit::json_view::Tree::parse(r#"{"a":1}"#).unwrap());
    assert!(b.active_json().is_some_and(|t| t.rows.len() == 2));
    b.set_json_raw(true);
    assert!(b.active_json().is_some_and(|t| t.raw));
    b.on_event(key, "committed", &json!({ "url": "https://api.test/other", "title": "T" }));
    assert!(b.active_json().is_none());
    b.outbox.clear();
    b.reload();
    assert_eq!(b.outbox.last().map(|(_, n, _)| *n), Some("reload"));
    visit(&mut b, key, "https://locked.test/data.json");
    b.set_site_level(Some(ShieldLevel::Lockdown));
    b.outbox.clear();
    visit(&mut b, key, "https://locked.test/data.json");
    assert!(asked(&b, "readJson").is_empty(), "no script of Tarn's runs on a Lockdown page");
    b.set_developer(false);
    b.create_dev_profile();
    assert!(b.current_is_dev() && b.devtools_on() && b.dev_profile().is_some());
    let dev = b.model.active.unwrap();
    b.outbox.clear();
    visit(&mut b, dev, "https://a.test/");
    assert!(!asked(&b, "clearCache").is_empty() && !asked(&b, "readJson").is_empty());
    b.reload();
    assert_eq!(b.outbox.last().map(|(_, n, _)| *n), Some("reloadFromOrigin"));
    let spaces = b.model.spaces.len();
    b.create_dev_profile();
    assert_eq!(b.model.spaces.len(), spaces, "a second Create opens the same profile");
}

#[test]
fn responsive_sizes_zoom_the_page_and_give_its_zoom_back() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    b.cycle_responsive();
    assert!(b.responsive_preset().is_none(), "only while the developer tools are on");
    b.set_developer(true);
    assert_eq!(b.responsive_preset().map(|p| p.name), Some("iPhone SE"));
    b.outbox.clear();
    b.fit_responsive(1000., 333.5);
    assert_eq!(asked(&b, "zoom"), [json!({ "factor": 0.5 })]);
    b.fit_responsive(1000., 333.5);
    assert_eq!(asked(&b, "zoom").len(), 1);
    b.zoom(1);
    assert_eq!(asked(&b, "zoom").len(), 1, "the zoom keys wait while a size is shown");
    let profile = b.model.profile().unwrap().1;
    b.core.disk.set_site_zoom(profile, "z.test", Some(1.25)).unwrap();
    b.on_event(key, "committed", &json!({ "url": "https://z.test/", "title": "Z" }));
    assert_eq!(asked(&b, "zoom").len(), 1, "a saved site zoom waits too");
    b.set_developer(false);
    assert_eq!(
        b.outbox.last().map(|(k, n, a)| (*k, *n, a.clone())),
        Some((key, "zoom", json!({ "factor": 1.25 }))),
        "the switch going off gives the zoom back"
    );
    b.set_developer(true);
    b.cycle_responsive();
    b.fit_responsive(1000., 333.5);
    (0..devkit::responsive::PRESETS.len()).for_each(|_| b.cycle_responsive());
    assert!(b.responsive_preset().is_none());
    assert_eq!(b.outbox.last().map(|(k, n, a)| (*k, *n, a.clone())), Some((key, "zoom", json!({ "factor": 1.25 }))));
}

#[test]
fn browser_data_imports_into_the_current_profile_and_never_into_a_private_space() {
    use browser_core::import::{self as imports, Kinds, Outcome};
    let (_dir, mut b) = browser();
    let profile = b.model.profile().unwrap().1;
    let source_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        source_dir.path().join("Bookmarks"),
        r#"{"roots":{"bookmark_bar":{"type":"folder","name":"Bookmarks Bar","children":[{"type":"url","name":"A","url":"https://a.test/"},{"type":"url","name":"J","url":"javascript:x"}]}}}"#,
    )
    .unwrap();
    let source = imports::Source { browser: imports::Browser::Chromium, label: "Default".into(), dir: source_dir.path().into() };
    let confirm = imports::confirm(&source, "Personal", "this computer");
    assert!(confirm.cookies && confirm.detail.contains("keyring"), "{}", confirm.detail);
    assert_eq!(b.core.import_target(false, profile), Ok(()));
    let read = imports::read(&source, Kinds { cookies: false, history: true, bookmarks: true });
    let report = Rc::new(std::cell::RefCell::new(None));
    let slot = report.clone();
    b.core.import(profile, source.browser, read, move |r| *slot.borrow_mut() = Some(r)).unwrap();
    let report = report.take().unwrap();
    assert_eq!(report.bookmarks, Some(Ok(Outcome { imported: 1, skipped: 1, failed: 0 })));
    assert_eq!(report.history, Some(Err(imports::Error::NotFound)));
    assert_eq!(imports::summary("Chromium", Some(&report)), "History: none found in this profile.\nBookmarks: 1 imported, 1 skipped.");
    let marks = b.core.disk.bookmarks(profile).unwrap();
    assert!(marks.iter().any(|m| m.parent_id.is_none() && m.title == "Imported from Chromium"));
    assert!(marks.iter().any(|m| m.url.as_deref() == Some("https://a.test/")));
    b.private_space();
    let space = b.model.current_space().unwrap();
    assert!(space.private);
    assert!(b.core.import_target(true, space.space.profile_id).is_err(), "a private space keeps nothing, so it takes no import");
}

#[test]
fn filter_list_updates_default_on_switch_off_and_show_the_installed_lists() {
    let (_dir, mut b) = browser();
    let (switch, summary) = b.filter_updates();
    assert_eq!(switch, Some(true));
    assert_eq!(summary, "No lists installed yet.");
    b.toggle_setting(shield::update::SETTING);
    assert_eq!(b.filter_updates().0, Some(false));
    assert_eq!(b.core.disk.setting(shield::update::SETTING).unwrap().as_deref(), Some("off"));
    let lists =
        vec![shield::bundle::VerifiedList { name: "ads.txt".into(), category: "ads".into(), kind: shield::bundle::Kind::Filters, count: 3, data: Vec::new() }];
    let mut status = shield::update::Status::default();
    status.installed(&shield::bundle::VerifiedBundle { version: shield::update::now(), created: "2026-11-17T08:00:00Z".into(), lists });
    status.checked(Err("offline".into()));
    crate::filter_updates::set_status(status);
    assert_eq!(b.filter_updates().1, "Lists from 2026-11-17 installed. Last update failed: offline.");
    assert_eq!(crate::filter_updates::status().counts.get("ads"), Some(&3));
    b.toggle_setting(shield::update::SETTING);
    assert_eq!(b.filter_updates().0, Some(true));
}

#[test]
fn a_hovered_link_shows_until_the_pointer_leaves_or_the_page_changes() {
    let (_dir, mut b) = browser();
    let key = b.model.active.unwrap();
    visit(&mut b, key, "https://a.test/");
    b.on_event(key, "hover", &json!({ "url": "javascript:void(0)" }));
    assert_eq!(crate::snapshot::snapshot(&b, true).link, Some((key, "javascript:void(0)".to_owned())));
    b.on_event(key, "hover", &json!({ "url": null }));
    assert_eq!(crate::snapshot::snapshot(&b, true).link, None);
    b.on_event(key, "hover", &json!({ "url": "https://a.test/next" }));
    b.on_event(key, "committed", &json!({ "url": "https://a.test/next", "title": "T" }));
    assert_eq!(crate::snapshot::snapshot(&b, true).link, None);
    b.on_event(key, "hover", &json!({ "url": "https://a.test/again" }));
    b.on_event(key, "crashed", &json!({}));
    assert_eq!(crate::snapshot::snapshot(&b, true).link, None);
    b.on_event(key, "hover", &json!({ "url": "https://a.test/blocked" }));
    b.on_event(key, "blocked", &json!({ "url": "https://evil.test/x", "reason": "listed", "category": "malware" }));
    assert!(blocked_page(&b).is_some());
    assert_eq!(crate::snapshot::snapshot(&b, true).link, None, "a blocked page covers the view");
}
