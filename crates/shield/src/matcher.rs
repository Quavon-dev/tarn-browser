//! In-process request matching over the filter lists WebKit's rule lists are compiled from, for
//! engines that ask per request instead of loading rule lists.

use std::sync::Arc;

use adblock::Engine;
use adblock::request::Request;

use crate::lists::{HideSheets, network_set};

/// Ads and trackers each carry the other's exceptions, as `lists::compile` does, so either switch
/// works alone; annoyances carry both, so an unbreak exception in the ad lists also lifts a
/// cookie-list block. Element hiding follows the ads and annoyances switches, as the `cosmetic`
/// and `annoyances` rule lists do. Cloning shares the engines.
#[derive(Clone, Default)]
pub struct Matcher {
    ads: Option<Arc<Engine>>,
    trackers: Option<Arc<Engine>>,
    annoyances: Option<Arc<Engine>>,
    ads_hide: Option<Arc<HideSheets>>,
    annoyances_hide: Option<Arc<HideSheets>>,
}

impl Matcher {
    pub fn new(ads: &[&str], trackers: &[&str], annoyances: &[&str]) -> Self {
        let engine = |lists: &[&str], shared: &[&str]| Some(Arc::new(Engine::new_with_filter_set(network_set(lists, shared, false))));
        // Only Windows Chromium reads element hiding from here, so other clients skip building it.
        let hide = |lists: &[&str]| cfg!(windows).then(|| Arc::new(HideSheets::new(lists)));
        Matcher {
            ads: engine(ads, trackers),
            trackers: engine(trackers, ads),
            annoyances: engine(annoyances, &[ads, trackers].concat()),
            ads_hide: hide(ads),
            annoyances_hide: hide(annoyances),
        }
    }

    /// The same engines with the switched-off groups left out.
    pub fn only(&self, ads: bool, trackers: bool, annoyances: bool) -> Self {
        Matcher {
            ads: self.ads.clone().filter(|_| ads),
            trackers: self.trackers.clone().filter(|_| trackers),
            annoyances: self.annoyances.clone().filter(|_| annoyances),
            ads_hide: self.ads_hide.clone().filter(|_| ads),
            annoyances_hide: self.annoyances_hide.clone().filter(|_| annoyances),
        }
    }

    /// The switched-on element hiding for a page on `host` as a style sheet; empty without any.
    pub fn hide_css(&self, host: &str) -> String {
        [&self.ads_hide, &self.annoyances_hide].into_iter().flatten().map(|h| h.css(host)).collect()
    }

    /// Whether `url`, requested by a page at `source` as `kind` (adblock's request type names,
    /// such as `script`, `xhr`, `sub_frame` or `main_frame`), is blocked by a group. A request the
    /// matcher cannot read is blocked, except a main frame, which the navigation gate judges.
    pub fn blocks(&self, url: &str, source: &str, kind: &str) -> bool {
        let Ok(request) = Request::new(url, source, kind, "GET") else {
            return kind != "main_frame" && (self.ads.is_some() || self.trackers.is_some() || self.annoyances.is_some());
        };
        [&self.ads, &self.trackers, &self.annoyances].into_iter().flatten().any(|e| e.check_network_request(&request).should_block())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_request_is_blocked_unless_it_is_a_page() {
        let on = Matcher::new(&[ADS], &[TRACKERS], &[]);
        assert!(on.blocks("not a url", "https://a.test/", "script"));
        assert!(!on.blocks("not a url", "https://a.test/", "main_frame"));
        assert!(!Matcher::default().blocks("not a url", "https://a.test/", "script"));
    }

    const ADS: &str = "||ads.example^\n@@||ok-ad.example^\n";
    const TRACKERS: &str = "||tracker.example^$third-party\n||pixel.example^\n@@||pixel.example/allowed^\n";

    #[cfg(windows)]
    #[test]
    fn hiding_follows_the_ads_and_annoyances_switches() {
        let m = Matcher::new(&["##.ad\n"], &["##.tracker\n"], &["example.com##.cookies\n"]);
        assert_eq!(m.hide_css("example.com"), ".ad{display:none!important}\n.cookies{display:none!important}\n");
        assert_eq!(m.only(false, true, true).hide_css("example.com"), ".cookies{display:none!important}\n");
        assert_eq!(m.only(true, true, false).hide_css("other.test"), ".ad{display:none!important}\n");
        assert!(m.only(false, true, false).hide_css("example.com").is_empty());
    }

    #[test]
    fn is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Matcher>();
    }

    #[test]
    fn keeps_rules_whose_domains_idna_rejects() {
        let m = Matcher::new(&["||ads.example^$domain=~a.\u{751}le.org\n"], &[], &[]);
        assert!(m.blocks("https://ads.example/x.js", "https://news.test/", "script"));
    }

    #[test]
    fn blocks_listed_third_party_requests() {
        let m = Matcher::new(&[ADS], &[TRACKERS], &[]);
        let page = "https://news.test/article";
        let cases = [
            ("https://tracker.example/t.js", page, "script", true),
            ("https://cdn.tracker.example/p.gif", page, "image", true),
            ("https://ads.example/banner.js", page, "script", true),
            ("https://ads.example/feed", page, "xhr", true),
            ("wss://ads.example/socket", page, "websocket", true),
            ("https://tracker.example/app.js", "https://tracker.example/", "script", false),
            ("https://pixel.example/allowed/x.gif", page, "image", false),
            ("https://news.test/app.js", page, "script", false),
            ("https://ok-ad.example/x.js", page, "script", false),
            ("not a url", page, "script", true),
        ];
        for (url, source, kind, blocked) in cases {
            assert_eq!(m.blocks(url, source, kind), blocked, "{url} from {source} as {kind}");
        }
    }

    #[test]
    fn switched_off_groups_and_shared_exceptions() {
        let m = Matcher::new(&[ADS], &["||ok-ad.example^\n||tracker.example^\n"], &["||consent.example^\n"]);
        let page = "https://news.test/";
        assert!(!m.blocks("https://ok-ad.example/x.js", page, "script"), "an ads exception reaches the tracker group");
        assert!(m.blocks("https://tracker.example/x.js", page, "script"));
        assert!(m.blocks("https://consent.example/cmp.js", page, "script"));
        let unbroken = Matcher::new(&["@@||consent.example^\n"], &[], &["||consent.example^\n"]);
        assert!(!unbroken.blocks("https://consent.example/cmp.js", page, "script"), "an ads exception reaches the annoyances group");
        let ads_only = m.only(true, false, false);
        assert!(!ads_only.blocks("https://tracker.example/x.js", page, "script"));
        assert!(!ads_only.blocks("https://consent.example/cmp.js", page, "script"));
        assert!(ads_only.blocks("https://ads.example/x.js", page, "script"));
        assert!(!Matcher::default().blocks("https://ads.example/x.js", page, "script"));
    }
}
