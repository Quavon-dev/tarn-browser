//! Encrypted sync file format and merge core (threat model S19). Not used by the app yet.
#![forbid(unsafe_code)]

mod file;
mod model;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use engine::nav_policy::{Decision, Source, decide};
use shield::ShieldLevel;
use url::Url;

pub use file::{KdfParams, MAX_FILE_LEN, SALT_LEN, SyncKey, header_kdf, open, open_with_key, seal, seal_with_key};
pub use model::{Folder, Meta, Setting, Snapshot, Space, SyncError, SyncId, Tab, TabKind};

pub const MAX_SPACES: usize = 100;
pub const MAX_FOLDERS: usize = 2_000;
pub const MAX_TABS: usize = 10_000;
pub const MAX_SETTINGS: usize = 1_000;
pub const MAX_TEXT_LEN: usize = 1_024;
pub const MAX_ICON_LEN: usize = 64;
pub const MAX_URL_LEN: usize = 8_192;
/// How far ahead of this device's clock a remote timestamp may be.
pub const MAX_CLOCK_SKEW_MS: i64 = 24 * 60 * 60 * 1000;

trait Item: Ord {
    fn key(&self) -> &str;
    fn meta(&self) -> &Meta;
}

macro_rules! item {
    ($t:ty, $key:ident) => {
        impl Item for $t {
            fn key(&self) -> &str {
                &self.$key
            }
            fn meta(&self) -> &Meta {
                &self.meta
            }
        }
    };
}
item!(Space, id);
item!(Folder, id);
item!(Tab, id);
item!(Setting, key);

fn merge_items<T: Item>(local: Vec<T>, remote: Vec<T>) -> Vec<T> {
    let mut out: BTreeMap<String, T> = BTreeMap::new();
    for item in local.into_iter().chain(remote) {
        match out.get(item.key()) {
            Some(cur) if (cur.meta(), cur) >= (item.meta(), &item) => {}
            _ => {
                out.insert(item.key().to_owned(), item);
            }
        }
    }
    out.into_values().collect()
}

/// Per-item last-writer-wins by `(updated_at, device, deleted, item)`. Commutative, associative
/// and idempotent, so devices converge whatever order they read files in. Keeps `local`'s
/// `device` and `seq`.
pub fn merge(local: Snapshot, remote: Snapshot) -> Snapshot {
    Snapshot {
        device: local.device,
        seq: local.seq,
        spaces: merge_items(local.spaces, remote.spaces),
        folders: merge_items(local.folders, remote.folders),
        tabs: merge_items(local.tabs, remote.tabs),
        settings: merge_items(local.settings, remote.settings),
    }
}

fn check_id(id: &str) -> Result<(), SyncError> {
    let hex = id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if hex { Ok(()) } else { Err(SyncError::BadId) }
}

fn check_len(s: &str, max: usize) -> Result<(), SyncError> {
    if s.len() <= max { Ok(()) } else { Err(SyncError::TooLong) }
}

fn check_count<T>(items: &[T], max: usize) -> Result<(), SyncError> {
    if items.len() <= max { Ok(()) } else { Err(SyncError::TooMany) }
}

fn check_meta(meta: &Meta, now: i64) -> Result<(), SyncError> {
    check_id(&meta.device)?;
    if (0..=now.saturating_add(MAX_CLOCK_SKEW_MS)).contains(&meta.updated_at) { Ok(()) } else { Err(SyncError::BadTime) }
}

/// Normalises an allowed URL in place. Tombstones keep no URL, so a delete still propagates.
fn keep_tab(tab: &mut Tab) -> bool {
    if tab.meta.deleted {
        tab.url.clear();
        return true;
    }
    match Url::parse(&tab.url) {
        Ok(url) if decide(&url, Source::Sync) == Decision::Allow => {
            tab.url = url.into();
            true
        }
        _ => false,
    }
}

fn setting_allowed(s: &Setting) -> bool {
    let host_ok = |host: &str| Url::parse(&format!("https://{host}/")).is_ok_and(|u| u.host_str() == Some(host));
    let key_ok = s.key == "shield_level" || s.key.strip_prefix("site_override:").is_some_and(host_ok);
    key_ok && (s.meta.deleted || ShieldLevel::parse(&s.value).is_some())
}

fn keep_setting(s: &mut Setting) -> bool {
    if s.meta.deleted {
        s.value.clear();
    }
    setting_allowed(s)
}

/// Checks a decrypted snapshot from another device. Oversized collections or strings and malformed
/// ids reject the whole snapshot; tabs whose URL `nav_policy` does not allow for [`Source::Sync`]
/// and unknown or invalid settings are dropped.
pub fn validate(snapshot: Snapshot) -> Result<Snapshot, SyncError> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    validate_at(snapshot, now)
}

fn validate_at(mut snapshot: Snapshot, now: i64) -> Result<Snapshot, SyncError> {
    check_count(&snapshot.spaces, MAX_SPACES)?;
    check_count(&snapshot.folders, MAX_FOLDERS)?;
    check_count(&snapshot.tabs, MAX_TABS)?;
    check_count(&snapshot.settings, MAX_SETTINGS)?;
    check_id(&snapshot.device)?;
    for s in &snapshot.spaces {
        check_id(&s.id)?;
        check_meta(&s.meta, now)?;
        check_len(&s.name, MAX_TEXT_LEN)?;
        check_len(&s.icon, MAX_ICON_LEN)?;
    }
    for f in &snapshot.folders {
        check_id(&f.id)?;
        check_id(&f.space)?;
        f.parent.as_deref().map_or(Ok(()), check_id)?;
        check_meta(&f.meta, now)?;
        check_len(&f.name, MAX_TEXT_LEN)?;
    }
    for t in &snapshot.tabs {
        check_id(&t.id)?;
        check_id(&t.space)?;
        t.folder.as_deref().map_or(Ok(()), check_id)?;
        check_meta(&t.meta, now)?;
        check_len(&t.title, MAX_TEXT_LEN)?;
        check_len(&t.url, MAX_URL_LEN)?;
    }
    for s in &snapshot.settings {
        check_meta(&s.meta, now)?;
        check_len(&s.key, MAX_TEXT_LEN)?;
        check_len(&s.value, MAX_TEXT_LEN)?;
    }
    snapshot.tabs.retain_mut(keep_tab);
    snapshot.settings.retain_mut(keep_setting);
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const S1: &str = "00000000000000000000000000000001";
    const T1: &str = "00000000000000000000000000000002";

    fn meta(updated_at: i64, device: &str, deleted: bool) -> Meta {
        Meta { updated_at, device: device.into(), deleted }
    }

    fn tab(url: &str, title: &str, meta: Meta) -> Tab {
        Tab { id: T1.into(), space: S1.into(), folder: None, kind: TabKind::Pinned, url: url.into(), title: title.into(), position: 0, meta }
    }

    pub(crate) fn sample() -> Snapshot {
        Snapshot {
            device: A.into(),
            seq: 7,
            spaces: vec![Space { id: S1.into(), name: "Work".into(), icon: "".into(), position: 0, meta: meta(1, A, false) }],
            folders: vec![Folder {
                id: "00000000000000000000000000000003".into(),
                space: S1.into(),
                parent: None,
                name: "Docs".into(),
                position: 0,
                meta: meta(1, A, false),
            }],
            tabs: vec![tab("https://example.com/", "Example", meta(1, A, false))],
            settings: vec![
                Setting { key: "shield_level".into(), value: "strict".into(), meta: meta(1, A, false) },
                Setting { key: "site_override:example.com".into(), value: "lockdown".into(), meta: meta(1, A, false) },
            ],
        }
    }

    fn with_tabs(tabs: Vec<Tab>) -> Snapshot {
        Snapshot { tabs, ..sample() }
    }

    #[test]
    fn valid_sample_passes_unchanged() {
        assert_eq!(validate(sample()), Ok(sample()));
    }

    #[test]
    fn dangerous_urls_dropped() {
        let urls = [
            "javascript:alert(1)",
            "JAVASCRIPT:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "blob:https://example.com/1",
            "tarn://settings",
            "mailto:a@example.com",
            "not a url",
        ];
        for url in urls {
            let out = validate(with_tabs(vec![tab(url, "", meta(1, A, false))])).unwrap();
            assert!(out.tabs.is_empty(), "{url}");
        }
        let ok = validate(with_tabs(vec![tab("http://localhost:3000/", "", meta(1, A, false))])).unwrap();
        assert_eq!(ok.tabs.len(), 1);
    }

    #[test]
    fn caps_and_ids_reject() {
        let long_title = tab("https://a.test/", &"x".repeat(MAX_TEXT_LEN + 1), meta(1, A, false));
        assert_eq!(validate(with_tabs(vec![long_title])), Err(SyncError::TooLong));
        let long_url = tab(&format!("https://a.test/{}", "x".repeat(MAX_URL_LEN)), "", meta(1, A, false));
        assert_eq!(validate(with_tabs(vec![long_url])), Err(SyncError::TooLong));
        let many = vec![tab("https://a.test/", "", meta(1, A, false)); MAX_TABS + 1];
        assert_eq!(validate(with_tabs(many)), Err(SyncError::TooMany));
        let bad_device = tab("https://a.test/", "", meta(1, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", false));
        assert_eq!(validate(with_tabs(vec![bad_device])), Err(SyncError::BadId));
        assert_eq!(validate(Snapshot { device: "../x".into(), ..sample() }), Err(SyncError::BadId));
    }

    #[test]
    fn urls_normalised_and_tombstones_kept_without_url() {
        let out = validate(with_tabs(vec![tab(" https://EXAMPLE.com/a\tb\n", "", meta(1, A, false))])).unwrap();
        assert_eq!(out.tabs[0].url, "https://example.com/ab");
        let out = validate(with_tabs(vec![tab("javascript:alert(1)", "", meta(1, A, true))])).unwrap();
        assert_eq!(out.tabs[0].url, "");
    }

    #[test]
    fn timestamps_bounded() {
        let at = |t: i64| validate_at(with_tabs(vec![tab("https://a.test/", "", meta(t, A, false))]), 1_000);
        assert!(at(1_000 + MAX_CLOCK_SKEW_MS).is_ok());
        assert_eq!(at(1_001 + MAX_CLOCK_SKEW_MS), Err(SyncError::BadTime));
        assert_eq!(at(i64::MAX), Err(SyncError::BadTime));
        assert_eq!(at(-1), Err(SyncError::BadTime));
    }

    #[test]
    fn unknown_or_invalid_settings_dropped() {
        let setting = |key: &str, value: &str| Setting { key: key.into(), value: value.into(), meta: meta(1, A, false) };
        let snap = Snapshot {
            settings: vec![
                setting("ai_enabled", "true"),
                setting("shield_level", "off"),
                setting("site_override:Example.COM", "strict"),
                setting("site_override:a.test/evil", "strict"),
                setting("site_override:", "strict"),
                setting("site_override:a.test", "standard"),
            ],
            ..sample()
        };
        let keys: Vec<_> = validate(snap).unwrap().settings.into_iter().map(|s| s.key).collect();
        assert_eq!(keys, ["site_override:a.test"]);

        let removed = Setting { meta: meta(2, A, true), ..setting("site_override:a.test", "none") };
        let out = validate(Snapshot { settings: vec![removed], ..sample() }).unwrap();
        assert_eq!(out.settings[0].value, "");
    }

    #[test]
    fn every_collection_is_checked() {
        let bad_id = || "X".repeat(32);
        let long = || "x".repeat(MAX_TEXT_LEN + 1);
        let s = sample();
        let cases = [
            (Snapshot { spaces: vec![Space { id: bad_id(), ..s.spaces[0].clone() }], ..sample() }, SyncError::BadId),
            (Snapshot { spaces: vec![Space { name: long(), ..s.spaces[0].clone() }], ..sample() }, SyncError::TooLong),
            (Snapshot { spaces: vec![Space { icon: "x".repeat(MAX_ICON_LEN + 1), ..s.spaces[0].clone() }], ..sample() }, SyncError::TooLong),
            (Snapshot { spaces: vec![Space { meta: meta(-1, A, false), ..s.spaces[0].clone() }], ..sample() }, SyncError::BadTime),
            (Snapshot { spaces: vec![s.spaces[0].clone(); MAX_SPACES + 1], ..sample() }, SyncError::TooMany),
            (Snapshot { folders: vec![Folder { space: bad_id(), ..s.folders[0].clone() }], ..sample() }, SyncError::BadId),
            (Snapshot { folders: vec![Folder { parent: Some(bad_id()), ..s.folders[0].clone() }], ..sample() }, SyncError::BadId),
            (Snapshot { folders: vec![Folder { name: long(), ..s.folders[0].clone() }], ..sample() }, SyncError::TooLong),
            (Snapshot { folders: vec![s.folders[0].clone(); MAX_FOLDERS + 1], ..sample() }, SyncError::TooMany),
            (with_tabs(vec![Tab { folder: Some(bad_id()), ..s.tabs[0].clone() }]), SyncError::BadId),
            (Snapshot { settings: vec![Setting { value: long(), ..s.settings[0].clone() }], ..sample() }, SyncError::TooLong),
            (Snapshot { settings: vec![Setting { meta: meta(1, &bad_id(), false), ..s.settings[0].clone() }], ..sample() }, SyncError::BadId),
            (Snapshot { settings: vec![s.settings[0].clone(); MAX_SETTINGS + 1], ..sample() }, SyncError::TooMany),
        ];
        for (i, (snapshot, expected)) in cases.into_iter().enumerate() {
            assert_eq!(validate(snapshot), Err(expected), "case {i}");
        }
    }

    #[test]
    fn merge_last_writer_wins() {
        let old = with_tabs(vec![tab("https://old.test/", "", meta(1, A, false))]);
        let new = Snapshot { device: B.into(), ..with_tabs(vec![tab("https://new.test/", "", meta(2, B, false))]) };
        assert_eq!(merge(old.clone(), new.clone()).tabs[0].url, "https://new.test/");
        assert_eq!(merge(new.clone(), old.clone()).tabs[0].url, "https://new.test/");
        assert_eq!(merge(old.clone(), new).device, A);
    }

    #[test]
    fn tombstones() {
        let live = with_tabs(vec![tab("https://a.test/", "", meta(1, A, false))]);
        let deleted = with_tabs(vec![tab("https://a.test/", "", meta(2, B, true))]);
        let recreated = with_tabs(vec![tab("https://a.test/", "", meta(3, A, false))]);
        assert!(merge(live.clone(), deleted.clone()).tabs[0].meta.deleted);
        assert!(merge(deleted.clone(), live).tabs[0].meta.deleted);
        assert!(!merge(deleted, recreated).tabs[0].meta.deleted);
    }

    #[test]
    fn tie_break_is_deterministic() {
        let a = with_tabs(vec![tab("https://a.test/", "", meta(5, A, false))]);
        let b = with_tabs(vec![tab("https://b.test/", "", meta(5, B, false))]);
        assert_eq!(merge(a.clone(), b.clone()).tabs, merge(b.clone(), a.clone()).tabs);
        assert_eq!(merge(a.clone(), b).tabs[0].meta.device, B);

        let live = with_tabs(vec![tab("https://a.test/", "", meta(5, A, false))]);
        let gone = with_tabs(vec![tab("https://a.test/", "", meta(5, A, true))]);
        assert!(merge(live.clone(), gone.clone()).tabs[0].meta.deleted);
        assert!(merge(gone, live).tabs[0].meta.deleted);

        let x = with_tabs(vec![tab("https://a.test/", "x", meta(5, A, false))]);
        let y = with_tabs(vec![tab("https://a.test/", "y", meta(5, A, false))]);
        assert_eq!(merge(x.clone(), y.clone()).tabs, merge(y, x).tabs);
    }

    #[test]
    fn merge_is_idempotent_and_associative() {
        let a = sample();
        let b = Snapshot {
            device: B.into(),
            spaces: vec![Space { id: S1.into(), name: "Home".into(), icon: "".into(), position: 1, meta: meta(9, B, false) }],
            tabs: vec![tab("https://b.test/", "", meta(0, B, false))],
            ..Snapshot::default()
        };
        let c = with_tabs(vec![tab("https://c.test/", "", meta(3, B, true))]);
        assert_eq!(merge(a.clone(), a.clone()), a);
        assert_eq!(merge(merge(a.clone(), b.clone()), c.clone()), merge(a.clone(), merge(b, c)),);
    }
}
