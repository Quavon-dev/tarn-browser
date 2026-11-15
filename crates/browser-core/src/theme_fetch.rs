use std::path::Path;

use store::{Id, Store};

use crate::github::{self, Fetch, Wanted};
use crate::theme::{self, Origin, Scheme, Source, editable_colors, parse};
use crate::theme_library::{self as library, sha256};

pub const CATALOG_SEEN_SETTING: &str = "theme_catalog_seen";
pub const DOWNLOADS_SETTING: &str = "extension_downloads";

pub fn downloads_on(store: &Store) -> bool {
    store.setting(DOWNLOADS_SETTING).ok().flatten().as_deref() != Some("off")
}

pub fn shown(raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| !c.is_control() && !crate::text::is_bidi(*c)).collect();
    crate::text::bounded_title(&clean)
}

#[derive(Clone, Debug)]
pub struct Download {
    pub css: String,
    pub sha256: String,
    pub resolved: github::Resolved,
}

impl Download {
    pub fn origin(&self) -> Origin {
        Origin { repo_id: self.resolved.repo_id as i64, owner_id: self.resolved.owner_id as i64, tag: self.resolved.tag.clone(), sha256: self.sha256.clone() }
    }
}

pub enum Release<'a> {
    Named(&'a str),
    Newest(u64),
}

pub fn fetch_theme(fetch: &dyn Fetch, release: Release) -> Result<Download, String> {
    let resolved = match release {
        Release::Named(input) => github::resolve_for(fetch, &github::parse_github(input)?, Wanted::Theme)?,
        Release::Newest(repo_id) => github::latest_for(fetch, repo_id, Wanted::Theme)?,
    };
    let asset = resolved.assets.first().ok_or("This release has no theme.css file; a theme repository publishes one with each release.")?;
    let bytes = fetch.get(&asset.url, theme::MAX_THEME_BYTES as u64).map_err(|e| e.describe())?;
    let css = String::from_utf8(bytes).map_err(|_| "theme.css is not UTF-8 text.".to_owned())?;
    parse(&css, Source::Unverified).map_err(|e| e.to_string())?;
    Ok(Download { sha256: sha256(css.as_bytes()), css, resolved })
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub download: Download,
    pub existing: Option<Id>,
    pub name: String,
    pub lines: Vec<String>,
}

impl Pending {
    pub fn title(&self) -> String {
        let r = &self.download.resolved;
        format!(
            "{} \u{201C}{}\u{201D} from {} ({})?",
            if self.existing.is_some() { "Update" } else { "Install" },
            self.name,
            shown(&r.full_name),
            shown(&r.tag)
        )
    }
}

pub fn pending(store: &Store, result: Result<Download, String>, existing: Option<Id>) -> Result<Pending, (bool, String)> {
    let previous = existing.and_then(|id| store.theme(id).ok().flatten());
    let d = result.map_err(|e| (true, e))?;
    if library::withdrawn(store, &d.sha256, Some(d.resolved.repo_id)) {
        return Err((true, "This theme or one from the same repository was withdrawn as unsafe and cannot be installed.".to_owned()));
    }
    if previous.as_ref().is_some_and(|p| p.tag == d.resolved.tag && p.sha256 == d.sha256) {
        return Err((false, "This theme is up to date.".to_owned()));
    }
    let name = parse(&d.css, Source::Unverified).map(|p| p.meta.name).unwrap_or_default();
    let lines = summary(&d, previous.as_ref());
    Ok(Pending { name: if name.is_empty() { "Untitled".to_owned() } else { shown(&name) }, lines, download: d, existing })
}

pub fn confirm(store: &Store, p: &Pending, now: i64) -> Result<(Id, Option<Scheme>), String> {
    let d = &p.download;
    if library::withdrawn(store, &d.sha256, Some(d.resolved.repo_id)) {
        return Err("This theme was withdrawn as unsafe and cannot be installed.".to_owned());
    }
    let id = theme::save(store, p.existing.unwrap_or(0), &d.resolved.source(), &d.css, d.origin(), now)?;
    let scheme = parse(&d.css, Source::Unverified).map_or(Scheme::Dark, |t| t.meta.scheme);
    Ok((id, p.existing.is_none().then_some(scheme)))
}

pub fn catalog_configured() -> bool {
    shield::catalog::THEME_CATALOG_PUBLIC_KEY != [0; 32]
}

pub fn cached_catalog(dir: &Path) -> Option<shield::catalog::Index> {
    let (index, sig) = (std::fs::read(dir.join("index.json")).ok()?, std::fs::read(dir.join("index.sig")).ok()?);
    shield::catalog::verify_catalog(&shield::catalog::THEMES, &index, &sig, &shield::catalog::THEME_CATALOG_PUBLIC_KEY).ok()
}

pub fn fetch_catalog(fetch: &dyn Fetch, dir: &Path, last: Option<u64>, now: u64) -> Result<shield::catalog::Index, String> {
    use shield::catalog::{THEME_CATALOG_PUBLIC_KEY, THEMES, check_fetched, verify_catalog};
    let (index, sig) = github::catalog_files(fetch, &THEMES)?;
    let parsed = verify_catalog(&THEMES, &index, &sig, &THEME_CATALOG_PUBLIC_KEY).map_err(|e| e.to_string())?;
    check_fetched(&parsed, last, now).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(dir.join("index.json"), &index))
        .and_then(|()| std::fs::write(dir.join("index.sig"), &sig))
        .map_err(|e| e.to_string())?;
    Ok(parsed)
}

pub fn last_catalog(store: &Store) -> Option<u64> {
    store.setting(CATALOG_SEEN_SETTING).ok().flatten().and_then(|v| v.parse().ok())
}

pub fn catalog_fetched(store: &Store, index: &shield::catalog::Index) -> bool {
    store.set_setting(CATALOG_SEEN_SETTING, &index.version.to_string()).ok();
    merge_catalog_revocations(store, index)
}

pub fn merge_cached_catalog(store: &Store, dir: &Path) -> bool {
    catalog_configured().then(|| cached_catalog(dir)).flatten().is_some_and(|index| merge_catalog_revocations(store, &index))
}

pub fn merge_catalog_revocations(store: &Store, index: &shield::catalog::Index) -> bool {
    library::merge_revocations(store, &shield::revocations::Revocations { sha256: index.revoked.iter().map(|r| r.sha256.clone()).collect() })
}

pub fn fetch_official(fetch: &dyn Fetch, entry: &shield::catalog::ThemeEntry) -> Result<(String, String), String> {
    let bytes = fetch.get(&entry.url, entry.size.min(shield::catalog::MAX_THEME_BYTES)).map_err(|e| e.describe())?;
    let hash = sha256(&bytes);
    if bytes.len() as u64 != entry.size || hash != entry.sha256 {
        return Err("The theme differs from the one the catalog lists.".to_owned());
    }
    let css = String::from_utf8(bytes).map_err(|_| "The theme is not UTF-8 text.".to_owned())?;
    let parsed = parse(&css, Source::Official).map_err(|e| e.to_string())?;
    let hosts: Vec<String> = parsed.sites.iter().flat_map(|s| s.hosts.iter().map(ToString::to_string)).collect();
    if !hosts.iter().all(|h| entry.page_css && entry.hosts.contains(h)) {
        return Err("The theme styles more websites than the catalog lists.".to_owned());
    }
    Ok((css, hash))
}

pub fn save_official(
    store: &Store,
    entry: &shield::catalog::ThemeEntry,
    existing: Option<Id>,
    fetched: Result<(String, String), String>,
    now: i64,
) -> Result<(Id, Option<Scheme>), String> {
    let (css, sha256) = fetched?;
    if library::withdrawn(store, &sha256, None) {
        return Err("This theme was withdrawn as unsafe and cannot be installed.".to_owned());
    }
    let id = theme::save(store, existing.unwrap_or(0), &format!("official:{}", entry.id), &css, Origin { sha256, ..Default::default() }, now)?;
    let scheme = if entry.scheme == "light" { Scheme::Light } else { Scheme::Dark };
    Ok((id, existing.is_none().then_some(scheme)))
}

#[derive(Clone, Debug, PartialEq)]
pub struct OfficialRow {
    pub entry: shield::catalog::ThemeEntry,
    pub existing: Option<Id>,
    pub action: Option<&'static str>,
    pub name: String,
    pub detail: String,
}

pub fn official_rows(store: &Store, index: &shield::catalog::Index) -> Vec<OfficialRow> {
    let installed = store.themes().unwrap_or_default();
    index
        .themes
        .iter()
        .map(|entry| {
            let source = format!("official:{}", entry.id);
            let row_of = installed.iter().find(|t| t.source == source);
            let action = match row_of {
                Some(t) if t.sha256 == entry.sha256 => None,
                Some(_) => Some("Update"),
                None => Some("Install"),
            };
            let detail = format!(
                "{}{} · {}",
                if entry.author.is_empty() { String::new() } else { format!("{} · ", shown(&entry.author)) },
                shown(&entry.version),
                entry.scheme
            );
            OfficialRow { entry: entry.clone(), existing: row_of.map(|t| t.id), action, name: shown(&entry.name), detail }
        })
        .collect()
}

pub fn summary(download: &Download, previous: Option<&store::InstalledTheme>) -> Vec<String> {
    let parsed = parse(&download.css, Source::Unverified).ok();
    let mut lines = vec!["Nobody reviewed this theme. It can only change colours, fonts and corners, and the colours of websites it names.".to_owned()];
    let hosts: Vec<String> = parsed.iter().flat_map(|p| p.sites.iter().flat_map(|s| s.hosts.iter().map(ToString::to_string))).collect();
    let Some(previous) = previous else {
        if !hosts.is_empty() {
            lines.push(shown(&format!("Website styles for {}; they stay off until you turn them on.", hosts.join(", "))));
        }
        return lines;
    };
    let source = download.resolved.source();
    lines.extend(github::provenance(
        (&previous.source, previous.owner_id as u64, &previous.tag, &previous.sha256),
        (&source, download.resolved.owner_id, &download.resolved.tag, &download.sha256),
        shown,
    ));
    let before = parse(&previous.css, Source::Unverified).ok();
    if let (Some(before), Some(after)) = (&before, &parsed) {
        let changed: Vec<&str> = editable_colors(&before.tokens)
            .into_iter()
            .zip(editable_colors(&after.tokens))
            .filter(|(a, b)| a.1 != b.1 && (a.0 != "search-color" || before.tokens.search_color != after.tokens.search_color))
            .map(|(a, _)| a.0)
            .collect();
        if !changed.is_empty() {
            lines.push(format!("Changes colours: {}.", changed.join(", ")));
        }
        if before.tokens.search_effect != after.tokens.search_effect {
            lines.push(format!("Changes the search bar effect to {}.", after.tokens.search_effect.label()));
        }
        let old: Vec<String> = before.sites.iter().flat_map(|s| s.hosts.iter().map(ToString::to_string)).collect();
        let added: Vec<&String> = hosts.iter().filter(|h| !old.contains(h)).collect();
        if !added.is_empty() {
            lines.push(shown(&format!("New website styles for {}.", added.iter().map(|h| h.as_str()).collect::<Vec<_>>().join(", "))));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Release, fetch_theme, summary};
    use crate::github::testing::Fake;

    const ASSET: &str = "https://github.com/o/r/releases/download/v1/theme.css";

    fn github(assets: serde_json::Value) -> Fake {
        Fake::default()
            .with("https://api.github.com/repos/o/r", json!({ "id": 1, "owner": { "id": 2 }, "full_name": "o/r" }))
            .with("https://api.github.com/repositories/1", json!({ "id": 1, "owner": { "id": 3 }, "full_name": "o/r2" }))
            .with("https://api.github.com/repositories/1/releases/latest", json!({ "tag_name": "v1", "assets": assets }))
    }

    #[test]
    fn a_theme_is_the_theme_css_asset_of_the_release() {
        let assets = json!([
            { "name": "x.zip", "browser_download_url": "https://github.com/o/r/releases/download/v1/x.zip" },
            { "name": "theme.css", "browser_download_url": ASSET },
        ]);
        let mut fake = github(assets);
        let css = "@tarn-theme { name: Got; } :root { --tarn-accent: #222 } @tarn-site a.test { a { color: red } }";
        fake.answers.insert(ASSET.into(), Ok(css.as_bytes().to_vec()));
        let d = fetch_theme(&fake, Release::Named("o/r")).unwrap();
        assert_eq!((d.css.as_str(), d.resolved.tag.as_str(), d.resolved.source()), (css, "v1", "github:1:o/r".to_owned()));
        assert_eq!(d.sha256.len(), 64);
        let lines = summary(&d, None);
        assert!(lines[0].starts_with("Nobody reviewed") && lines[1].contains("a.test"), "{lines:?}");

        let previous = store::InstalledTheme {
            source: "github:1:o/r".into(),
            owner_id: 2,
            tag: "v1".into(),
            sha256: "old".into(),
            css: ":root { --tarn-accent: #333; --tarn-search-effect: glow }".into(),
            ..Default::default()
        };
        let moved = "https://github.com/o/r2/releases/download/v1/theme.css";
        let mut renamed = github(json!([{ "name": "theme.css", "browser_download_url": moved }]));
        renamed.answers.insert(moved.into(), Ok(css.as_bytes().to_vec()));
        assert!(fetch_theme(&fake, Release::Newest(1)).is_err(), "an asset outside the repository's current name is refused");
        let update = fetch_theme(&renamed, Release::Newest(1)).unwrap();
        let lines = summary(&update, Some(&previous)).join("\n");
        assert!(!lines.contains("search-color"), "an unset search colour follows the accent silently: {lines}");
        for expected in [
            "now named o/r2",
            "another GitHub account",
            "changed although its version did not",
            "Changes colours: accent.",
            "search bar effect to None",
            "New website styles for a.test",
        ] {
            assert!(lines.contains(expected), "{expected}: {lines}");
        }
    }

    #[test]
    fn official_themes_must_match_their_entry() {
        let css = ":root { --tarn-accent: #222 } @tarn-site github.com { a { color: red } }";
        let url = "https://github.com/Quavon-dev/tarn-themes/releases/download/g-1/theme.css";
        let entry = |size: usize, sha: String, hosts: &[&str]| shield::catalog::ThemeEntry {
            id: "g".into(),
            name: "G".into(),
            author: String::new(),
            version: "1".into(),
            scheme: "dark".into(),
            url: url.into(),
            sha256: sha,
            size: size as u64,
            page_css: !hosts.is_empty(),
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
        };
        let mut fake = Fake::default();
        fake.answers.insert(url.into(), Ok(css.as_bytes().to_vec()));
        let good = entry(css.len(), crate::theme_library::sha256(css.as_bytes()), &["github.com"]);
        assert_eq!(super::fetch_official(&fake, &good).unwrap().0, css);
        assert!(super::fetch_official(&fake, &entry(css.len(), "0".repeat(64), &["github.com"])).is_err(), "hash");
        assert!(super::fetch_official(&fake, &entry(css.len() + 1, crate::theme_library::sha256(css.as_bytes()), &["github.com"])).is_err(), "size");
        assert!(super::fetch_official(&fake, &entry(css.len(), crate::theme_library::sha256(css.as_bytes()), &[])).is_err(), "styles beyond the entry");
    }

    #[test]
    fn missing_foreign_and_oversized_assets_are_refused() {
        let foreign = json!([{ "name": "theme.css", "browser_download_url": "https://github.com/evil/x/releases/download/v1/theme.css" }]);
        assert!(fetch_theme(&github(foreign), Release::Named("o/r")).unwrap_err().contains("no theme.css"));
        assert!(fetch_theme(&github(json!([])), Release::Named("o/r")).unwrap_err().contains("no theme.css"));
        let mut big = github(json!([{ "name": "THEME.css", "browser_download_url": ASSET }]));
        big.answers.insert(ASSET.into(), Ok(vec![b' '; crate::theme::MAX_THEME_BYTES + 1]));
        assert!(fetch_theme(&big, Release::Named("o/r")).is_err());
        assert!(fetch_theme(&github(json!([])), Release::Named("o/r/../../x")).is_err());
    }

    #[test]
    fn a_pending_theme_is_refused_when_withdrawn_and_saved_once_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let mut fake = github(json!([{ "name": "theme.css", "browser_download_url": ASSET }]));
        let css = "@tarn-theme { name: Got; scheme: light; } :root { --tarn-accent: #222 }";
        fake.answers.insert(ASSET.into(), Ok(css.as_bytes().to_vec()));
        let p = super::pending(&store, fetch_theme(&fake, Release::Named("o/r")), None).unwrap();
        assert_eq!(p.title(), "Install \u{201C}Got\u{201D} from o/r (v1)?");
        let (id, scheme) = super::confirm(&store, &p, 1).unwrap();
        assert_eq!(scheme, Some(crate::theme::Scheme::Light));
        let again = super::pending(&store, fetch_theme(&fake, Release::Named("o/r")), Some(id));
        assert_eq!(again.unwrap_err(), (false, "This theme is up to date.".to_owned()));
        let revoked = shield::revocations::Revocations { sha256: vec![p.download.sha256.clone()] };
        crate::theme_library::merge_revocations(&store, &revoked);
        assert!(super::confirm(&store, &p, 2).is_err());
        assert!(super::pending(&store, fetch_theme(&fake, Release::Named("o/r")), None).unwrap_err().1.contains("withdrawn"));
        assert_eq!(super::pending(&store, Err("x".into()), None).unwrap_err(), (true, "x".to_owned()));
    }

    #[test]
    fn official_rows_offer_install_update_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let entry = |id: &str, sha: &str| shield::catalog::ThemeEntry {
            id: id.into(),
            name: id.into(),
            author: "A".into(),
            version: "1".into(),
            scheme: "dark".into(),
            url: String::new(),
            sha256: sha.into(),
            size: 1,
            page_css: false,
            hosts: Vec::new(),
        };
        let css = ":root { --tarn-accent: #222 }".to_owned();
        let sha = crate::theme_library::sha256(css.as_bytes());
        super::save_official(&store, &entry("a", &sha), None, Ok((css.clone(), sha.clone())), 1).unwrap();
        super::save_official(&store, &entry("b", &sha), None, Ok((css, sha.clone())), 1).unwrap();
        let index: shield::catalog::Index = serde_json::from_value(json!({
            "kind": "tarn-theme-catalog", "version": 1, "created": 0, "themes": [], "revoked": [{ "id": "x", "sha256": "f".repeat(64) }]
        }))
        .unwrap();
        let index = shield::catalog::Index { themes: vec![entry("a", &sha), entry("b", "other"), entry("c", "x")], ..index };
        let actions: Vec<_> = super::official_rows(&store, &index).into_iter().map(|r| (r.action, r.detail)).collect();
        assert_eq!(actions[0], (None, "A · 1 · dark".to_owned()));
        assert_eq!((actions[1].0, actions[2].0), (Some("Update"), Some("Install")));
        assert!(super::catalog_fetched(&store, &index));
        assert_eq!(super::last_catalog(&store), Some(1));
        assert!(crate::theme_library::withdrawn(&store, &"f".repeat(64), None));
    }
}
