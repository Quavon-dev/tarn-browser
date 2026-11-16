use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use shield::dnr::Scope;
use store::{Id, InstalledExtension};

use super::manifest::Manifest;
use crate::extensions::every_site;

pub const ONCE: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
    /// The extension wants the host and the user has not answered yet in this session.
    Ask,
}

/// What one install may do, from its row, the global "every site" switch, the answers stored for
/// it and the profile's Lockdown hosts.
pub struct Install<'a> {
    pub row: &'a InstalledExtension,
    pub manifest: &'a Manifest,
    pub every_site_pref: bool,
    pub stored: &'a [(String, bool)],
    pub lockdown_hosts: &'a [String],
}

fn plain(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The user's answer to the tab-modal site prompt (`PromptKind::ExtensionAccess`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub profile: Id,
    pub install_id: String,
    pub host: String,
    pub allow: bool,
    pub remember: bool,
    pub tab: Option<u64>,
}

/// Answers given in this session: Once grants (an hour, or until their tab closes), Deny without
/// Remember, and which hosts were asked about. Kept per profile and install.
#[derive(Default)]
pub struct Session {
    once: HashMap<(Id, String, String), (Instant, Option<u64>)>,
    denied: HashSet<(Id, String, String)>,
    asked: HashSet<(Id, String, String)>,
}

impl Session {
    pub fn access(&self, install: &Install, host: &str, now: Instant) -> Access {
        let host = plain(host);
        let key = (install.row.profile_id, install.row.install_id.clone(), host.clone());
        if host.is_empty() || install.lockdown_hosts.iter().any(|h| plain(h) == host) || !engine_plain_host(&host) {
            return Access::Denied;
        }
        match install.stored.iter().find(|(h, _)| plain(h) == host) {
            Some((_, false)) => return Access::Denied,
            Some((_, true)) => return Access::Granted,
            None => {}
        }
        if self.denied.contains(&key) {
            return Access::Denied;
        }
        if self.once.get(&key).is_some_and(|(until, _)| *until > now) {
            return Access::Granted;
        }
        if !install.manifest.wants_host(&host) {
            return Access::Denied;
        }
        if every_site(install.row.all_sites, install.every_site_pref, &install.row.source) {
            return Access::Granted;
        }
        if self.asked.contains(&key) { Access::Denied } else { Access::Ask }
    }

    /// Marks `host` as asked about, so the prompt shows once per session; false if it was already.
    pub fn ask(&mut self, profile: Id, install_id: &str, host: &str) -> bool {
        self.asked.insert((profile, install_id.to_owned(), plain(host)))
    }

    /// An answer to the site prompt. A remembered one is returned for the shell to store with
    /// `Store::set_extension_host`; a Once allow lasts `ONCE` or until its tab closes.
    pub fn answer(&mut self, a: &Answer, now: Instant) -> Option<(String, bool)> {
        let key = (a.profile, a.install_id.clone(), plain(&a.host));
        self.asked.insert(key.clone());
        if a.remember {
            self.once.remove(&key);
            self.denied.remove(&key);
            return Some((key.2, a.allow));
        }
        if a.allow {
            self.once.insert(key, (now + ONCE, a.tab));
        } else {
            self.denied.insert(key);
        }
        None
    }

    pub fn tab_closed(&mut self, tab: u64) {
        self.once.retain(|_, (_, t)| *t != Some(tab));
    }

    pub fn forget(&mut self, profile: Id, install_id: Option<&str>) {
        let ours = |k: &(Id, String, String)| k.0 == profile && install_id.is_none_or(|i| i == k.1);
        self.once.retain(|k, _| !ours(k));
        self.denied.retain(|k| !ours(k));
        self.asked.retain(|k| !ours(k));
    }

    /// Where the install's network rules may act: every site with the trusted switch, else the
    /// hosts granted now. A Lockdown-level view never gets extension rules, so a Lockdown host is
    /// covered by `Every` only for requests of other pages.
    pub fn scope(&self, install: &Install, now: Instant) -> Scope {
        if every_site(install.row.all_sites, install.every_site_pref, &install.row.source) {
            return Scope::Every;
        }
        let profile = install.row.profile_id;
        let mut hosts: Vec<String> = install.stored.iter().filter(|(_, allow)| *allow).map(|(h, _)| plain(h)).collect();
        hosts.extend(
            self.once.iter().filter(|((p, i, _), (until, _))| *p == profile && *i == install.row.install_id && *until > now).map(|((_, _, h), _)| h.clone()),
        );
        hosts.retain(|h| self.access(install, h, now) == Access::Granted);
        hosts.sort();
        hosts.dedup();
        Scope::Hosts(hosts)
    }

    /// The soonest Once grant to expire, when the shell must compute scopes again.
    pub fn next_expiry(&self, now: Instant) -> Option<Instant> {
        self.once.values().map(|(until, _)| *until).filter(|u| *u > now).min()
    }
}

fn engine_plain_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)) && !host.starts_with('.')
}
