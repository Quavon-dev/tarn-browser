use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Browser, Source};

/// Labels are optional; a `Local State` larger than this is not read.
const MAX_LOCAL_STATE: u64 = 16 * 1024 * 1024;

/// Other browsers' profiles are out of reach inside the App Sandbox, which macOS marks with this
/// variable in every sandboxed process.
pub fn available() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_none()
}

/// Every profile of every supported browser installed under `home`. Profile folders are found
/// by listing the directory; names read from the browser's own files only label them.
pub fn sources(home: &Path) -> Vec<Source> {
    let mut out = Vec::new();
    for browser in Browser::ALL {
        let Some(root) = browser.profiles_dir().and_then(|dir| resolve(home, dir)) else { continue };
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut found: Vec<Source> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| {
                let dir = e.path();
                let name = e.file_name().into_string().ok()?;
                let label = match browser {
                    Browser::Firefox => (dir.join("places.sqlite").is_file() || dir.join("cookies.sqlite").is_file()).then(|| firefox_label(&name))?,
                    _ => ((name == "Default" || name.starts_with("Profile ")) && dir.join("Preferences").is_file()).then(|| name.clone())?,
                };
                Some(Source { browser, label, dir })
            })
            .collect();
        if browser != Browser::Firefox {
            let names = chromium_names(&root);
            for s in &mut found {
                if let Some(name) = names.as_ref().and_then(|n| n.get(&s.label)).and_then(|p| p.get("name")).and_then(Value::as_str) {
                    s.label = name.to_owned();
                }
            }
        }
        found.sort_by(|a, b| a.dir.cmp(&b.dir));
        out.extend(found);
    }
    out
}

/// `home` joined with `dir`; a part ending in `*` is the folder whose name starts with the rest,
/// as a packaged app's folder carries its publisher's hash.
fn resolve(home: &Path, dir: &str) -> Option<PathBuf> {
    let mut path = home.to_path_buf();
    for part in dir.split('/') {
        path = match part.strip_suffix('*') {
            Some(prefix) => std::fs::read_dir(&path)
                .ok()?
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(prefix)) && e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .min()?,
            None => path.join(part),
        };
    }
    Some(path)
}

/// `Local State`'s profile names, keyed by folder name.
fn chromium_names(root: &Path) -> Option<serde_json::Map<String, Value>> {
    let path = root.join("Local State");
    if std::fs::metadata(&path).ok()?.len() > MAX_LOCAL_STATE {
        return None;
    }
    let json: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    json.get("profile")?.get("info_cache")?.as_object().cloned()
}

/// `a1b2c3d4.default-release` is shown as `default-release`.
fn firefox_label(folder: &str) -> String {
    folder.split_once('.').map_or(folder, |(_, name)| name).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn finds_chromium_and_firefox_profiles_in_a_fixture_home() {
        let home = tempfile::tempdir().unwrap();
        let support = home.path().join("Library/Application Support");
        for dir in ["Arc/User Data/Default", "Arc/User Data/Profile 2", "Arc/User Data/Crashpad", "Google/Chrome/Default"] {
            std::fs::create_dir_all(support.join(dir)).unwrap();
            std::fs::write(support.join(dir).join("Preferences"), "{}").unwrap();
        }
        std::fs::write(support.join("Arc/User Data/Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Work"},"Profile 2":{"name":"../../etc"}}}}"#)
            .unwrap();
        std::fs::create_dir_all(support.join("Firefox/Profiles/ab12.default-release")).unwrap();
        std::fs::write(support.join("Firefox/Profiles/ab12.default-release/places.sqlite"), "").unwrap();
        std::fs::create_dir_all(support.join("Firefox/Profiles/empty.old")).unwrap();

        let found: Vec<(Browser, String, bool)> = sources(home.path()).into_iter().map(|s| (s.browser, s.label, s.dir.starts_with(&support))).collect();
        assert_eq!(
            found,
            [
                (Browser::Chrome, "Default".into(), true),
                (Browser::Arc, "Work".into(), true),
                (Browser::Arc, "../../etc".into(), true),
                (Browser::Firefox, "default-release".into(), true),
            ]
        );
        let arc = sources(home.path()).into_iter().find(|s| s.label == "../../etc").unwrap();
        assert_eq!(arc.dir, support.join("Arc/User Data/Profile 2"), "a label never becomes a path");
    }

    #[cfg(windows)]
    #[test]
    fn finds_windows_profiles_in_a_fixture_home() {
        let home = tempfile::tempdir().unwrap();
        let edge = home.path().join("AppData/Local/Microsoft/Edge/User Data/Default");
        std::fs::create_dir_all(&edge).unwrap();
        std::fs::write(edge.join("Preferences"), "{}").unwrap();
        let firefox = home.path().join("AppData/Roaming/Mozilla/Firefox/Profiles/ab12.default-release");
        std::fs::create_dir_all(&firefox).unwrap();
        std::fs::write(firefox.join("places.sqlite"), "").unwrap();
        let found: Vec<(Browser, String)> = sources(home.path()).into_iter().map(|s| (s.browser, s.label)).collect();
        assert_eq!(found, [(Browser::Edge, "Default".into()), (Browser::Firefox, "default-release".into())]);
    }

    #[cfg(windows)]
    #[test]
    fn finds_arc_in_its_package_folder() {
        let home = tempfile::tempdir().unwrap();
        let packages = home.path().join("AppData/Local/Packages");
        let arc = packages.join("TheBrowserCompany.Arc_ab12cd34/LocalCache/Local/Arc/User Data/Default");
        std::fs::create_dir_all(&arc).unwrap();
        std::fs::write(arc.join("Preferences"), "{}").unwrap();
        std::fs::create_dir_all(packages.join("Other.App_1")).unwrap();
        std::fs::write(packages.join("TheBrowserCompany.Arc_file"), "").unwrap();
        let found: Vec<(Browser, String, PathBuf)> = sources(home.path()).into_iter().map(|s| (s.browser, s.label, s.dir)).collect();
        assert_eq!(found, [(Browser::Arc, "Default".into(), arc)]);
    }

    #[test]
    fn a_missing_package_folder_finds_nothing() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(resolve(home.path(), "Packages/Some.App_*/Data"), None);
        assert_eq!(resolve(home.path(), "a/b"), Some(home.path().join("a").join("b")));
    }
}
