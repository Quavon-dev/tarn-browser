//! The rule-list groups built from a filter-list bundle, shared by every shell.

use std::path::Path;

use crate::bundle::{FILTER_BUNDLE_PUBLIC_KEY, Kind, VerifiedBundle};
use crate::{Category, cache, lists, rules};

/// Groups built from the bundle, in install order.
pub const GROUPS: [&str; 5] = ["ads", "trackers", "cosmetic", "malware", "annoyances"];
/// The setting that switches each group; cosmetic filtering follows the ads switch.
pub const SETTINGS: [(&str, &str); 5] =
    [("ads", "block_ads"), ("cosmetic", "block_ads"), ("trackers", "block_trackers"), ("malware", "block_malware"), ("annoyances", "block_annoyances")];

/// The bundle signing key. Debug builds take `TARN_TEST_FILTER_KEY` (64 hex digits) instead, so
/// tests can install bundles they signed themselves; release builds never read it.
pub fn public_key() -> [u8; 32] {
    #[cfg(debug_assertions)]
    if let Some(key) = std::env::var("TARN_TEST_FILTER_KEY").ok().and_then(|hex| parse_key(&hex)) {
        return key;
    }
    FILTER_BUNDLE_PUBLIC_KEY
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn parse_key(hex: &str) -> Option<[u8; 32]> {
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
    bytes.try_into().ok()
}

pub fn hex(fingerprint: [u8; 32]) -> String {
    fingerprint.iter().map(|b| format!("{b:02x}")).collect()
}

fn texts(bundle: &VerifiedBundle, category: Category, kind: Kind) -> Vec<String> {
    bundle.of(category.as_str(), kind).map(|l| String::from_utf8_lossy(&l.data).into_owned()).collect()
}

/// The bundle's text inputs of each rule-list group.
pub struct Inputs {
    pub ads: Vec<String>,
    pub trackers: Vec<String>,
    pub annoyances: Vec<String>,
    pub malware: String,
}

impl Inputs {
    pub fn new(bundle: &VerifiedBundle) -> Self {
        Inputs {
            ads: texts(bundle, Category::Ads, Kind::Filters),
            trackers: texts(bundle, Category::Trackers, Kind::Filters),
            annoyances: texts(bundle, Category::Annoyances, Kind::Filters),
            malware: texts(bundle, Category::Malware, Kind::Domains).concat(),
        }
    }

    pub fn refs(lists: &[String]) -> Vec<&str> {
        lists.iter().map(String::as_str).collect()
    }

    /// Names the group's compiled lists; changes with the input and with the converter.
    pub fn key(&self, group: &str) -> String {
        let (ads, trackers, annoyances) = (Self::refs(&self.ads), Self::refs(&self.trackers), Self::refs(&self.annoyances));
        hex(match group {
            "ads" => lists::network_key(&ads, &trackers),
            "trackers" => lists::network_key(&trackers, &ads),
            "cosmetic" => lists::cosmetic_key(&ads),
            "annoyances" => lists::network_key(&annoyances, &[ads, trackers].concat()),
            _ => lists::fingerprint(&[&self.malware, rules::DOMAIN_RULES_VERSION]),
        })
    }

    /// Ads and trackers each carry the other's exceptions, so either switch works alone. Annoyances
    /// are one group of network and element-hiding rules and carry both lists' exceptions, so
    /// uBlock Unbreak's fixes for consent scripts also lift cookie-list blocks.
    pub fn convert(&self, group: &str) -> Vec<String> {
        let (ads, trackers, annoyances) = (Self::refs(&self.ads), Self::refs(&self.trackers), Self::refs(&self.annoyances));
        match group {
            "ads" => lists::compile(&ads, &trackers).chunks,
            "trackers" => lists::compile(&trackers, &ads).chunks,
            "cosmetic" => lists::compile_cosmetic(&ads).chunks,
            "annoyances" => [lists::compile(&annoyances, &[ads, trackers].concat()).chunks, lists::compile_cosmetic(&annoyances).chunks].concat(),
            _ => rules::domain_rules(&self.malware),
        }
    }
}

/// One group's content-blocker source, named by its key.
pub struct Group {
    pub name: &'static str,
    pub key: String,
    pub chunks: Vec<String>,
}

/// Every group of the bundle cached in `dir`, converted.
pub fn load(dir: &Path, key: &[u8; 32]) -> Result<Vec<Group>, String> {
    let bundle = cache::read(dir, key).map_err(|e| format!("{e:?}"))?;
    Ok(of(&bundle))
}

pub fn of(bundle: &VerifiedBundle) -> Vec<Group> {
    let inputs = Inputs::new(bundle);
    GROUPS.into_iter().map(|name| Group { name, key: inputs.key(name), chunks: inputs.convert(name) }).collect()
}

/// The groups whose switch is on; every switch defaults to on.
pub fn switched_on(setting: impl Fn(&str) -> Option<String>) -> Vec<String> {
    SETTINGS.iter().filter(|(_, key)| setting(key).is_none_or(|v| v != "off")).map(|(group, _)| (*group).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::VerifiedList;

    fn list(category: &str, kind: Kind, data: &[u8]) -> VerifiedList {
        VerifiedList { name: format!("{category}.x"), category: category.into(), kind, count: 2, data: data.to_vec() }
    }

    fn bundle() -> VerifiedBundle {
        VerifiedBundle {
            version: 1,
            created: String::new(),
            lists: vec![
                list("ads", Kind::Filters, b"||ads.example^\n@@||ok-ad.example^\n##.banner\n"),
                list("trackers", Kind::Filters, b"||tracker.example^\n@@||ok-tracker.example^\n"),
                list("malware", Kind::Domains, b"evil.example\n"),
                list("annoyances", Kind::Filters, b"||consent.example^\n##.cookie-banner\n"),
            ],
        }
    }

    #[test]
    fn each_group_converts_its_own_input() {
        let inputs = Inputs::new(&bundle());
        let ads = inputs.convert("ads").concat();
        assert!(ads.contains("ads\\\\.example") && ads.contains("ok-tracker") && !ads.contains("||tracker"), "{ads}");
        let trackers = inputs.convert("trackers").concat();
        assert!(trackers.contains("tracker\\\\.example") && trackers.contains("ok-ad") && !trackers.contains("\"ads\\\\.example"), "{trackers}");
        assert!(inputs.convert("cosmetic").concat().contains(".banner"));
        let annoyances = inputs.convert("annoyances");
        assert_eq!(annoyances.len(), 2);
        assert!(annoyances[0].contains("consent\\\\.example") && annoyances[0].contains("ok-ad") && annoyances[1].contains(".cookie-banner"), "{annoyances:?}");
        assert!(!inputs.convert("cosmetic").concat().contains("cookie-banner") && !inputs.convert("ads").concat().contains("consent"));
        assert!(inputs.convert("malware").concat().contains("evil\\\\.example"));
        let keys: Vec<String> = GROUPS.iter().map(|g| inputs.key(g)).collect();
        assert!(keys.iter().all(|k| k.len() == 64));
        assert_eq!(keys.iter().collect::<std::collections::BTreeSet<_>>().len(), keys.len());
        assert_eq!(inputs.key("ads"), Inputs::new(&bundle()).key("ads"));
    }

    #[test]
    fn groups_follow_the_blocking_switches_which_default_on() {
        assert_eq!(switched_on(|_| None), ["ads", "cosmetic", "trackers", "malware", "annoyances"]);
        let off = |k: &str| (k == "block_ads").then(|| "off".to_owned());
        assert_eq!(switched_on(off), ["trackers", "malware", "annoyances"]);
    }

    #[test]
    fn no_bundle_without_a_signing_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), &[0; 32]).is_err());
        assert_eq!(parse_key(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(parse_key("abc"), None);
    }
}
