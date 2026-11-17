use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{Browser, Source};

/// Labels are optional; a `Local State` larger than this is not read.
const MAX_LOCAL_STATE: u64 = 16 * 1024 * 1024;
const MAX_PROFILES_INI: u64 = 1024 * 1024;

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
        let mut found: Vec<Source> = browser.profiles_dirs().iter().filter_map(|dir| resolve(home, dir)).flat_map(|root| profiles(browser, &root)).collect();
        found.sort_by(|a, b| a.dir.cmp(&b.dir));
        found.dedup_by(|a, b| a.dir == b.dir);
        out.extend(found);
    }
    out
}

fn profiles(browser: Browser, root: &Path) -> Vec<Source> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Source> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let dir = e.path();
            let name = e.file_name().into_string().ok()?;
            let label = match browser {
                Browser::Firefox => firefox_profile(&dir).then(|| firefox_label(&name))?,
                _ => ((name == "Default" || name.starts_with("Profile ")) && dir.join("Preferences").is_file()).then(|| name.clone())?,
            };
            Some(Source { browser, label, dir })
        })
        .collect();
    if browser == Browser::Firefox {
        for (dir, label) in firefox_ini(root) {
            if firefox_profile(&dir) && !found.iter().any(|s| s.dir == dir) {
                found.push(Source { browser, label, dir });
            }
        }
    } else {
        let names = chromium_names(root);
        for s in &mut found {
            if let Some(name) = names.as_ref().and_then(|n| n.get(&s.label)).and_then(|p| p.get("name")).and_then(Value::as_str) {
                s.label = name.to_owned();
            }
        }
    }
    found
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

fn firefox_profile(dir: &Path) -> bool {
    dir.join("places.sqlite").is_file() || dir.join("cookies.sqlite").is_file()
}

/// The profiles `profiles.ini` in `root` names, which may live outside `root`, with their names.
fn firefox_ini(root: &Path) -> Vec<(PathBuf, String)> {
    let path = root.join("profiles.ini");
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= MAX_PROFILES_INI => std::fs::read_to_string(path).map(|text| ini_profiles(&text, root)).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[derive(Default)]
struct IniProfile {
    name: Option<String>,
    path: Option<String>,
    relative: Option<bool>,
}

/// Each `[Profile…]` section with a `Path` and an `IsRelative`, as Firefox requires.
fn ini_profiles(text: &str, root: &Path) -> Vec<(PathBuf, String)> {
    let mut sections: Vec<IniProfile> = Vec::new();
    let mut in_profile = false;
    for line in text.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            in_profile = header.starts_with("Profile");
            if in_profile {
                sections.push(IniProfile::default());
            }
        } else if let (true, Some((key, value)), Some(section)) = (in_profile, line.split_once('='), sections.last_mut()) {
            let value = value.trim().to_owned();
            match key.trim() {
                "Name" => section.name = Some(value),
                "Path" => section.path = Some(value),
                "IsRelative" => section.relative = Some(value == "1"),
                _ => {}
            }
        }
    }
    sections
        .into_iter()
        .filter_map(|s| {
            let path = s.path?;
            let dir = if s.relative? { root.join(&path) } else { PathBuf::from(&path) };
            let label = s.name.unwrap_or_else(|| firefox_label(path.rsplit('/').next().unwrap_or_default()));
            Some((dir, label))
        })
        .collect()
}

/// `a1b2c3d4.default-release` is shown as `default-release`.
fn firefox_label(folder: &str) -> String {
    folder.split_once('.').map_or(folder, |(_, name)| name).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_linux_profiles_in_a_fixture_home() {
        let home = tempfile::tempdir().unwrap();
        for dir in [
            ".config/google-chrome/Default",
            ".config/google-chrome/Profile 1",
            ".config/google-chrome/Crashpad",
            ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser/Default",
            ".config/microsoft-edge/Default",
            ".config/chromium/Default",
        ] {
            std::fs::create_dir_all(home.path().join(dir)).unwrap();
            std::fs::write(home.path().join(dir).join("Preferences"), "{}").unwrap();
        }
        std::fs::write(home.path().join(".config/google-chrome/Local State"), r#"{"profile":{"info_cache":{"Profile 1":{"name":"Work"}}}}"#).unwrap();
        for dir in [".mozilla/firefox/ab12.default-release", "snap/firefox/common/.mozilla/firefox/cd34.default", ".config/mozilla/firefox/ef56.new"] {
            std::fs::create_dir_all(home.path().join(dir)).unwrap();
            std::fs::write(home.path().join(dir).join("places.sqlite"), "").unwrap();
        }
        std::fs::create_dir_all(home.path().join(".mozilla/firefox/Crash Reports")).unwrap();
        let elsewhere = home.path().join("elsewhere/gh78.work");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("cookies.sqlite"), "").unwrap();
        let ini =
            format!("[Profile1]\nName=Work\nIsRelative=0\nPath={}\n\n[Profile0]\nName=Main\nIsRelative=1\nPath=ab12.default-release\n", elsewhere.display());
        std::fs::write(home.path().join(".mozilla/firefox/profiles.ini"), ini).unwrap();
        let found: Vec<(Browser, String)> = sources(home.path()).into_iter().map(|s| (s.browser, s.label)).collect();
        assert_eq!(
            found,
            [
                (Browser::Chrome, "Default".into()),
                (Browser::Chrome, "Work".into()),
                (Browser::Edge, "Default".into()),
                (Browser::Brave, "Default".into()),
                (Browser::Chromium, "Default".into()),
                (Browser::Firefox, "new".into()),
                (Browser::Firefox, "default-release".into()),
                (Browser::Firefox, "Work".into()),
                (Browser::Firefox, "default".into()),
            ]
        );
        let brave = sources(home.path()).into_iter().find(|s| s.browser == Browser::Brave).unwrap();
        assert_eq!(brave.dir, home.path().join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser/Default"), "the Flatpak's folder");
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
    fn reads_firefox_profiles_ini() {
        let root = Path::new("/r");
        let ini = "[General]\nPath=general\nIsRelative=1\n\n[Profile0]\nName=default\nIsRelative=1\nPath=Profiles/ab12.default\n\n[Profile1]\nIsRelative=0\nPath=/elsewhere/cd34.work\n\n[Profile2]\nName=no flag\nPath=x\n\n[Install4F96D1932A9F858E]\nDefault=Profiles/ab12.default\n";
        assert_eq!(
            ini_profiles(ini, root),
            [(root.join("Profiles/ab12.default"), "default".into()), (PathBuf::from("/elsewhere/cd34.work"), "work".into())],
            "only profile sections with a path and IsRelative count"
        );
    }

    #[test]
    fn a_profile_named_only_in_profiles_ini_is_found_once() {
        let home = tempfile::tempdir().unwrap();
        let Some(root) = Browser::Firefox.profiles_dirs().first().and_then(|d| resolve(home.path(), d)) else { return };
        let outside = home.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(root.join("ab12.main")).unwrap();
        std::fs::write(root.join("ab12.main/places.sqlite"), "").unwrap();
        let ini = format!("[Profile0]\nName=Kept\nIsRelative=0\nPath={}\n[Profile1]\nName=Main\nIsRelative=1\nPath=ab12.main\n", outside.display());
        std::fs::write(root.join("profiles.ini"), ini).unwrap();
        let labels = || sources(home.path()).into_iter().map(|s| s.label).collect::<Vec<_>>();
        assert_eq!(labels(), ["main"], "a folder without Firefox's files is no profile, and a listed one is not added twice");
        std::fs::write(outside.join("places.sqlite"), "").unwrap();
        let found: Vec<(String, PathBuf)> = sources(home.path()).into_iter().map(|s| (s.label, s.dir)).collect();
        assert_eq!(found, [("main".to_owned(), root.join("ab12.main")), ("Kept".to_owned(), outside)]);
    }

    #[test]
    fn a_missing_package_folder_finds_nothing() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(resolve(home.path(), "Packages/Some.App_*/Data"), None);
        assert_eq!(resolve(home.path(), "a/b"), Some(home.path().join("a").join("b")));
    }
}
