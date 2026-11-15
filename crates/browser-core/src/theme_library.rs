use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use store::{Id, Store};

use crate::effects::{Effect, chosen};
use crate::theme::{self, Active, Origin, Scheme, Selection, Source, ThemeRef};

pub const REVOKED_SETTING: &str = "theme.revoked";
pub const REVOKED_REPOS_SETTING: &str = "theme.revoked_repos";
pub const SEARCH_EFFECT_SETTING: &str = "search_effect";

pub fn revoked(store: &Store) -> String {
    store.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default()
}

pub fn merge_revocations(store: &Store, list: &shield::revocations::Revocations) -> bool {
    let stored = revoked(store);
    let merged = shield::revocations::merge(&stored, list);
    let grew = merged != stored && store.set_setting(REVOKED_SETTING, &merged).is_ok();
    remember_withdrawn_repos(store);
    grew
}

pub fn merge_cached_bundle(store: &Store, dir: &Path) -> bool {
    let Ok(bundle) = shield::cache::read(dir, &shield::groups::public_key()) else { return false };
    let list = bundle.of(shield::revocations::CATEGORY, shield::bundle::Kind::Config).next().and_then(|l| shield::revocations::parse(&l.data));
    list.is_some_and(|list| merge_revocations(store, &list))
}

pub fn remember_withdrawn_repos(store: &Store) {
    let revoked = revoked(store);
    let stored = store.setting(REVOKED_REPOS_SETTING).ok().flatten().unwrap_or_default();
    let mut repos: BTreeSet<u64> = stored.lines().filter_map(|l| l.parse().ok()).collect();
    let before = repos.len();
    repos.extend(
        store.themes().unwrap_or_default().iter().filter(|t| t.repo_id > 0 && shield::revocations::contains(&revoked, &t.sha256)).map(|t| t.repo_id as u64),
    );
    if repos.len() != before {
        store.set_setting(REVOKED_REPOS_SETTING, &repos.iter().map(u64::to_string).collect::<Vec<_>>().join("\n")).ok();
    }
}

pub fn withdrawn(store: &Store, sha256: &str, repo_id: Option<u64>) -> bool {
    remember_withdrawn_repos(store);
    let repos = store.setting(REVOKED_REPOS_SETTING).ok().flatten().unwrap_or_default();
    shield::revocations::contains(&revoked(store), sha256) || repo_id.is_some_and(|id| repos.lines().any(|l| l == id.to_string()))
}

pub fn github_source(source: &str) -> Option<(u64, &str)> {
    let (id, name) = source.strip_prefix("github:")?.split_once(':')?;
    Some((id.parse().ok()?, name))
}

fn repo_of(row: &store::InstalledTheme) -> Option<u64> {
    u64::try_from(row.repo_id).ok().filter(|r| *r > 0)
}

pub fn current(store: &Store, system_light: bool, safe: bool) -> Active {
    let wanted = if safe { ThemeRef::DARK } else { Selection::load(store).active(system_light).clone() };
    theme::resolve(store, &wanted, &|sha, repo| withdrawn(store, sha, u64::try_from(repo).ok().filter(|r| *r > 0)))
}

pub fn search_effect(store: &Store, active: &Active) -> Effect {
    chosen(active.parsed.tokens.search_effect, store.setting(SEARCH_EFFECT_SETTING).ok().flatten().as_deref())
}

pub fn chosen_effect(store: &Store) -> Option<Effect> {
    store.setting(SEARCH_EFFECT_SETTING).ok().flatten().and_then(|v| Effect::parse(&v))
}

pub fn set_effect(store: &Store, effect: Option<Effect>) -> Result<(), String> {
    store.set_setting(SEARCH_EFFECT_SETTING, effect.map_or("theme", Effect::name)).map_err(|e| e.to_string())
}

pub fn pages_on(store: &Store, profile_id: Id) -> bool {
    store.setting(&theme::pages_setting(profile_id)).ok().flatten().as_deref() != Some("off")
}

pub fn set_pages(store: &Store, profile_id: Id, on: bool) -> Result<(), String> {
    store.set_setting(&theme::pages_setting(profile_id), if on { "on" } else { "off" }).map_err(|e| e.to_string())
}

pub fn profile_sheets(store: &Store, active: &Active, safe: bool, private: bool, profile_id: Id) -> Vec<(String, String)> {
    theme::page_sheets(active, !safe && !private && pages_on(store, profile_id))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: ThemeRef,
    pub name: String,
    pub badge: String,
    pub scheme: Scheme,
    pub swatches: [u32; 4],
    pub editable: bool,
    pub stored: Option<Id>,
    pub hosts: Vec<String>,
    pub warnings: Vec<String>,
    pub adjusted: Vec<String>,
    pub page_css: bool,
    pub unverified: bool,
    pub withdrawn: bool,
    pub repo_id: Option<u64>,
}

pub use crate::text::bounded_title as shown;

pub fn badge(source: &str) -> String {
    match source {
        "yours" => "Yours".to_owned(),
        "file" => "Unverified · file".to_owned(),
        s if s.starts_with("official:") => "Official".to_owned(),
        s => match github_source(s) {
            Some((_, repo)) => format!("Unverified · {}", shown(repo)),
            None => "Unverified".to_owned(),
        },
    }
}

pub fn cards(store: &Store) -> Vec<Card> {
    let built_in =
        theme::BUILT_IN.iter().map(|(id, css)| (ThemeRef::BuiltIn(id), (*css).to_owned(), Source::BuiltIn, "Built in".to_owned(), None, true, false, None));
    let stored = store.themes().unwrap_or_default().into_iter().map(|t| {
        let withdrawn = withdrawn(store, &t.sha256, repo_of(&t));
        let badge = if withdrawn { "Withdrawn as unsafe".to_owned() } else { badge(&t.source) };
        let repo_id = github_source(&t.source).map(|(id, _)| id);
        (ThemeRef::Stored(t.id), t.css, theme::source_of(&t.source), badge, Some(t.id), t.page_css, withdrawn, repo_id)
    });
    built_in
        .chain(stored)
        .filter_map(|(id, css, source, badge, stored, page_css, withdrawn, repo_id)| {
            let p = theme::parse(&css, source).ok()?;
            let t = p.tokens;
            Some(Card {
                name: if p.meta.name.is_empty() { "Untitled".to_owned() } else { shown(&p.meta.name) },
                badge,
                scheme: p.meta.scheme,
                swatches: [t.sidebar.from, t.surface, t.accent, t.text],
                editable: source == Source::Yours,
                stored,
                hosts: p.sites.iter().flat_map(|s| s.hosts.iter().map(ToString::to_string)).collect(),
                warnings: p.warnings,
                adjusted: p.adjusted,
                page_css,
                unverified: source == Source::Unverified,
                withdrawn,
                repo_id,
                id,
            })
        })
        .collect()
}

pub fn select(store: &Store, id: ThemeRef, scheme: Scheme) -> Result<(), String> {
    let mut selection = Selection::load(store);
    if selection.match_system && scheme == Scheme::Light {
        selection.light = id;
    } else {
        selection.dark = id;
    }
    selection.save(store).map_err(|e| e.to_string())
}

pub fn set_match_system(store: &Store, on: bool) -> Result<(), String> {
    Selection { match_system: on, ..Selection::load(store) }.save(store).map_err(|e| e.to_string())
}

pub fn sha256(bytes: &[u8]) -> String {
    <sha2::Sha256 as sha2::Digest>::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn add(store: &Store, css: &str, now: i64) -> Result<Id, String> {
    let origin = Origin { sha256: sha256(css.as_bytes()), ..Default::default() };
    if withdrawn(store, &origin.sha256, None) {
        return Err("This theme was withdrawn as unsafe and cannot be added.".to_owned());
    }
    let id = theme::save(store, 0, "file", css, origin, now)?;
    let scheme = store.theme(id).ok().flatten().map_or(Scheme::Dark, |t| if t.scheme == "light" { Scheme::Light } else { Scheme::Dark });
    select(store, ThemeRef::Stored(id), scheme)?;
    Ok(id)
}

pub fn duplicate(store: &Store, id: &ThemeRef, now: i64) -> Result<Id, String> {
    if let ThemeRef::Stored(stored) = id
        && let Some(row) = store.theme(*stored).ok().flatten()
        && withdrawn(store, &row.sha256, repo_of(&row))
    {
        return Err("This theme was withdrawn as unsafe and cannot be copied.".to_owned());
    }
    theme::duplicate(store, id, now)
}

pub fn remove(store: &Store, id: Id) -> Result<(), String> {
    remember_withdrawn_repos(store);
    theme::remove(store, id).map_err(|e| e.to_string())
}

pub fn set_page_css(store: &Store, id: Id, on: bool) -> Result<(), String> {
    match store.theme(id) {
        Ok(Some(mut row)) => {
            row.page_css = on;
            store.put_theme(&row).map(drop).map_err(|e| e.to_string())
        }
        _ => Err("The theme is gone.".to_owned()),
    }
}

pub fn reset(store: &Store) -> Result<(), String> {
    Selection { dark: ThemeRef::DARK, light: ThemeRef::LIGHT, match_system: false }.save(store).map_err(|e| e.to_string())
}

pub fn exportable(store: &Store, id: &ThemeRef) -> Result<String, String> {
    if let ThemeRef::Stored(stored) = id
        && let Some(row) = store.theme(*stored).ok().flatten()
        && withdrawn(store, &row.sha256, repo_of(&row))
    {
        return Err("This theme was withdrawn as unsafe and cannot be saved.".to_owned());
    }
    theme::css_of(store, id).ok_or_else(|| "The theme is gone.".to_owned())
}

pub fn read_file(path: &Path) -> Result<String, String> {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) {
        return Err("Choose a .css file.".to_owned());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("The file could not be read: {e}"))?;
    let mut bytes = Vec::new();
    file.take(theme::MAX_THEME_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("The file could not be read: {e}"))?;
    read_bytes(&path.to_string_lossy(), bytes)
}

pub fn read_bytes(name: &str, bytes: Vec<u8>) -> Result<String, String> {
    if !Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) {
        return Err("Choose a .css file.".to_owned());
    }
    if bytes.len() > theme::MAX_THEME_BYTES {
        return Err(theme::ThemeError::TooLarge.to_string());
    }
    String::from_utf8(bytes).map_err(|_| "The file is not UTF-8 text.".to_owned())
}

pub fn hex(c: u32) -> String {
    if c & 0xff == 0xff { format!("#{:06x}", c >> 8) } else { format!("#{c:08x}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shield::revocations::Revocations;

    const MINE: &str = "@tarn-theme { name: Mine; scheme: light; } :root { --tarn-accent: #224466; --tarn-search-effect: flames } \
        @tarn-site example.org { a { color: #ff0000 } }";

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn an_added_theme_is_used_for_its_scheme_and_lists_unverified_with_styles_off() {
        let (_d, s) = store();
        let id = add(&s, MINE, 1).unwrap();
        assert_eq!(Selection::load(&s).dark, ThemeRef::Stored(id), "without Match system a light theme still shows");
        let active = current(&s, true, false);
        assert_eq!(active.id, ThemeRef::Stored(id));
        assert_eq!(search_effect(&s, &active), Effect::Flames);
        set_effect(&s, Some(Effect::None)).unwrap();
        assert_eq!((search_effect(&s, &active), chosen_effect(&s)), (Effect::None, Some(Effect::None)));
        set_effect(&s, None).unwrap();
        assert_eq!(chosen_effect(&s), None);
        let card = cards(&s).into_iter().find(|c| c.stored == Some(id)).unwrap();
        assert_eq!(
            (card.name.as_str(), card.badge.as_str(), card.unverified, card.page_css, card.scheme),
            ("Mine", "Unverified · file", true, false, Scheme::Light)
        );
        assert_eq!(card.hosts, ["example.org"]);
        assert!(profile_sheets(&s, &active, false, false, 1).is_empty(), "unverified styles start off");
        set_page_css(&s, id, true).unwrap();
        let active = current(&s, true, false);
        assert_eq!(profile_sheets(&s, &active, false, false, 1).len(), 1);
        assert!(profile_sheets(&s, &active, true, false, 1).is_empty(), "safe mode");
        assert!(profile_sheets(&s, &active, false, true, 1).is_empty(), "private");
        set_pages(&s, 1, false).unwrap();
        assert!(profile_sheets(&s, &active, false, false, 1).is_empty() && !pages_on(&s, 1) && pages_on(&s, 2));
        assert_eq!(current(&s, true, true).id, ThemeRef::DARK, "safe mode shows the built-in theme");
    }

    #[test]
    fn match_system_picks_by_appearance_and_reset_goes_back_to_tarn() {
        let (_d, s) = store();
        set_match_system(&s, true).unwrap();
        let id = add(&s, MINE, 1).unwrap();
        assert_eq!(Selection::load(&s).light, ThemeRef::Stored(id));
        assert_eq!(current(&s, true, false).id, ThemeRef::Stored(id));
        assert_eq!(current(&s, false, false).id, ThemeRef::DARK);
        select(&s, ThemeRef::BuiltIn("graphite"), Scheme::Dark).unwrap();
        assert_eq!(current(&s, false, false).id, ThemeRef::BuiltIn("graphite"));
        reset(&s).unwrap();
        assert_eq!(Selection::load(&s), Selection { dark: ThemeRef::DARK, light: ThemeRef::LIGHT, match_system: false });
    }

    #[test]
    fn withdrawn_themes_stop_showing_and_cannot_be_added_copied_or_saved() {
        let (_d, s) = store();
        let id = add(&s, MINE, 1).unwrap();
        let hash = sha256(MINE.as_bytes());
        assert!(!merge_revocations(&s, &Revocations { sha256: vec![] }));
        assert!(merge_revocations(&s, &Revocations { sha256: vec![hash.clone()] }));
        assert!(!merge_revocations(&s, &Revocations { sha256: vec![hash] }), "nothing new");
        let active = current(&s, false, false);
        assert_eq!(active.id, ThemeRef::LIGHT, "the built-in theme of its scheme");
        assert!(active.notice.unwrap().contains("withdrawn"));
        assert!(cards(&s).iter().any(|c| c.withdrawn && c.badge == "Withdrawn as unsafe"));
        assert!(duplicate(&s, &ThemeRef::Stored(id), 2).unwrap_err().contains("withdrawn"));
        assert!(exportable(&s, &ThemeRef::Stored(id)).is_err());
        remove(&s, id).unwrap();
        assert!(add(&s, MINE, 3).unwrap_err().contains("withdrawn"));
        assert!(exportable(&s, &ThemeRef::DARK).unwrap().contains("--tarn-"));
    }

    #[test]
    fn a_withdrawn_repository_stays_withdrawn_after_its_theme_is_removed() {
        let (_d, s) = store();
        let origin = Origin { repo_id: 7, owner_id: 8, tag: "v1".into(), sha256: "a".repeat(64) };
        let id = theme::save(&s, 0, "github:7:o/r", MINE, origin, 1).unwrap();
        assert_eq!(badge("github:7:o/r"), "Unverified · o/r");
        assert!(!withdrawn(&s, "", Some(7)));
        let copy = theme::duplicate(&s, &ThemeRef::Stored(id), 2).unwrap();
        let other = Origin { repo_id: 7, owner_id: 8, tag: "v2".into(), sha256: "b".repeat(64) };
        theme::save(&s, 0, "github:7:o/r", MINE, other, 1).unwrap();
        merge_revocations(&s, &Revocations { sha256: vec!["b".repeat(64)] });
        assert!(duplicate(&s, &ThemeRef::Stored(copy), 3).unwrap_err().contains("withdrawn"), "a copy keeps its repository");
        assert!(cards(&s).iter().any(|c| c.stored == Some(copy) && c.withdrawn));
        remove(&s, id).unwrap();
        assert!(withdrawn(&s, "", Some(7)) && !withdrawn(&s, "", Some(9)));
    }

    #[test]
    fn copies_are_yours_and_names_are_one_clean_line() {
        let (_d, s) = store();
        let copy = duplicate(&s, &ThemeRef::DARK, 1).unwrap();
        let card = cards(&s).into_iter().find(|c| c.stored == Some(copy)).unwrap();
        assert!(card.editable && card.badge == "Yours" && card.name.ends_with("Copy"));
        theme::set_color(&s, copy, "accent", "#0a7aff", 2).unwrap();
        assert_eq!(theme::parse(&theme::css_of(&s, &ThemeRef::Stored(copy)).unwrap(), Source::Yours).unwrap().tokens.accent, 0x0a7affff);
        assert_eq!(shown("a\u{202e}b\n  c\u{7}"), "ab c");
        assert_eq!((hex(0x1f75b0ff), hex(0x1f75b080)), ("#1f75b0".to_owned(), "#1f75b080".to_owned()));
        assert_eq!(badge("official:x"), "Official");
    }

    #[test]
    fn the_engines_take_as_much_theme_css_as_a_profile_gets() {
        assert_eq!(engine::page_scripts::MAX_THEME_SHEETS, store::MAX_SITE_CSS);
    }

    #[test]
    fn no_cached_bundle_withdraws_nothing() {
        let (dir, s) = store();
        assert!(!merge_cached_bundle(&s, &dir.path().join("filters")));
        assert!(revoked(&s).is_empty());
    }

    #[test]
    fn theme_files_must_be_small_css_text() {
        let dir = tempfile::tempdir().unwrap();
        let css = dir.path().join("t.css");
        std::fs::write(&css, MINE).unwrap();
        assert_eq!(read_file(&css).unwrap(), MINE);
        assert!(read_file(&dir.path().join("t.txt")).unwrap_err().contains(".css"));
        std::fs::write(&css, vec![b' '; theme::MAX_THEME_BYTES + 1]).unwrap();
        assert!(read_file(&css).is_err());
        std::fs::write(&css, [0xff, 0xfe]).unwrap();
        assert!(read_file(&css).unwrap_err().contains("UTF-8"));
        assert_eq!(read_bytes("Lagoon.CSS", b"a".to_vec()).unwrap(), "a");
        assert!(read_bytes("theme.txt", b"a".to_vec()).is_err());
        assert!(read_bytes("t.css", vec![b' '; theme::MAX_THEME_BYTES + 1]).is_err());
    }
}
