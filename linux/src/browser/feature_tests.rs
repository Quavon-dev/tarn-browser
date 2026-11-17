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
    b.navigate("https://docs.test/typed");
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
    assert_eq!(b.reading_list().len(), 1);
    assert_eq!(b.toast.as_ref().unwrap().text, "Saved to Reading List");
    let id = b.reading_list()[0].id;
    b.remove_from_reading_list(id);
    assert!(b.reading_list().is_empty());
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
fn a_dragged_tab_takes_the_place_of_the_row_it_lands_on() {
    let (_dir, mut b) = browser();
    let a = b.model.active.unwrap();
    let c = b.new_tab("https://c.test/").unwrap();
    let d = b.new_tab("https://d.test/").unwrap();
    let order = |b: &Browser| b.model.ephemeral_keys();
    assert_eq!(order(&b), [a, c, d]);
    b.move_tab_to(d, a);
    assert_eq!(order(&b), [d, a, c], "from below it goes before the row");
    b.move_tab_to(d, c);
    assert_eq!(order(&b), [a, c, d], "from above it goes after the row");
    b.pin_toggle(a);
    b.move_tab_to(c, a);
    assert_eq!(b.model.tab(c).unwrap().kind, TabKind::Pinned, "a row of another zone takes the tab into it");
    b.move_tab_to(d, d);
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
    b.clear_history();
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
