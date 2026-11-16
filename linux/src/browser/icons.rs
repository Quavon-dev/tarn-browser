use std::collections::HashMap;
use std::sync::Arc;

use browser_core::now;
use browser_core::tabs::TabKey;
use engine::favicon;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Id;
use url::Url;

use super::{Browser, Request};

pub type Profile = (bool, Id);

/// One profile's site icons as `data:` URIs by host, for the UI; `stamp` changes with them.
#[derive(Clone, Debug, Default)]
pub struct SiteIcons {
    stamp: Option<(Profile, u64)>,
    hosts: Arc<HashMap<String, Arc<str>>>,
}

impl SiteIcons {
    pub fn find(&self, host: &str) -> Option<Arc<str>> {
        favicon::lookup_hosts(host).iter().find_map(|h| self.hosts.get(h)).cloned()
    }
}

impl PartialEq for SiteIcons {
    fn eq(&self, other: &Self) -> bool {
        self.stamp == other.stamp
    }
}

impl Browser {
    fn load_favicons(&mut self, (private, id): Profile) {
        let store = self.core.store(private);
        self.favicons.load(&(private, id), || {
            let icons = store.favicons(id).unwrap_or_default().into_iter().map(|(host, png)| (host, Arc::from(favicon::data_uri(&png)))).collect();
            (icons, store.stale_favicons(id, now() - favicon::MAX_AGE).unwrap_or_default())
        });
    }

    /// The current profile's icons, or `None` while `shown` still has them.
    pub fn site_icons(&mut self, shown: &SiteIcons) -> Option<SiteIcons> {
        let profile = self.model.profile();
        if let Some(profile) = profile {
            self.load_favicons(profile);
        }
        let stamp = profile.map(|p| (p, self.favicons.version()));
        (shown.stamp != stamp).then(|| SiteIcons {
            stamp,
            hosts: Arc::new(profile.map(|p| self.favicons.icons(&p).map(|(h, i)| (h.to_owned(), i.clone())).collect()).unwrap_or_default()),
        })
    }

    pub(super) fn request_favicon(&mut self, key: TabKey, url: &Url) {
        let lockdown = self.level_for(key, url.as_str()) == ShieldLevel::Lockdown;
        let (Some(id), Some(host)) = (self.model.profile_of(&self.core, key), favicon::icon_host(url, lockdown)) else { return };
        let profile = (key.private, id);
        self.load_favicons(profile);
        let live: Vec<TabKey> = self.model.lifecycle.live().collect();
        self.withdraw_icons(|k| !live.contains(&k));
        if self.favicons.request(&profile, &host) {
            self.ask(key, Request::IconLinks(key, profile, host), "readIconLinks", json!({}));
        }
    }

    /// A tab whose view went away never answers, so its requests must not hold their host.
    pub(super) fn withdraw_icons(&mut self, gone: impl Fn(TabKey) -> bool) {
        let seqs: Vec<u64> =
            self.requests.iter().filter(|(_, r)| matches!(r, Request::IconLinks(k, ..) | Request::IconData(k, ..) if gone(*k))).map(|(s, _)| *s).collect();
        for seq in seqs {
            if let Some(Request::IconLinks(_, profile, host) | Request::IconData(_, profile, host)) = self.requests.remove(&seq) {
                self.favicons.withdraw(&profile, &host);
            }
        }
    }

    /// An `iconLinks` answer, or an `iconData` answer with the icon `favicon::normalize` made of it.
    pub fn on_icon_event(&mut self, key: TabKey, data: &Value, png: Option<Vec<u8>>) {
        let icon = |r: &Request| matches!(r, Request::IconLinks(..) | Request::IconData(..));
        let Some(request) = data["seq"].as_u64().filter(|s| self.requests.get(s).is_some_and(icon)).and_then(|s| self.requests.remove(&s)) else { return };
        let text = data["text"].as_str().unwrap_or_default();
        match request {
            Request::IconLinks(_, profile, host) => {
                let page = Url::parse(data["url"].as_str().unwrap_or_default()).ok();
                let shown = page.as_ref().filter(|p| self.page_url(key).is_some_and(|u| u.origin() == p.origin()));
                let urls: Vec<String> = shown.map(|p| favicon::candidates(p, text)).unwrap_or_default().into_iter().map(String::from).collect();
                if urls.is_empty() || !self.favicons.is_requested(&profile, &host) {
                    return self.favicons.withdraw(&profile, &host);
                }
                self.ask(key, Request::IconData(key, profile, host), "fetchIcon", json!({ "urls": urls }));
            }
            Request::IconData(_, profile, host) => {
                let uri = png.as_deref().map(|p| Arc::from(favicon::data_uri(p)));
                if self.favicons.accept(&profile, &host, uri)
                    && let Some(png) = png.filter(|_| self.core.keeps_records(profile.0, profile.1))
                {
                    self.core.store(profile.0).set_favicon(profile.1, &host, &png).ok();
                }
            }
            _ => {}
        }
    }
}
