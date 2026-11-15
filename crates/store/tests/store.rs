use shield::ShieldLevel;
use store::WebEngine::{Chromium, WebKit};
use store::{Store, TabKind, WebEngine};

fn seeded() -> (Store, i64) {
    let store = Store::open_in_memory().unwrap();
    let profile = store.create_profile("Personal", "uuid-a", false).unwrap();
    let space = store.create_space(profile.id, "Home", "house").unwrap();
    (store, space.id)
}

#[test]
fn roundtrip_and_ordering() {
    let (store, space) = seeded();
    let a = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap();
    let b = store.create_tab(space, TabKind::Ephemeral, None, "https://b.test/", "B").unwrap();
    store.set_interaction_state(a.id, Some(&[1, 2, 3])).unwrap();
    let tabs = store.tabs(space).unwrap();
    assert_eq!(tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [a.id, b.id]);
    assert_eq!(store.interaction_state(a.id).unwrap().as_deref(), Some(&[1u8, 2, 3][..]));
    assert_eq!(store.interaction_state(b.id).unwrap(), None);
    assert_eq!((a.position, b.position), (0, 1));
}

#[test]
fn move_tab_renumbers_and_drop_on_folder_pins() {
    let (store, space) = seeded();
    let folder = store.create_folder(space, None, "Work").unwrap();
    let a = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap();
    let b = store.create_tab(space, TabKind::Ephemeral, None, "https://b.test/", "B").unwrap();
    let c = store.create_tab(space, TabKind::Ephemeral, None, "https://c.test/", "C").unwrap();
    store.move_tab(b.id, TabKind::Pinned, Some(folder.id), 0).unwrap();
    let moved = store.tab(b.id).unwrap();
    assert_eq!((moved.kind, moved.folder_id, moved.position), (TabKind::Pinned, Some(folder.id), 0));
    assert_eq!(store.tab(c.id).unwrap().position, 1);
    store.move_tab(c.id, TabKind::Ephemeral, None, 0).unwrap();
    assert_eq!((store.tab(c.id).unwrap().position, store.tab(a.id).unwrap().position), (0, 1));
}

#[test]
fn a_tab_never_moves_into_another_spaces_folder() {
    let (store, space) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    let other = store.create_space(profile, "Other", "").unwrap().id;
    let elsewhere = store.create_folder(other, None, "Elsewhere").unwrap();
    let t = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
    assert!(store.move_tab(t.id, TabKind::Pinned, Some(elsewhere.id), 0).is_err());
    let tab = store.tab(t.id).unwrap();
    assert_eq!((tab.kind, tab.folder_id), (TabKind::Ephemeral, None));
}

#[test]
fn folder_constraint_rejects_unpinned_tab_in_folder() {
    let (store, space) = seeded();
    let folder = store.create_folder(space, None, "Work").unwrap();
    assert!(store.create_tab(space, TabKind::Ephemeral, Some(folder.id), "https://a.test/", "").is_err());
    let t = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
    assert!(store.move_tab(t.id, TabKind::Tile, Some(folder.id), 0).is_err());
    assert_eq!(store.tab(t.id).unwrap().kind, TabKind::Ephemeral);
}

#[test]
fn folders_nest_without_cycles() {
    let (store, space) = seeded();
    let other = store.create_space(store.profiles().unwrap()[0].id, "Work", "").unwrap();
    let outer = store.create_folder(space, None, "Outer").unwrap();
    let inner = store.create_folder(space, Some(outer.id), "Inner").unwrap();
    let deep = store.create_folder(space, Some(inner.id), "Deep").unwrap();
    let loose = store.create_folder(space, None, "Loose").unwrap();
    let foreign = store.create_folder(other.id, None, "Foreign").unwrap();
    assert!(store.move_folder(outer.id, Some(outer.id)).is_err());
    assert!(store.move_folder(outer.id, Some(deep.id)).is_err());
    assert!(store.move_folder(loose.id, Some(foreign.id)).is_err());
    store.move_folder(loose.id, Some(inner.id)).unwrap();
    store.move_folder(deep.id, None).unwrap();
    let parent = |id| store.folders(space).unwrap().into_iter().find(|f| f.id == id).unwrap().parent_id;
    assert_eq!((parent(loose.id), parent(deep.id), parent(outer.id)), (Some(inner.id), None, None));
    store.create_tab(space, TabKind::Pinned, Some(loose.id), "https://a.test/", "").unwrap();
    store.delete_folder(outer.id).unwrap();
    assert!(store.tabs(space).unwrap().is_empty());
    assert_eq!(store.folders(space).unwrap().iter().map(|f| f.id).collect::<Vec<_>>(), [deep.id]);
}

#[test]
fn cascade_delete() {
    let (store, space) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    let folder = store.create_folder(space, None, "Work").unwrap();
    store.create_folder(space, Some(folder.id), "Sub").unwrap();
    store.create_tab(space, TabKind::Pinned, Some(folder.id), "https://a.test/", "").unwrap();
    store.add_history(profile, "https://a.test/", "", 1, false).unwrap();
    store.add_download(profile, "https://a.test/f.pdf", "/tmp/f.pdf", "finished", 1).unwrap();
    store.delete_folder(folder.id).unwrap();
    assert!(store.tabs(space).unwrap().is_empty());
    assert!(store.folders(space).unwrap().is_empty());
    store.delete_profile(profile).unwrap();
    assert!(store.spaces(profile).unwrap().is_empty());
    assert!(store.history(profile, 10).unwrap().is_empty());
}

#[test]
fn history_dedupes_newest_first_per_profile() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.add_history(p, "https://a.test/", "A", 1, false).unwrap();
    store.add_history(p, "https://b.test/", "B", 2, false).unwrap();
    store.add_history(p, "https://a.test/", "A", 3, true).unwrap();
    store.add_history(other, "https://secret.test/", "S", 4, true).unwrap();
    let urls: Vec<_> = store.history(p, 10).unwrap().into_iter().map(|h| (h.url, h.visits, h.typed)).collect();
    assert_eq!(urls, [("https://a.test/".to_owned(), 2, 1), ("https://b.test/".to_owned(), 1, 0)]);
}

#[test]
fn history_text_is_searched_per_profile_and_cleared_with_history() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    let old = store.add_history(p, "https://a.test/", "A", 1, false).unwrap();
    store.set_history_text(p, old, "https://a.test/", "stale words").unwrap();
    let visit = store.add_history(p, "https://a.test/", "A", 2, false).unwrap();
    store.set_history_text(p, visit, "https://a.test/", "Crème brûlée recipes and caramel").unwrap();
    let b = store.add_history(p, "https://b.test/", "B", 3, false).unwrap();
    store.set_history_text(p, b, "https://b.test/", "caramel sauce").unwrap();
    let theirs = store.add_history(other, "https://secret.test/", "S", 4, false).unwrap();
    store.set_history_text(other, theirs, "https://secret.test/", "caramel secrets").unwrap();
    assert!(store.set_history_text(p, theirs, "https://secret.test/", "x").is_ok());
    assert!(store.set_history_text(p, visit, "https://a.test/", &"x".repeat(store::MAX_HISTORY_TEXT + 1)).is_err());

    let urls = |q: &str| store.search_history_text(p, q, 10).unwrap().into_iter().map(|h| h.url).collect::<Vec<_>>();
    assert_eq!(urls("creme BRUL"), ["https://a.test/"]);
    store.add_history(other, "https://a.test/", "A", 5, true).unwrap();
    store.add_history(p, "https://a.test/", "A", 6, true).unwrap();
    let visits = |q: &str| store.search_history_text(p, q, 10).unwrap().into_iter().map(|h| (h.visits, h.typed)).collect::<Vec<_>>();
    assert_eq!(visits("creme"), [(3, 1)], "every visit of the URL in this profile counts");
    let mut both = urls("caramel");
    both.sort();
    assert_eq!(both, ["https://a.test/", "https://b.test/"]);
    assert!(urls("stale").is_empty(), "an earlier visit's text is replaced");
    assert!(urls("secrets").is_empty());
    for hostile in ["\"", "*", "a OR b", "NEAR(caramel", "text:caramel\"", "^caramel", "-- ;", "caramel AND", ""] {
        assert!(store.search_history_text(p, hostile, 10).is_ok(), "{hostile}");
    }
    assert_eq!(urls("\"caramel\" OR"), Vec::<String>::new(), "OR is a word, not an operator");

    store.clear_history(p).unwrap();
    assert!(urls("caramel").is_empty());
    assert_eq!(store.search_history_text(other, "caramel", 10).unwrap().len(), 1);
    store.add_history(other, "https://reused.test/", "R", 5, false).unwrap();
    store.delete_profile(other).unwrap();
    let fresh = store.create_profile("New", "uuid-c", false).unwrap().id;
    store.add_history(fresh, "https://secret.test/", "S", 6, false).unwrap();
    assert!(store.search_history_text(fresh, "secrets", 10).unwrap().is_empty(), "text goes with its visit");
}

#[test]
fn bulk_history_skips_visits_already_recorded() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.add_history(other, "https://a.test/", "A", 1, false).unwrap();
    let visits = [("https://a.test/", "A", 1), ("https://a.test/", "A", 2), ("https://b.test/", "B", 1)];
    assert_eq!(store.add_history_bulk(p, visits).unwrap(), 3, "another profile's visit is not a duplicate");
    assert_eq!(store.add_history_bulk(p, visits).unwrap(), 0);
    assert_eq!(store.history(p, 10).unwrap().len(), 2);
}

#[test]
fn a_failed_batch_leaves_nothing_behind() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let failed = store.batch(|s| {
        s.add_bookmark(p, None, None, "Folder", 1)?;
        s.add_bookmark(p, Some(9999), Some("https://a.test/"), "A", 1)
    });
    assert!(failed.is_err());
    assert!(store.bookmarks(p).unwrap().is_empty());
    store.batch(|s| s.add_bookmark(p, None, None, "Folder", 1)).unwrap();
    assert_eq!(store.bookmarks(p).unwrap().len(), 1);
}

#[test]
fn overrides_and_permissions() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    assert_eq!(store.site_override(p, "a.test").unwrap(), None);
    store.set_site_override(p, "a.test", Some(ShieldLevel::Lockdown)).unwrap();
    store.set_site_override(p, "a.test", Some(ShieldLevel::Strict)).unwrap();
    assert_eq!(store.site_override(p, "a.test").unwrap(), Some(ShieldLevel::Strict));
    assert_eq!(store.site_overrides(p).unwrap(), [("a.test".to_owned(), ShieldLevel::Strict)]);
    store.set_site_override(p, "a.test", None).unwrap();
    assert_eq!(store.site_override(p, "a.test").unwrap(), None);
    store.set_permission(p, "https://a.test", "camera", "deny").unwrap();
    assert_eq!(store.permission(p, "https://a.test", "camera").unwrap().as_deref(), Some("deny"));
}

#[test]
fn file_permissions_and_private_rows_absent_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("profile-data");
    {
        let disk = Store::open(&data).unwrap();
        let p = disk.create_profile("Personal", "uuid-a", false).unwrap();
        let s = disk.create_space(p.id, "Home", "").unwrap();
        disk.create_tab(s.id, TabKind::Ephemeral, None, "https://public.test/", "").unwrap();

        let private = Store::open_in_memory().unwrap();
        let pp = private.create_profile("Private", "uuid-p", true).unwrap();
        let ps = private.create_space(pp.id, "Private", "").unwrap();
        private.create_tab(ps.id, TabKind::Ephemeral, None, "https://private-marker.test/", "").unwrap();
        private.add_history(pp.id, "https://private-marker.test/", "", 1, false).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&data.join("tarn.db")), 0o600);
    }
    let mut found_public = false;
    for entry in std::fs::read_dir(&data).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let hay = String::from_utf8_lossy(&bytes);
        assert!(!hay.contains("private-marker"));
        found_public |= hay.contains("public.test");
    }
    assert!(found_public);
}

#[test]
fn reopen_keeps_data_and_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    {
        let s = Store::open(dir.path()).unwrap();
        s.create_profile("Personal", "uuid-a", false).unwrap();
    }
    let s = Store::open(dir.path()).unwrap();
    assert_eq!(s.profiles().unwrap().len(), 1);
}

#[test]
fn favicons_are_per_profile_and_cleared_with_history() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_favicon(p, "a.test", b"one").unwrap();
    store.set_favicon(p, "a.test", b"two").unwrap();
    store.set_favicon(other, "a.test", b"other").unwrap();
    assert_eq!(store.favicons(p).unwrap(), [("a.test".to_owned(), b"two".to_vec())]);
    store.clear_history(p).unwrap();
    assert!(store.favicons(p).unwrap().is_empty());
    assert_eq!(store.favicons(other).unwrap(), [("a.test".to_owned(), b"other".to_vec())]);
}

#[test]
fn settings_roundtrip() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.setting("filter_updates").unwrap(), None);
    s.set_setting("filter_updates", "off").unwrap();
    s.set_setting("filter_updates", "on").unwrap();
    assert_eq!(s.setting("filter_updates").unwrap().as_deref(), Some("on"));
}

#[test]
fn export_and_reset_touch_only_one_profile() {
    let (store, space) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    let other_space = store.create_space(other, "Work", "").unwrap().id;
    let folder = store.create_folder(space, None, "F").unwrap();
    store.create_tab(space, TabKind::Pinned, Some(folder.id), "https://mine.test/", "M").unwrap();
    store.create_tab(other_space, TabKind::Ephemeral, None, "https://theirs.test/", "T").unwrap();
    let visit = store.add_history(p, "https://mine.test/", "M", 1, false).unwrap();
    store.set_history_text(p, visit, "https://mine.test/", "minetext").unwrap();
    store.add_history(other, "https://theirs.test/", "T", 1, false).unwrap();
    store.set_favicon(p, "mine.test", &[0xab, 0x01]).unwrap();
    store.set_site_override(p, "mine.test", Some(ShieldLevel::Strict)).unwrap();
    store.set_permission(p, "https://mine.test", "camera", "deny").unwrap();

    let json = store.export_profile(p).unwrap();
    assert!(json.contains("mine.test") && json.contains("\"ab01\"") && json.contains("camera"));
    assert!(json.contains(&format!("\"history_id\": {visit}")) && json.contains("minetext"));
    assert!(!json.contains("theirs.test"));

    let fresh = store.reset_profile(p, "Home").unwrap();
    assert_eq!(store.spaces(p).unwrap(), std::slice::from_ref(&fresh));
    assert!(store.tabs(fresh.id).unwrap().is_empty() && store.folders(fresh.id).unwrap().is_empty());
    assert!(store.history(p, 10).unwrap().is_empty() && store.favicons(p).unwrap().is_empty());
    assert!(store.search_history_text(p, "minetext", 10).unwrap().is_empty());
    assert_eq!(store.site_override(p, "mine.test").unwrap(), None);
    assert_eq!(store.permission(p, "https://mine.test", "camera").unwrap(), None);
    assert_eq!(store.profile(p).unwrap().name, "Personal");
    assert_eq!(store.tabs(other_space).unwrap().len(), 1);
    assert_eq!(store.history(other, 10).unwrap().len(), 1);
}

#[test]
fn deleted_rows_leave_no_trace_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    let p = s.create_profile("Personal", "uuid-a", false).unwrap().id;
    let space = s.create_space(p, "Home", "").unwrap().id;
    s.create_tab(space, TabKind::Ephemeral, None, "https://tab-marker.test/", "T").unwrap();
    let visit = s.add_history(p, "https://history-marker.test/", "H", 1, false).unwrap();
    s.set_history_text(p, visit, "https://history-marker.test/", "pagetextmarker").unwrap();
    s.clear_history(p).unwrap();
    s.reset_profile(p, "Home").unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let hay = String::from_utf8_lossy(&std::fs::read(entry.unwrap().path()).unwrap()).into_owned();
        assert!(!hay.contains("history-marker") && !hay.contains("tab-marker") && !hay.contains("pagetextmarker"));
    }
}

#[test]
fn forgotten_permissions_leave_no_trace_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    let p = s.create_profile("Personal", "uuid-a", false).unwrap().id;
    s.set_permission(p, "https://permission-marker.test", "camera", "deny").unwrap();
    s.delete_permission(p, "https://permission-marker.test", "camera").unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let hay = String::from_utf8_lossy(&std::fs::read(entry.unwrap().path()).unwrap()).into_owned();
        assert!(!hay.contains("permission-marker"));
    }
}

#[test]
fn deleted_profiles_and_spaces_leave_no_trace_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    let keep = s.create_profile("Personal", "uuid-a", false).unwrap().id;
    s.create_space(keep, "Home", "").unwrap();
    let gone = s.create_profile("Work", "uuid-b", false).unwrap().id;
    let space = s.create_space(gone, "Home", "").unwrap().id;
    s.create_tab(space, TabKind::Ephemeral, None, "https://profile-tab-marker.test/", "T").unwrap();
    s.add_history(gone, "https://profile-history-marker.test/", "H", 1, false).unwrap();
    let extra = s.create_space(keep, "Extra", "").unwrap().id;
    s.create_tab(extra, TabKind::Ephemeral, None, "https://space-tab-marker.test/", "T").unwrap();
    s.delete_space(extra).unwrap();
    s.delete_profile(gone).unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let hay = String::from_utf8_lossy(&std::fs::read(entry.unwrap().path()).unwrap()).into_owned();
        assert!(!hay.contains("profile-history-marker") && !hay.contains("profile-tab-marker") && !hay.contains("space-tab-marker"));
    }
}

#[test]
fn developer_flag_roundtrips() {
    let s = Store::open_in_memory().unwrap();
    assert!(!s.create_profile("Plain", "uuid-d", false).unwrap().is_dev);
    let (dev, space) = s.create_dev_profile("uuid-e").unwrap();
    assert!(dev.is_dev && dev.name == "Developer");
    assert!(s.profile(dev.id).unwrap().is_dev);
    assert_eq!(s.spaces(dev.id).unwrap(), [space]);
}

#[test]
fn groups_hold_unpinned_tabs_of_their_space_and_close_with_them() {
    let (store, space) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    let other = store.create_space(profile, "Work", "").unwrap();
    let group = store.create_group(space, "Group 1").unwrap();
    let a = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
    let pinned = store.create_tab(space, TabKind::Pinned, None, "https://p.test/", "").unwrap();
    let elsewhere = store.create_tab(other.id, TabKind::Ephemeral, None, "https://b.test/", "").unwrap();
    store.set_tab_group(a.id, Some(group.id)).unwrap();
    assert!(store.set_tab_group(pinned.id, Some(group.id)).is_err());
    assert!(store.set_tab_group(elsewhere.id, Some(group.id)).is_err());
    assert_eq!(store.tab(a.id).unwrap().group_id, Some(group.id));
    store.update_group(group.id, "Research", true).unwrap();
    let g = &store.groups(space).unwrap()[0];
    assert_eq!((g.name.as_str(), g.collapsed), ("Research", true));
    store.move_tab(a.id, TabKind::Pinned, None, 0).unwrap();
    assert_eq!(store.tab(a.id).unwrap().group_id, None);
    store.move_tab(a.id, TabKind::Ephemeral, None, 0).unwrap();
    store.set_tab_group(a.id, Some(group.id)).unwrap();
    store.delete_group(group.id).unwrap();
    assert!(store.tab(a.id).is_err());
    assert!(store.tab(pinned.id).is_ok());
}

#[test]
fn ungroup_keeps_the_tabs() {
    let (store, space) = seeded();
    let group = store.create_group(space, "Group 1").unwrap();
    let a = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
    store.set_tab_group(a.id, Some(group.id)).unwrap();
    store.ungroup(group.id).unwrap();
    assert!(store.groups(space).unwrap().is_empty());
    assert_eq!(store.tab(a.id).unwrap().group_id, None);
}

#[test]
fn settings_can_be_deleted() {
    let (store, _) = seeded();
    store.set_setting("search_engine.1", "DuckDuckGo").unwrap();
    store.delete_setting("search_engine.1").unwrap();
    assert_eq!(store.setting("search_engine.1").unwrap(), None);
}

#[test]
fn bookmarks_nest_only_in_folders_of_the_same_profile() {
    let (store, _) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap();
    let folder = store.add_bookmark(profile, None, None, "Reading", 1).unwrap();
    let mark = store.add_bookmark(profile, Some(folder.id), Some("https://a.test/"), "A", 2).unwrap();
    assert_eq!((mark.parent_id, mark.position), (Some(folder.id), 0));
    assert!(store.add_bookmark(profile, Some(mark.id), Some("https://b.test/"), "B", 3).is_err());
    assert!(store.add_bookmark(other.id, Some(folder.id), Some("https://b.test/"), "B", 3).is_err());
    store.rename_bookmark(mark.id, "Alpha").unwrap();
    assert_eq!(store.bookmarks(profile).unwrap()[1].title, "Alpha");
    assert!(store.bookmarks(other.id).unwrap().is_empty());
    store.delete_bookmark(folder.id).unwrap();
    assert!(store.bookmarks(profile).unwrap().is_empty());
}

#[test]
fn groups_keep_a_colour_and_reorder() {
    let (store, space) = seeded();
    let [a, b, c] = ["A", "B", "C"].map(|n| store.create_group(space, n).unwrap().id);
    assert_eq!(store.groups(space).unwrap()[0].color, "");
    store.set_group_color(b, "green").unwrap();
    store.move_group(c, 0).unwrap();
    store.move_group(a, 9).unwrap();
    let groups = store.groups(space).unwrap();
    assert_eq!(groups.iter().map(|g| g.id).collect::<Vec<_>>(), [c, b, a]);
    assert_eq!(groups.iter().map(|g| g.position).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(groups[1].color, "green");
}

#[test]
fn bookmarks_move_only_into_folders_outside_themselves() {
    let (store, _) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap();
    let foreign = store.add_bookmark(other.id, None, None, "Theirs", 1).unwrap();
    let outer = store.add_bookmark(profile, None, None, "Outer", 1).unwrap();
    let inner = store.add_bookmark(profile, Some(outer.id), None, "Inner", 2).unwrap();
    let mark = store.add_bookmark(profile, None, Some("https://a.test/"), "A", 3).unwrap();
    store.move_bookmark(mark.id, Some(inner.id)).unwrap();
    assert!(store.move_bookmark(mark.id, Some(mark.id)).is_err());
    assert!(store.move_bookmark(outer.id, Some(inner.id)).is_err());
    assert!(store.move_bookmark(outer.id, Some(outer.id)).is_err());
    assert!(store.move_bookmark(mark.id, Some(foreign.id)).is_err());
    let moved = store.bookmarks(profile).unwrap().into_iter().find(|b| b.id == mark.id).unwrap();
    assert_eq!((moved.parent_id, moved.position), (Some(inner.id), 0));
    store.move_bookmark(inner.id, None).unwrap();
    let inner = store.bookmarks(profile).unwrap().into_iter().find(|b| b.id == inner.id).unwrap();
    assert_eq!((inner.parent_id, inner.position), (None, 1));
}

#[test]
fn profile_name_and_color_update() {
    let (store, _) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    assert_eq!(store.profile(profile).unwrap().color, "");
    store.set_profile(profile, "Samuel", "blue").unwrap();
    let p = store.profile(profile).unwrap();
    assert_eq!((p.name.as_str(), p.color.as_str()), ("Samuel", "blue"));
}

#[test]
fn site_zoom_is_per_profile_and_cleared_with_history() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_site_zoom(p, "a.test", Some(1.5)).unwrap();
    store.set_site_zoom(p, "a.test", Some(1.25)).unwrap();
    store.set_site_zoom(p, "b.test", Some(0.9)).unwrap();
    store.set_site_zoom(other, "a.test", Some(2.0)).unwrap();
    assert_eq!(store.site_zoom(p, "a.test").unwrap(), Some(1.25));
    assert_eq!(store.site_zooms(p).unwrap(), [("a.test".to_owned(), 1.25), ("b.test".to_owned(), 0.9)]);
    store.set_site_zoom(p, "b.test", None).unwrap();
    assert_eq!(store.site_zoom(p, "b.test").unwrap(), None);
    store.clear_history(p).unwrap();
    assert!(store.site_zooms(p).unwrap().is_empty());
    assert_eq!(store.site_zoom(other, "a.test").unwrap(), Some(2.0));
    store.clear_site_zooms(other).unwrap();
    assert!(store.site_zooms(other).unwrap().is_empty());
}

#[test]
fn site_zoom_and_downloads_go_with_their_profile() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_site_zoom(p, "mine.test", Some(1.5)).unwrap();
    store.set_site_zoom(other, "theirs.test", Some(1.5)).unwrap();
    store.add_download(p, "https://mine.test/a.zip", "/tmp/a.zip", "finished", 1).unwrap();
    let json = store.export_profile(p).unwrap();
    assert!(json.contains("\"site_zoom\"") && json.contains("mine.test") && json.contains("a.zip"));
    assert!(!json.contains("theirs.test"));
    store.reset_profile(p, "Home").unwrap();
    assert!(store.site_zooms(p).unwrap().is_empty() && store.downloads(p).unwrap().is_empty());
    store.delete_profile(other).unwrap();
    assert!(store.site_zooms(other).unwrap().is_empty());
}

#[test]
fn downloads_list_newest_first_per_profile() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.add_download(p, "https://a.test/old.zip", "/d/old.zip", "finished", 1).unwrap();
    store.add_download(p, "https://a.test/new.zip", "/d/new.zip", "failed", 2).unwrap();
    store.add_download(other, "https://b.test/x.zip", "/d/x.zip", "finished", 3).unwrap();
    let list = store.downloads(p).unwrap();
    assert_eq!(list.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), ["/d/new.zip", "/d/old.zip"]);
    assert_eq!((list[0].url.as_str(), list[0].state.as_str(), list[0].started_at), ("https://a.test/new.zip", "failed", 2));
    let theirs = store.downloads(other).unwrap()[0].id;
    store.delete_download(p, theirs).unwrap();
    assert_eq!(store.downloads(other).unwrap().len(), 1, "a row of another profile is not removed");
    store.delete_download(p, list[0].id).unwrap();
    assert_eq!(store.downloads(p).unwrap().len(), 1);
    store.clear_downloads(p).unwrap();
    assert!(store.downloads(p).unwrap().is_empty());
    assert_eq!(store.downloads(other).unwrap().len(), 1);
}

#[test]
fn spaces_keep_a_name_colour_and_icon() {
    let (store, space) = seeded();
    let profile = store.profiles().unwrap()[0].id;
    assert_eq!(store.spaces(profile).unwrap()[0].color, "");
    store.update_space(space, "Work", "indigo", "briefcase").unwrap();
    let saved = &store.spaces(profile).unwrap()[0];
    assert_eq!((saved.name.as_str(), saved.color.as_str(), saved.icon.as_str()), ("Work", "indigo", "briefcase"));
}

#[test]
fn muted_sites_are_per_profile_and_cleared_with_history() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_site_muted(p, "b.test", true).unwrap();
    store.set_site_muted(p, "a.test", true).unwrap();
    store.set_site_muted(p, "a.test", true).unwrap();
    store.set_site_muted(other, "theirs.test", true).unwrap();
    assert_eq!(store.muted_sites(p).unwrap(), ["a.test", "b.test"]);
    store.set_site_muted(p, "b.test", false).unwrap();
    assert_eq!(store.muted_sites(p).unwrap(), ["a.test"]);
    let json = store.export_profile(p).unwrap();
    assert!(json.contains("\"muted_sites\"") && json.contains("a.test") && !json.contains("theirs.test"));
    store.clear_history(p).unwrap();
    assert!(store.muted_sites(p).unwrap().is_empty());
    assert_eq!(store.muted_sites(other).unwrap(), ["theirs.test"]);
    store.reset_profile(other, "Home").unwrap();
    assert!(store.muted_sites(other).unwrap().is_empty());
}

#[test]
fn retitling_changes_only_the_visit_it_names() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    let first = store.add_history(p, "https://a.test/r", "Inbox", 1, false).unwrap();
    let second = store.add_history(p, "https://b.test/", "Other tab", 2, false).unwrap();
    let theirs = store.add_history(other, "https://a.test/r", "Theirs", 3, false).unwrap();
    assert!(first != second && second != theirs);
    store.retitle_visit(p, first, "https://a.test/r", "Settings").unwrap();
    store.retitle_visit(other, first, "https://a.test/r", "Wrong profile").unwrap();
    store.retitle_visit(p, first, "https://elsewhere.test/", "Wrong page").unwrap();
    store.clear_history(other).unwrap();
    let reused = store.add_history(other, "https://b.test/x", "B", 4, false).unwrap();
    store.retitle_visit(p, reused, "https://a.test/r", "Stale id").unwrap();
    assert_eq!(store.history(other, 10).unwrap()[0].title, "B");
    let titles: Vec<(String, String)> = store.history(p, 10).unwrap().into_iter().map(|e| (e.url, e.title)).collect();
    assert_eq!(titles, [("https://b.test/".to_owned(), "Other tab".to_owned()), ("https://a.test/r".to_owned(), "Settings".to_owned())]);
}

#[test]
fn paused_sites_are_per_profile_and_kept_with_history() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_site_paused(p, "b.test", true).unwrap();
    store.set_site_paused(p, "a.test", true).unwrap();
    store.set_site_paused(p, "a.test", true).unwrap();
    store.set_site_paused(other, "theirs.test", true).unwrap();
    assert_eq!(store.paused_sites(p).unwrap(), ["a.test", "b.test"]);
    store.set_site_paused(p, "b.test", false).unwrap();
    store.clear_history(p).unwrap();
    assert_eq!(store.paused_sites(p).unwrap(), ["a.test"]);
    assert!(store.export_profile(p).unwrap().contains("\"paused_sites\""));
    store.reset_profile(other, "Home").unwrap();
    assert!(store.paused_sites(other).unwrap().is_empty());
    store.delete_profile(p).unwrap();
    assert!(store.paused_sites(p).unwrap().is_empty());
}

#[test]
fn dark_sites_are_per_profile() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    store.set_site_dark(p, "b.test", true).unwrap();
    store.set_site_dark(p, "a.test", true).unwrap();
    store.set_site_dark(p, "a.test", true).unwrap();
    store.set_site_dark(other, "theirs.test", true).unwrap();
    assert_eq!(store.dark_sites(p).unwrap(), ["a.test", "b.test"]);
    store.set_site_dark(p, "b.test", false).unwrap();
    assert_eq!(store.dark_sites(p).unwrap(), ["a.test"]);
    assert!(store.site_is_dark(p, "a.test").unwrap() && !store.site_is_dark(p, "b.test").unwrap() && !store.site_is_dark(p, "theirs.test").unwrap());
    store.set_site_css(p, "a.test", "p { color: red }").unwrap();
    assert_eq!(store.site_css_for(p, "a.test").unwrap(), "p { color: red }");
    assert_eq!(store.site_css_for(other, "a.test").unwrap(), "");
    assert!(store.export_profile(p).unwrap().contains("\"site_dark\""));
    store.reset_profile(other, "Home").unwrap();
    assert!(store.dark_sites(other).unwrap().is_empty());
}

#[test]
fn pinned_tabs_remember_their_url_and_unpinning_forgets_it() {
    let (store, space) = seeded();
    let tab = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/start", "A").unwrap();
    assert_eq!(tab.pinned_url, None);
    store.move_tab(tab.id, TabKind::Pinned, None, 0).unwrap();
    assert_eq!(store.tab(tab.id).unwrap().pinned_url.as_deref(), Some("https://a.test/start"));
    store.update_tab_page(tab.id, "https://a.test/other", "Other").unwrap();
    store.move_tab(tab.id, TabKind::Tile, None, 0).unwrap();
    assert_eq!(store.tab(tab.id).unwrap().pinned_url.as_deref(), Some("https://a.test/start"));
    store.set_pinned_url(tab.id, "https://a.test/other").unwrap();
    assert_eq!(store.tab(tab.id).unwrap().pinned_url.as_deref(), Some("https://a.test/other"));
    store.move_tab(tab.id, TabKind::Ephemeral, None, 0).unwrap();
    assert_eq!(store.tab(tab.id).unwrap().pinned_url, None);
    store.set_pinned_url(tab.id, "https://a.test/").unwrap();
    assert_eq!(store.tab(tab.id).unwrap().pinned_url, None);
    let pinned = store.create_tab(space, TabKind::Pinned, None, "https://b.test/", "").unwrap();
    assert_eq!(pinned.pinned_url.as_deref(), Some("https://b.test/"));
}

#[test]
fn custom_titles_are_kept_until_cleared() {
    let (store, space) = seeded();
    let tab = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "Page").unwrap();
    assert_eq!(tab.custom_title, "");
    store.set_tab_custom_title(tab.id, "Mine").unwrap();
    store.update_tab_page(tab.id, "https://a.test/b", "Next page").unwrap();
    assert_eq!(store.tab(tab.id).unwrap().custom_title, "Mine");
    store.set_tab_custom_title(tab.id, "").unwrap();
    assert_eq!(store.tab(tab.id).unwrap().custom_title, "");
}

#[test]
fn permissions_are_listed_per_profile_and_removed_one_by_one() {
    let (store, _) = seeded();
    let p = store.profiles().unwrap()[0].id;
    let other = store.create_profile("Other", "uuid-b", false).unwrap().id;
    store.set_permission(p, "https://b.test", "microphone", "allow").unwrap();
    store.set_permission(p, "https://a.test", "camera", "deny").unwrap();
    store.set_permission(other, "https://a.test", "camera", "allow").unwrap();
    let row = |o: &str, k: &str, d: &str| (o.to_owned(), k.to_owned(), d.to_owned());
    assert_eq!(store.permissions(p).unwrap(), [row("https://a.test", "camera", "deny"), row("https://b.test", "microphone", "allow")]);
    store.delete_permission(p, "https://a.test", "camera").unwrap();
    assert_eq!(store.permissions(p).unwrap(), [row("https://b.test", "microphone", "allow")]);
    assert_eq!(store.permissions(other).unwrap().len(), 1);
}

#[test]
fn a_profile_can_move_to_a_new_webkit_store() {
    let (store, _) = seeded();
    let id = store.profiles().unwrap()[0].id;
    store.set_profile_store(id, "uuid-new").unwrap();
    assert_eq!(store.profile(id).unwrap().data_store_uuid, "uuid-new");
    let other = store.create_profile("Work", "uuid-b", false).unwrap().id;
    assert!(store.set_profile_store(other, "uuid-new").is_err(), "store ids stay unique");
}

#[test]
fn move_tab_to_space_keeps_the_tab() {
    let store = Store::open_in_memory().unwrap();
    let profile = store.create_profile("P", "uuid-p", false).unwrap().id;
    let home = store.create_space(profile, "Home", "").unwrap().id;
    let work = store.create_space(profile, "Work", "").unwrap().id;
    store.create_tab(work, TabKind::Pinned, None, "https://w.test/", "W").unwrap();
    let tab = store.create_tab(home, TabKind::Pinned, None, "https://a.test/", "A").unwrap();
    store.set_interaction_state(tab.id, Some(&[7])).unwrap();
    store.set_tab_custom_title(tab.id, "Mine").unwrap();
    store.move_tab_to_space(tab.id, work).unwrap();
    let moved = store.tab(tab.id).unwrap();
    assert_eq!((moved.space_id, moved.position, moved.custom_title.as_str()), (work, 1, "Mine"));
    assert_eq!(moved.pinned_url.as_deref(), Some("https://a.test/"));
    assert_eq!(store.interaction_state(tab.id).unwrap().as_deref(), Some(&[7u8][..]));
}

#[test]
fn web_engine_roundtrips_and_follows_moves() {
    let (store, space) = seeded();
    let tab = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap();
    assert_eq!(tab.web_engine, None);
    store.set_tab_web_engine(tab.id, Some(WebEngine::Chromium)).unwrap();
    store.move_tab(tab.id, TabKind::Pinned, None, 0).unwrap();
    let other = store.create_space(store.profiles().unwrap()[0].id, "Work", "").unwrap();
    store.move_tab_to_space(tab.id, other.id).unwrap();
    assert_eq!(store.tab(tab.id).unwrap().web_engine, Some(WebEngine::Chromium));
    store.set_tab_web_engine(tab.id, None).unwrap();
    assert_eq!(store.tab(tab.id).unwrap().web_engine, None);
}

#[test]
fn web_engine_resolution_order() {
    use ShieldLevel::{Lockdown, Standard, Strict};
    let cases = [
        (Standard, Standard, None, None, None, WebKit),
        (Standard, Standard, None, None, Some(Chromium), Chromium),
        (Standard, Standard, None, Some(Chromium), Some(WebKit), Chromium),
        (Standard, Standard, Some(WebKit), Some(Chromium), Some(Chromium), WebKit),
        (Strict, Strict, Some(Chromium), None, None, Chromium),
        (Standard, Lockdown, Some(Chromium), Some(Chromium), Some(Chromium), WebKit),
        (Lockdown, Standard, Some(Chromium), Some(Chromium), Some(Chromium), WebKit),
    ];
    for (profile, site, tab, window, default, expected) in cases {
        assert_eq!(WebEngine::resolve(profile, site, tab, window, default), expected, "{profile:?} {site:?} {tab:?} {window:?} {default:?}");
    }
}
