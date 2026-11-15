use super::*;

fn two_windows() -> (tempfile::TempDir, Browser, Browser) {
    let dir = tempfile::tempdir().unwrap();
    let core = Browser::open_core(dir.path());
    let first = Browser::saved_windows(&core).remove(0);
    let themes = themes::Themes::open(&core.disk, &dir.path().join("filters"));
    let mut a = Browser::new(core.clone(), first, HashSet::new(), themes.clone());
    a.new_tab("https://first.test/");
    let row = Browser::window_row(&core, SavedWindow::default());
    let b = Browser::new(core, row, a.model.lifecycle.live().collect(), themes);
    (dir, a, b)
}

#[test]
fn windows_share_the_core_and_keep_their_own_tabs_and_space() {
    let (_dir, mut a, mut b) = two_windows();
    assert!(Rc::ptr_eq(&a.core, &b.core));
    assert_eq!(a.core.disk.windows().unwrap().len(), 2);
    let key = a.new_tab("https://a.test/").unwrap();
    assert_ne!(b.model.active, Some(key), "a tab opened in one window does not take the other's focus");
    a.new_space();
    assert_ne!(a.model.space_ix, b.model.space_ix, "each window keeps its space");
    b.sync_shared();
    assert_eq!(b.model.spaces.len(), a.model.spaces.len(), "a space made in one window shows in the other");
}

#[test]
fn a_tab_closed_in_one_window_leaves_the_other_on_sync() {
    let (_dir, mut a, mut b) = two_windows();
    let shared = b.new_tab("https://both.test/").unwrap();
    a.sync_shared();
    a.activate(shared);
    a.close_tab(shared);
    b.sync_shared();
    assert!(b.model.tab(shared).is_none());
    assert!(!b.pages.contains_key(&shared));
    assert!(b.model.active.is_some_and(|k| k != shared), "the window shows another tab");
}

#[test]
fn private_and_burner_spaces_stay_per_window_and_burn_with_their_window() {
    let (_dir, mut a, mut b) = two_windows();
    a.private_space();
    b.private_space();
    assert_eq!(a.model.profile(), b.model.profile(), "windows share the one private session");
    a.open_burner();
    let burner = a.model.profile().unwrap().1;
    b.sync_shared();
    assert!(b.model.spaces.iter().all(|s| s.space.profile_id != burner), "another window never shows the burner");
    let row = a.window().id;
    a.close_window();
    assert!(a.released.len() == 1 && !a.core.profiles.borrow().contains_key(&(true, burner)));
    assert!(a.core.disk.windows().unwrap().iter().all(|w| w.id != row), "a closed window's row goes");
    assert_eq!(b.core.disk.windows().unwrap().len(), 1);
}

#[test]
fn a_new_window_row_keeps_its_size_for_the_next_launch() {
    let (dir, a, mut b) = two_windows();
    b.set_window_size(900., 700.);
    let id = b.window().id;
    drop((a, b));
    let core = Browser::open_core(dir.path());
    let rows = Browser::saved_windows(&core);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|w| w.id == id && w.width == 900. && w.height == 700.));
}

#[test]
fn a_download_shows_in_every_window_and_is_recorded_once() {
    let (_dir, mut a, mut b) = two_windows();
    let key = a.model.active.unwrap();
    let profile = a.model.profile().unwrap().1;
    let finished = json!({ "id": "3", "state": "finished", "name": "f.zip", "path": "/d/f.zip", "url": "https://d.test/f.zip" });
    a.on_download(key, &finished, true);
    b.on_download(key, &finished, false);
    assert_eq!((a.downloads.len(), b.downloads.len()), (1, 1));
    assert_eq!(a.core.disk.downloads(profile).unwrap().len(), 1);
}

#[test]
fn a_tab_is_live_in_one_window_at_a_time() {
    let (_dir, mut a, mut b) = two_windows();
    let first = a.model.active.unwrap();
    assert_ne!(b.model.active, Some(first), "a new window does not open on a tab another window shows");
    b.sync_shared();
    b.activate(first);
    a.elsewhere = b.model.lifecycle.live().collect();
    a.give_up(first);
    assert!(a.model.lifecycle.live().all(|k| k != first));
    assert!(a.model.active.is_some_and(|k| k != first), "the window that gave the tab up shows another");
    assert!(!a.pages.contains_key(&first), "it loads the stored address again if shown here later");
}

#[test]
fn closing_a_tab_never_takes_the_neighbour_another_window_shows() {
    let (_dir, mut a, mut b) = two_windows();
    let left = a.model.active.unwrap();
    let right = b.new_tab("https://right.test/").unwrap();
    a.sync_shared();
    a.elsewhere = b.model.lifecycle.live().collect();
    a.close_tab(left);
    assert_ne!(a.model.active, Some(right));
    assert_eq!(a.forgotten, [left], "the shell tells other windows which tabs went");
}
