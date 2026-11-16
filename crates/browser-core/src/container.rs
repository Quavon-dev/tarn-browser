//! The one-time move of the App Sandbox container's data to where the unsandboxed app keeps it
//! so existing profiles survive the switch to the unsandboxed app.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use store::Store;

pub const BUNDLE_ID: &str = "de.quavon.tarn";
/// The bundle ID of the sandboxed builds, whose container this moves out of.
pub const SANDBOXED_ID: &str = "dev.tarn.browser";
const MARKER: &str = "tarn-moved-out";
const APP_SUPPORT: &str = "Library/Application Support/Tarn";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No container, or its data was moved before.
    Nothing,
    /// Moved; these items were symbolic links in the container and stayed where they were.
    Moved(Vec<PathBuf>),
    /// Nothing was moved because this target exists already.
    TargetExists(PathBuf),
}

/// Moves the container's data below `home` item by item, or nothing at all. An error means the
/// container is as it was, unless the error says moving back failed too.
pub fn move_out(home: &Path) -> io::Result<Outcome> {
    let relative = format!("Library/Containers/{SANDBOXED_ID}/Data");
    let container = home.join(&relative);
    if !real_dir(home, &relative) || fs::symlink_metadata(container.join(MARKER)).is_ok() {
        return Ok(Outcome::Nothing);
    }
    // WebKit keeps its data in `~/Library/WebKit` inside a container and in a folder named after
    // the bundle outside one (`engine::data_store`).
    let candidates = [(APP_SUPPORT, APP_SUPPORT.to_owned()), ("Library/WebKit", format!("Library/WebKit/{BUNDLE_ID}"))];
    let skipped: Vec<PathBuf> =
        candidates.iter().map(|(from, _)| container.join(from)).filter(|path| fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink())).collect();
    let items: Vec<(PathBuf, PathBuf)> =
        candidates.into_iter().filter(|(from, _)| real_dir(&container, from)).map(|(from, to)| (container.join(from), home.join(to))).collect();
    if let Some((_, to)) = items.iter().find(|(_, to)| fs::symlink_metadata(to).is_ok()) {
        return Ok(Outcome::TargetExists(to.clone()));
    }
    // The sandboxed app never makes a Chromium profile; one in the container was put there.
    if fs::symlink_metadata(container.join(APP_SUPPORT).join("Chromium")).is_ok() {
        return Err(io::Error::other("the container holds a Chromium profile no sandboxed Tarn made"));
    }
    // A link the sandbox kept inside the container would be followed once the app runs without it.
    for (from, _) in &items {
        if let Some(link) = find_link(from)? {
            return Err(io::Error::other(format!("{} is a symbolic link", link.display())));
        }
    }
    // A rename within one volume needs no free space; across volumes it would be a copy.
    let volume = fs::metadata(&container)?.dev();
    for (_, to) in &items {
        if to.ancestors().find_map(|a| fs::metadata(a).ok()).is_none_or(|m| m.dev() != volume) {
            return Err(io::Error::other(format!("{} is on another volume", to.display())));
        }
    }
    let mut moved = Vec::new();
    for (from, to) in &items {
        if let Err(e) = to.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::rename(from, to)) {
            return Err(move_back(&moved, e));
        }
        moved.push((from, to));
    }
    let marker = container.join(MARKER);
    if let Err(e) = fs::write(&marker, b"").and_then(|()| move_download_paths(home, &container)) {
        fs::remove_file(&marker).ok();
        return Err(move_back(&moved, e));
    }
    Ok(Outcome::Moved(skipped))
}

/// Whether `rel` below `root` is a directory reached without following a symbolic link.
/// Renames the WebKit data that builds with the bundle ID `dev.tarn.browser` kept outside a
/// container to the current bundle ID's folder, unless that exists already.
pub fn move_renamed(home: &Path) -> io::Result<bool> {
    let from = format!("Library/WebKit/{SANDBOXED_ID}");
    let to = home.join(format!("Library/WebKit/{BUNDLE_ID}"));
    if !real_dir(home, &from) || fs::symlink_metadata(&to).is_ok() {
        return Ok(false);
    }
    fs::rename(home.join(from), to)?;
    Ok(true)
}

fn real_dir(root: &Path, rel: &str) -> bool {
    let mut path = root.to_path_buf();
    Path::new(rel).components().all(|c| {
        path.push(c);
        fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
    })
}

fn find_link(dir: &Path) -> io::Result<Option<PathBuf>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Ok(Some(entry.path()));
        }
        if kind.is_dir()
            && let Some(link) = find_link(&entry.path())?
        {
            return Ok(Some(link));
        }
    }
    Ok(None)
}

fn move_back(moved: &[(&PathBuf, &PathBuf)], error: io::Error) -> io::Error {
    match moved.iter().rev().map(|(from, to)| fs::rename(to, from)).find_map(Result::err) {
        Some(back) => io::Error::other(format!("{error}; moving back failed: {back}")),
        None => error,
    }
}

/// Paths the sandboxed app stored point into the container's home, where `Downloads` and the
/// other user folders are links to the real ones.
fn move_download_paths(home: &Path, container: &Path) -> io::Result<()> {
    let dir = home.join(APP_SUPPORT);
    if !dir.join("tarn.db").is_file() {
        return Ok(());
    }
    Store::open(&dir)
        .and_then(|store| store.move_download_paths(&container.to_string_lossy(), &home.to_string_lossy()))
        .map(drop)
        .map_err(|e| io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    const CONTAINER: &str = "Library/Containers/dev.tarn.browser/Data";

    fn seed(home: &Path) -> PathBuf {
        let container = home.join(CONTAINER);
        let data = container.join(APP_SUPPORT);
        fs::create_dir_all(data.join("filters")).unwrap();
        fs::write(data.join("filters/bundle"), "lists").unwrap();
        let store = Store::open(&data).unwrap();
        let profile = store.create_profile("P", "uuid", false).unwrap();
        store.add_download(profile.id, "https://a/", &container.join("Downloads/a.zip").to_string_lossy(), "done", 1).unwrap();
        store.add_download(profile.id, "https://b/", &format!("{}2/b.zip", container.display()), "done", 2).unwrap();
        fs::create_dir_all(container.join("Library/WebKit/WebsiteDataStore/uuid")).unwrap();
        fs::write(container.join("Library/WebKit/WebsiteDataStore/uuid/cookies"), "c").unwrap();
        fs::create_dir_all(home.join("Downloads")).unwrap();
        symlink(home.join("Downloads"), container.join("Downloads")).unwrap();
        container
    }

    fn paths(home: &Path) -> Vec<String> {
        let store = Store::open(&home.join(APP_SUPPORT)).unwrap();
        let profile = store.profiles().unwrap()[0].id;
        store.downloads(profile).unwrap().into_iter().map(|d| d.path).collect()
    }

    #[test]
    fn moves_each_item_once_and_rewrites_paths_into_it() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let container = seed(home);
        assert_eq!(move_out(home).unwrap(), Outcome::Moved(Vec::new()));
        assert_eq!(fs::read_to_string(home.join(APP_SUPPORT).join("filters/bundle")).unwrap(), "lists");
        assert!(home.join("Library/WebKit/de.quavon.tarn/WebsiteDataStore/uuid/cookies").is_file());
        assert!(!container.join(APP_SUPPORT).exists() && !container.join("Library/WebKit").exists());
        assert!(fs::symlink_metadata(container.join("Downloads")).unwrap().is_symlink(), "user folder links stay");
        assert_eq!(
            paths(home),
            [format!("{}2/b.zip", container.display()), home.join("Downloads/a.zip").to_string_lossy().into_owned()],
            "only paths inside the container's home change"
        );
        assert!(container.join(MARKER).is_file());

        fs::create_dir_all(container.join("Library/WebKit/new")).unwrap();
        assert_eq!(move_out(home).unwrap(), Outcome::Nothing, "the marker keeps it from running again");
        assert!(container.join("Library/WebKit/new").is_dir());
    }

    #[test]
    fn symbolic_links_are_neither_followed_nor_moved() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let container = seed(home);
        let elsewhere = home.join("elsewhere");
        fs::rename(container.join("Library/WebKit"), &elsewhere).unwrap();
        symlink(&elsewhere, container.join("Library/WebKit")).unwrap();
        assert_eq!(move_out(home).unwrap(), Outcome::Moved(vec![container.join("Library/WebKit")]));
        assert!(fs::symlink_metadata(container.join("Library/WebKit")).unwrap().is_symlink());
        assert!(elsewhere.join("WebsiteDataStore/uuid/cookies").is_file());
        assert!(!home.join("Library/WebKit/de.quavon.tarn").exists());
        assert!(home.join("Downloads").is_dir() && fs::read_dir(home.join("Downloads")).unwrap().next().is_none());
    }

    #[test]
    fn a_link_inside_an_item_moves_nothing() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let container = seed(home);
        symlink(home.join("Downloads"), container.join("Library/WebKit/WebsiteDataStore/uuid/link")).unwrap();
        assert!(move_out(home).is_err());
        fs::remove_file(container.join("Library/WebKit/WebsiteDataStore/uuid/link")).unwrap();
        fs::create_dir(container.join(APP_SUPPORT).join("Chromium")).unwrap();
        assert!(move_out(home).is_err(), "a Chromium profile in the container is refused");
        assert!(container.join(APP_SUPPORT).join("tarn.db").is_file());
        assert!(!home.join(APP_SUPPORT).exists() && !home.join("Library/WebKit").exists());
        assert!(!container.join(MARKER).exists());
    }

    #[test]
    fn an_existing_target_moves_nothing() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let container = seed(home);
        let target = home.join("Library/WebKit/de.quavon.tarn");
        fs::create_dir_all(&target).unwrap();
        assert_eq!(move_out(home).unwrap(), Outcome::TargetExists(target));
        assert!(container.join(APP_SUPPORT).join("tarn.db").is_file());
        assert!(!home.join(APP_SUPPORT).exists());
        assert!(!container.join(MARKER).exists());
    }

    #[test]
    fn a_failure_on_the_second_item_moves_the_first_back() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let container = seed(home);
        fs::write(home.join("Library/WebKit"), "not a folder").unwrap();
        assert!(move_out(home).is_err());
        assert!(container.join(APP_SUPPORT).join("filters/bundle").is_file());
        assert!(!home.join(APP_SUPPORT).exists());
        assert!(container.join("Library/WebKit/WebsiteDataStore/uuid/cookies").is_file());
        assert!(!container.join(MARKER).exists(), "a failed move runs again next time");
    }

    #[test]
    fn webkit_data_of_the_old_bundle_id_moves_once_and_never_over_newer_data() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let old = home.join("Library/WebKit/dev.tarn.browser/WebsiteDataStore");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("cookies"), "c").unwrap();
        assert!(move_renamed(home).unwrap());
        assert!(home.join("Library/WebKit/de.quavon.tarn/WebsiteDataStore/cookies").is_file());
        assert!(!move_renamed(home).unwrap());
        fs::create_dir_all(&old).unwrap();
        assert!(!move_renamed(home).unwrap(), "the current folder wins");
        assert!(old.is_dir());
    }

    #[test]
    fn no_container_is_nothing_to_do() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(move_out(home.path()).unwrap(), Outcome::Nothing);
        assert!(!home.path().join(APP_SUPPORT).exists());
    }
}
