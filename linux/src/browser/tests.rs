use super::*;
use crate::data::StoreKey;

fn browser() -> (tempfile::TempDir, Browser) {
    let dir = tempfile::tempdir().unwrap();
    let b = Browser::open(dir.path());
    (dir, b)
}

fn active(b: &Browser) -> TabKey {
    b.model.active.unwrap()
}

#[test]
fn a_fresh_window_opens_one_blank_tab() {
    let (_dir, b) = browser();
    assert_eq!(b.model.tabs.len(), 1);
    let page = b.active_page().unwrap();
    assert_eq!(page.load.as_ref().map(|l| l.0.as_str()), Some(START_URL));
}

#[test]
fn typed_addresses_load_with_a_new_sequence_and_the_site_level() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.navigate("example.com");
    let first = b.pages[&key].load.clone().unwrap();
    assert_eq!(first.0, "https://example.com/");
    assert!(b.pages[&key].loading, "a load the shell starts shows as loading before it commits");
    b.navigate("example.com");
    assert!(b.pages[&key].load.as_ref().unwrap().1 > first.1, "the same URL loads again");
    b.on_event(key, "committed", &json!({ "url": "https://example.com/", "title": "", "canGoBack": false }));
    b.set_site_level(Some(ShieldLevel::Lockdown));
    assert_eq!(b.pages[&key].level, ShieldLevel::Lockdown);
    assert_eq!(b.levels(key)["example.com"], "lockdown");
    assert_eq!(b.levels(key)["*"], "standard");
    b.set_site_level(None);
    assert_eq!(b.pages[&key].level, ShieldLevel::Standard);
}

#[test]
fn store_links_install_only_from_a_profile_tab_outside_lockdown() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "committed", &json!({ "url": "https://example.com/", "title": "" }));
    assert!(b.store_link_allowed(key));
    b.set_site_level(Some(ShieldLevel::Lockdown));
    assert!(!b.store_link_allowed(key), "from a Lockdown site");
    b.set_site_level(None);
    assert!(b.store_link_allowed(key));
    b.navigate("https://extensions.tarnbrowser.com/install/helper");
    assert!(b.pages[&key].loading);
    b.on_event(key, browser_core::extensions::STORE_EVENT, &json!({ "id": "helper" }));
    assert!(!b.pages[&key].loading, "the cancelled load stops");
    b.on_event(key, "popup", &json!({ "url": "https://extensions.tarnbrowser.com/install/helper" }));
    let popup = active(&b);
    assert_ne!(popup, key);
    assert!(b.pages[&popup].link, "a page's pop-up loads as the page's link, not as the user's own load");
    let view = crate::snapshot::snapshot(&b, true).views.into_iter().find(|v| v.key == popup).unwrap();
    assert!(view.link);
    b.navigate("https://extensions.tarnbrowser.com/install/helper");
    assert!(!b.pages[&popup].link, "the user's own load in that tab is theirs");
    b.activate(key);
    b.private_space();
    let private = active(&b);
    assert!(!b.store_link_allowed(private), "never from the private space");
    assert!(!b.store_link_allowed(key), "only from the space shown");
    b.open_burner();
    assert!(!b.store_link_allowed(active(&b)), "never from a burner tab");
}

#[test]
fn tabs_close_and_reopen_in_their_space() {
    let (_dir, mut b) = browser();
    let first = active(&b);
    let second = b.new_tab("https://b.test/").unwrap();
    b.on_event(second, "finished", &json!({ "url": "https://b.test/", "title": "B" }));
    b.close_tab(second);
    assert_eq!(b.model.tabs.len(), 1);
    assert_eq!(b.model.active, Some(first));
    b.reopen();
    assert_eq!(b.model.tabs.len(), 2);
    let reopened = active(&b);
    assert_eq!(b.pages[&reopened].load.as_ref().unwrap().0, "https://b.test/");
}

#[test]
fn profiles_have_separate_stores_and_the_private_space_is_ephemeral() {
    let (_dir, mut b) = browser();
    let a = b.store_prop(active(&b));
    b.new_profile();
    let other = b.store_prop(active(&b));
    assert!(matches!(StoreKey::parse(&a), Some(StoreKey::Persistent(_))));
    assert!(matches!(StoreKey::parse(&other), Some(StoreKey::Persistent(_))));
    assert_ne!(a, other);
    b.private_space();
    let key = active(&b);
    assert!(key.private);
    assert!(matches!(StoreKey::parse(&b.store_prop(key)), Some(StoreKey::Ephemeral(_))));
    b.on_event(key, "finished", &json!({ "url": "https://secret.test/", "title": "S" }));
    let disk = b.core.disk.profiles().unwrap().iter().flat_map(|p| b.core.disk.history(p.id, 100).unwrap()).count();
    assert_eq!(disk, 0, "private history stays off disk");
}

#[test]
fn spaces_switch_and_tabs_move_between_them() {
    let (_dir, mut b) = browser();
    let key = b.new_tab("https://moved.test/").unwrap();
    b.on_event(key, "committed", &json!({ "url": "https://moved.test/" }));
    let home = b.model.space_ix;
    b.save_space(&b.new_space_draft());
    assert_ne!(b.model.space_ix, home);
    b.switch_space(home);
    let target = b.model.move_targets(&b.core, key)[0];
    b.move_to_space(key, target);
    assert!(b.model.tab(key).is_none());
    b.switch_space(target);
    assert!(b.model.tab(key).is_some());
}

#[test]
fn pins_and_folders_rearrange_tabs() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.pin_toggle(key);
    assert_eq!(b.model.tab(key).unwrap().kind, TabKind::Pinned);
    b.new_folder(None);
    let folder = b.model.folders[0].id;
    b.move_to_folder(key, folder);
    assert_eq!(b.model.tab(key).unwrap().folder_id, Some(folder));
    assert!(b.model.expanded.contains(&folder));
}

#[test]
fn rebuild_reloads_and_other_blocks_show_a_notice() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    let seq = b.pages[&key].load.as_ref().unwrap().1;
    b.on_event(key, "blocked", &json!({ "url": "https://x.test/", "reason": "rebuild" }));
    assert!(b.pages[&key].load.as_ref().unwrap().1 > seq);
    b.on_event(key, "blocked", &json!({ "url": "http://x.test/", "reason": "https_only" }));
    assert!(matches!(b.notice, Some(Notice::Message(ref m)) if m.contains("HTTPS")));
    b.on_event(key, "external", &json!({ "url": "mailto:a@b.test", "origin": "https://x.test" }));
    assert_eq!(b.notice, Some(Notice::External { url: "mailto:a@b.test".into(), origin: "https://x.test".into() }));
}

#[test]
fn open_anyway_loads_a_refused_local_address_as_the_users_own_navigation() {
    use engine::view_policy::{Levels, Navigation, Verdict, navigation};
    let (_dir, mut b) = browser();
    let key = active(&b);
    let public = Url::parse("https://evil.test/").unwrap();
    let target = Url::parse("http://192.168.1.1/admin").unwrap();
    let nav = |from_shell| Navigation {
        url: &target,
        from_shell,
        user_link: false,
        user_step: false,
        page: Some(&public),
        upgraded: false,
        likely_subframe: false,
        file_open: false,
    };
    assert_eq!(navigation(&Levels::default(), ShieldLevel::Standard, &nav(false)), Verdict::Blocked("local_network"));
    b.on_event(key, "committed", &json!({ "url": public.as_str() }));
    b.on_event(key, "blocked", &json!({ "url": target.as_str(), "reason": "local_network" }));
    assert!(!b.pages[&key].loading);
    let Some(Notice::Open { key: offered, ref url, ref message }) = b.notice else { panic!("{:?}", b.notice) };
    assert_eq!((offered, url.as_str()), (key, target.as_str()));
    assert!(message.ends_with("192.168.1.1/admin"));
    let seq = b.pages[&key].load.as_ref().unwrap().1;
    b.open_anyway();
    let load = b.pages[&key].load.clone().unwrap();
    assert!(load.0 == target.as_str() && load.1 > seq, "a shell load, which the view marks from_shell");
    assert_eq!(b.notice, None);
    assert_eq!(navigation(&Levels::default(), ShieldLevel::Standard, &nav(true)), Verdict::Allow { javascript: true });
    b.open_anyway();
    assert_eq!(b.pages[&key].load.as_ref().unwrap().1, load.1, "the offer is used once");

    b.on_event(key, "blocked", &json!({ "url": "http://u:p@10.0.0.1/", "reason": "local_network" }));
    assert!(matches!(b.notice, Some(Notice::Message(_))), "no offer for an address with credentials");
    b.on_event(key, "blocked", &json!({ "url": target.as_str(), "reason": "local_network" }));
    b.on_event(key, "committed", &json!({ "url": "https://next.test/" }));
    assert_eq!(b.notice, None, "the offer goes with the next page");

    b.on_event(key, "blocked", &json!({ "url": "https://tracker.example/pixel", "reason": "local_network" }));
    assert!(matches!(b.notice, Some(Notice::Message(_))), "only an address on the local network");
    b.on_event(key, "blocked", &json!({ "url": "https://tracker.example/pixel", "reason": "shield" }));
    assert!(matches!(b.notice, Some(Notice::Message(_))));
}

#[test]
fn only_open_file_loads_a_file_url_from_the_shell() {
    use engine::view_policy::{Levels, Navigation, Verdict, navigation};
    let (dir, mut b) = browser();
    let path = dir.path().join("page.html");
    std::fs::write(&path, "<p>local</p>").unwrap();
    let url = browser_core::open_file::url_for(&path).unwrap();
    b.open_file(&url);
    let key = active(&b);
    let view = crate::snapshot::snapshot(&b, true).views.into_iter().find(|v| v.key == key).unwrap();
    assert_eq!((view.load.unwrap().0, view.file, view.link), (url.to_string(), true, false));
    let tab = b.model.tab(key).cloned().unwrap();
    assert_eq!(crate::snapshot::tab_label(&tab, &b, key), "page.html", "a file without a title shows its name");
    let nav = |from_shell, file_open| Navigation {
        url: &url,
        from_shell,
        user_link: false,
        user_step: false,
        page: None,
        upgraded: false,
        likely_subframe: false,
        file_open,
    };
    let verdict = |from_shell, file_open| navigation(&Levels::default(), ShieldLevel::Standard, &nav(from_shell, file_open));
    assert_eq!(verdict(true, true), Verdict::Allow { javascript: true }, "the shell's Open File load");
    assert_eq!(verdict(true, false), Verdict::Blocked("nav_policy"), "a plain shell load");
    assert_eq!(verdict(false, true), Verdict::Blocked("nav_policy"), "a page's link");
    assert_eq!(verdict(false, false), Verdict::Blocked("nav_policy"));
    b.unload(key);
    b.load(key, url.as_str());
    assert!(b.pages[&key].file, "a slept tab loads its opened file again as Open File");
    b.load(key, "https://a.test/");
    assert!(!b.pages[&key].file, "any other load of the tab is a plain one");
}

#[test]
fn a_blocked_popup_keeps_the_opener_loading_and_offers_a_new_tab() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "committed", &json!({ "url": "https://evil.test/" }));
    assert!(b.pages[&key].loading);
    b.on_event(key, "blocked", &json!({ "url": "http://192.168.1.1/", "reason": "shield", "popup": true }));
    assert!(b.pages[&key].loading);
    assert_eq!(b.notice, Some(Notice::NewTab { key, url: "http://192.168.1.1/".into(), message: "Blocked a pop-up window to 192.168.1.1.".into() }));
}

#[test]
fn a_crashed_page_reloads_once_until_a_page_finishes() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    let seq = b.pages[&key].load.as_ref().unwrap().1;
    b.on_event(key, "crashed", &Value::Null);
    let after = b.pages[&key].load.as_ref().unwrap().1;
    assert!(after > seq);
    b.on_event(key, "crashed", &Value::Null);
    assert_eq!(b.pages[&key].load.as_ref().unwrap().1, after);
    assert!(b.pages[&key].error.is_some());
    assert!(!b.pages[&key].loading, "a crash that is not reloaded ends the load");
    b.on_event(key, "finished", &json!({ "url": "https://a.test/", "title": "A" }));
    b.on_event(key, "crashed", &Value::Null);
    assert!(b.pages[&key].load.as_ref().unwrap().1 > after, "a page that finished loading reloads after its next crash");
}

#[test]
fn prompts_queue_and_remembered_permissions_answer_themselves() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    let ask = |id: u64| json!({ "id": id, "kind": "camera", "origin": "https://cam.test" });
    b.on_event(key, "prompt", &ask(1));
    assert_eq!(b.pages[&key].prompts.len(), 1);
    b.answer(1, true, true, "", "", "");
    assert_eq!(b.outbox.pop().unwrap().2["accept"], true);
    b.on_event(key, "prompt", &ask(2));
    assert!(b.pages[&key].prompts.is_empty(), "answered from the remembered choice");
    assert_eq!(b.outbox.pop().unwrap().2, json!({ "id": 2, "accept": true }));
    b.on_event(key, "prompt", &json!({ "id": 3, "kind": "credentials", "origin": "https://a.test", "realm": "r" }));
    b.answer(3, true, true, "", "u", "p");
    let (_, name, args) = b.outbox.pop().unwrap();
    assert_eq!((name, args["user"].as_str()), ("answer", Some("u")));
    assert!(b.core.disk.permissions(b.model.profile().unwrap().1).unwrap().iter().all(|(_, k, _)| k != "credentials"));
}

#[test]
fn a_site_prompt_answer_leaves_the_page_s_own_prompts_alone() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "prompt", &json!({ "id": 1, "kind": "camera", "origin": "https://cam.test" }));
    b.answer_site_prompt(1, true, true);
    assert_eq!(b.pages[&key].prompts.len(), 1, "a page's prompt is not an extension's");
    assert!(b.outbox.is_empty());
}

#[test]
fn an_extension_opens_tabs_only_in_its_own_profiles_persistent_space() {
    let (_dir, mut b) = browser();
    let first = b.model.profile().unwrap().1;
    b.new_profile();
    let second = b.model.profile().unwrap().1;
    let open = |profile| crate::extensions::Note::OpenTab { profile, url: Url::parse("https://a.test/").unwrap(), active: true };
    let tabs = b.model.tabs.len();
    assert!(!b.on_extension_note(&open(first)), "not in another profile's space");
    assert_eq!(b.model.tabs.len(), tabs);
    assert!(b.on_extension_note(&open(second)));
    assert_eq!(b.model.tabs.len(), tabs + 1);
    b.private_space();
    let tabs = b.model.tabs.len();
    assert!(!b.on_extension_note(&open(second)), "never in the private space");
    assert_eq!(b.model.tabs.len(), tabs);
}

#[test]
fn a_permission_the_component_cannot_place_in_a_frame_is_never_remembered() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    let ask = |id: u64| json!({ "id": id, "kind": "location", "origin": "https://geo.test", "topOrigin": "" });
    b.on_event(key, "prompt", &ask(1));
    b.answer(1, true, true, "", "", "");
    b.outbox.clear();
    b.on_event(key, "prompt", &ask(2));
    assert_eq!(b.pages[&key].prompts.len(), 1, "asked again");
    assert!(b.outbox.is_empty());
}

#[test]
fn popups_open_as_new_tabs_and_downloads_are_listed() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "popup", &json!({ "url": "https://popup.test/" }));
    assert_eq!(b.model.tabs.len(), 2);
    let path = "/home/u/Downloads/a.pdf";
    b.on_download(key, &json!({ "id": "1", "state": "started", "name": "a.pdf", "path": path, "pausable": false }), true);
    b.on_download(key, &json!({ "id": "1", "state": "progress", "done": 50, "total": 200 }), true);
    assert_eq!((b.downloads[0].done, b.downloads[0].total), (50, Some(200)));
    assert_eq!(crate::snapshot::snapshot(&b, true).downloading, Some(0.25));
    b.on_download(key, &json!({ "id": "1", "state": "finished", "name": "a.pdf", "path": path, "url": "https://d.test/a.pdf" }), true);
    assert_eq!(b.downloads.len(), 1);
    assert_eq!(b.downloads[0].state, "finished");
    assert_eq!(crate::snapshot::snapshot(&b, true).downloading, None);
    assert_eq!(b.core.disk.downloads(b.model.profile().unwrap().1).unwrap()[0].path, path);
}

#[test]
fn zoom_steps_and_is_kept_per_site() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "committed", &json!({ "url": "https://z.test/" }));
    b.zoom(1);
    assert_eq!(b.pages[&key].zoom, 1.1);
    let profile = b.model.profile().unwrap().1;
    assert_eq!(b.core.disk.site_zoom(profile, "z.test").unwrap(), Some(1.1));
    b.zoom(0);
    assert_eq!(b.core.disk.site_zoom(profile, "z.test").unwrap(), None);
}

#[test]
fn the_window_reopens_on_its_space_and_tab() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = {
        let mut b = Browser::open(dir.path());
        let key = b.new_tab("https://keep.test/").unwrap();
        b.new_tab("https://other.test/");
        b.activate(key);
        key
    };
    let b = Browser::open(dir.path());
    assert_eq!(b.model.active, Some(wanted));
    assert_eq!(b.active_page().unwrap().load.as_ref().unwrap().0, "https://keep.test/");
}

#[test]
fn only_the_most_recent_tabs_stay_live() {
    let (_dir, mut b) = browser();
    let first = active(&b);
    for n in 0..LIVE_TABS {
        b.new_tab(&format!("https://t{n}.test/"));
    }
    assert!(b.model.lifecycle.live().all(|k| k != first));
    assert!(b.pages[&first].load.is_none(), "a sleeping tab loads again when shown");
    b.activate(first);
    assert!(b.pages[&first].load.is_some());
}

#[test]
fn settings_switch_lists_engine_and_level() {
    let (_dir, mut b) = browser();
    assert_eq!(b.lists().len(), shield::groups::SETTINGS.len());
    b.toggle_setting("block_ads");
    assert_eq!(b.lists(), ["trackers", "malware", "annoyances"]);
    b.set_search_engine(2);
    assert_eq!(b.search_engine(), 2);
    b.set_default_level(ShieldLevel::Strict);
    assert_eq!(b.default_level(), ShieldLevel::Strict);
    assert_eq!(b.pages[&active(&b)].level, ShieldLevel::Strict);
}

#[test]
fn a_blocked_popup_opens_in_a_new_tab_only_on_the_users_offer_and_once() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    let blocked = json!({ "url": "https://docs.test/win", "reason": "shield", "popup": true });
    for _ in 0..3 {
        b.on_event(key, "blocked", &blocked);
    }
    assert_eq!(b.model.tabs.len(), 1, "the page alone never opens it");
    let Some(Notice::NewTab { key: opener, ref url, ref message }) = b.notice else { panic!("{:?}", b.notice) };
    assert_eq!((opener, url.as_str(), message.as_str()), (key, "https://docs.test/win", "Blocked a pop-up window to docs.test."));
    b.open_blocked_popup();
    assert_eq!(b.model.tabs.len(), 2);
    assert_eq!(b.active_page().unwrap().load.as_ref().unwrap().0, "https://docs.test/win");
    assert_eq!(b.notice, None);
    b.open_blocked_popup();
    assert_eq!(b.model.tabs.len(), 2, "the offer is used once");

    for url in ["javascript:alert(1)", "file:///etc/passwd", "not a url"] {
        b.on_event(key, "blocked", &json!({ "url": url, "reason": "shield", "popup": true }));
        assert!(matches!(b.notice, Some(Notice::Message(_))), "no offer for {url}");
    }

    b.private_space();
    let private = active(&b);
    b.on_event(private, "blocked", &blocked);
    b.switch_space_by_user(0);
    let tabs = b.model.tabs.len();
    b.open_blocked_popup();
    assert_eq!(b.model.tabs.len(), tabs, "a private tab's pop-up never opens in another space");
}

#[test]
fn tiles_leave_the_rows_for_the_tile_grid_and_come_back() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.tile_toggle(key);
    assert_eq!(b.model.tab(key).unwrap().kind, TabKind::Tile);
    assert!(crate::snapshot::rows(&b).iter().all(|r| r.kind != crate::snapshot::RowKind::Tab(key)));
    assert_eq!(crate::snapshot::tiles(&b).iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), [crate::snapshot::RowKind::Tab(key)]);
    b.tile_toggle(key);
    assert_eq!(b.model.tab(key).unwrap().kind, TabKind::Ephemeral);
    assert!(crate::snapshot::tiles(&b).is_empty());
}

#[test]
fn rows_of_tabs_without_a_live_view_are_asleep() {
    let (_dir, mut b) = browser();
    let first = active(&b);
    let second = b.new_tab("https://two.test/").unwrap();
    let asleep = |b: &Browser, key| crate::snapshot::rows(b).into_iter().find(|r| r.kind == crate::snapshot::RowKind::Tab(key)).unwrap().asleep;
    assert!(!asleep(&b, first) && !asleep(&b, second));
    b.sleep_tab(first);
    assert!(asleep(&b, first) && !asleep(&b, second));
    b.activate(first);
    assert!(!asleep(&b, first));
}

#[test]
fn a_playing_tab_stays_awake_beyond_the_live_tab_limit() {
    let (_dir, mut b) = browser();
    let first = active(&b);
    b.on_event(first, "playing", &json!({ "playing": true }));
    for n in 0..LIVE_TABS {
        b.new_tab(&format!("https://t{n}.test/"));
    }
    assert!(b.model.lifecycle.live().any(|k| k == first));
    assert!(b.pages[&first].load.is_some());
    b.on_event(first, "playing", &json!({ "playing": false }));
    b.new_tab("https://next.test/");
    assert!(b.model.lifecycle.live().all(|k| k != first));
}

#[test]
fn idle_background_tabs_sleep_after_half_an_hour_or_a_minute_when_memory_is_short() {
    use std::time::{Duration, Instant};
    let (_dir, mut b) = browser();
    let pinned = active(&b);
    b.pin_toggle(pinned);
    let playing = b.new_tab("https://play.test/").unwrap();
    b.on_event(playing, "playing", &json!({ "playing": true }));
    let idle = b.new_tab("https://idle.test/").unwrap();
    let shown = b.new_tab("https://shown.test/").unwrap();
    let start = Instant::now();
    let live = |b: &Browser, key| b.model.lifecycle.live().any(|k| k == key);
    b.sleep_idle_tabs(start + Duration::from_secs(90), false);
    assert!(live(&b, idle), "a minute is not enough while memory is plentiful");
    b.sleep_idle_tabs(start + Duration::from_secs(90), true);
    assert!(!live(&b, idle) && b.pages[&idle].load.is_none());
    assert!(live(&b, pinned) && live(&b, playing) && live(&b, shown), "pinned, playing and shown tabs stay awake");
    b.activate(idle);
    b.activate(shown);
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(29 * 60), false);
    assert!(live(&b, idle));
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(31 * 60), false);
    assert!(!live(&b, idle) && live(&b, shown));
}

#[test]
fn a_tab_whose_view_went_with_its_space_is_no_longer_playing_nor_overdue() {
    use std::time::{Duration, Instant};
    let (_dir, mut b) = browser();
    let first = active(&b);
    b.on_event(first, "playing", &json!({ "playing": true }));
    let later = b.new_tab("https://later.test/").unwrap();
    b.save_space(&b.new_space_draft());
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(40 * 60), false);
    assert!(!b.playing.contains(&first), "its view is gone, so it no longer plays");
    b.switch_space_by_user(0);
    b.activate(later);
    b.sleep_idle_tabs(Instant::now() + Duration::from_secs(41 * 60), false);
    assert!(b.model.lifecycle.live().any(|k| k == first), "the idle time starts again when its space is shown");
}

#[test]
fn a_page_cannot_swap_an_offer_for_its_own() {
    let (_dir, mut b) = browser();
    let key = active(&b);
    b.on_event(key, "external", &json!({ "url": "mailto:a@b.test", "origin": "https://x.test" }));
    b.on_event(key, "blocked", &json!({ "url": "https://ads.test/", "reason": "shield", "popup": true }));
    assert!(matches!(b.notice, Some(Notice::External { .. })));
}
