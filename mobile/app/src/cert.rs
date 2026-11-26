use browser_core::cert_error::{self, CertError, CertPage, Exception, Problem, Step};
use browser_core::suggest;
use browser_core::tabs::TabKey;
use engine::cert::{MAX_CERT_BYTES, MAX_CHAIN, bounded_chain};
use serde_json::Value;
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 2 * MAX_CERT_BYTES || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// The `certificate_error` event as the engines send it: the URL, the problem (or Android's
/// primary error) and the certificates as hex DER, leaf first.
pub fn parse_error(data: &Value) -> Option<CertError> {
    let url = Url::parse(data.get("url")?.as_str()?).ok()?;
    let problem = match data.get("primary").and_then(Value::as_i64) {
        Some(primary) => Problem::from_android(primary),
        None => Problem::parse(data.get("problem").and_then(Value::as_str).unwrap_or_default()),
    };
    let chain = data.get("chain").and_then(Value::as_array).into_iter().flatten().take(MAX_CHAIN).map(|c| c.as_str().and_then(hex).unwrap_or_default());
    Some(CertError { url, problem, chain: bounded_chain(chain) })
}

impl<D: Platform> Shell<D> {
    pub(crate) fn certificate_error(&mut self, key: TabKey, data: &Value) {
        let (Some(error), Some(page)) = (parse_error(data), self.pages.get_mut(&key)) else { return };
        page.loading = false;
        page.error = None;
        suggest::typed_visit(&mut page.typed, error.url.as_str());
        page.cert = Some(CertPage::new(error));
    }

    fn cert_level(&self, key: TabKey, page: &CertPage) -> shield::ShieldLevel {
        self.model.level_for(&self.core, key, page.error.url.as_str())
    }

    /// Whether `key`'s certificate error page may offer a way through.
    pub fn cert_may_continue(&self, key: TabKey) -> bool {
        self.pages.get(&key).and_then(|p| p.cert.as_ref()).is_some_and(|c| c.error.may_continue(key.private, self.cert_level(key, c)))
    }

    pub fn set_cert_step(&mut self, step: Step) {
        let Some(key) = self.model.active else { return };
        let may_continue = self.cert_may_continue(key);
        if let Some(page) = self.pages.get_mut(&key).and_then(|p| p.cert.as_mut()) {
            page.go(step, may_continue);
        }
    }

    /// Go Back: leaves the engine's own error page where it committed one (Android); otherwise the
    /// failed load never became a history entry (iOS) and the card only closes.
    pub fn leave_cert_page(&mut self) -> Vec<Effect> {
        let back = self.active_page().is_some_and(|p| p.engine_error_page && p.back);
        self.dismiss_error();
        if back { self.active_command("back", Value::Null) } else { Vec::new() }
    }

    pub fn toggle_cert_viewer(&mut self) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)).and_then(|p| p.cert.as_mut()) {
            page.toggle_viewer();
        }
    }

    /// "Trust Certificate": the active tab's page once it may keep its certificate, marked as
    /// waiting for the owner check so a second press does not ask again.
    pub fn begin_cert_trust(&mut self) -> Option<(TabKey, CertPage)> {
        let key = self.model.active?;
        let page = self.pages.get(&key).filter(|p| !p.cert_asking)?.cert.clone()?;
        page.trusted(key.private, self.cert_level(key, &page))?;
        self.pages.get_mut(&key)?.cert_asking = true;
        Some((key, page))
    }

    /// The owner check answered; `confirmed` keeps the certificate.
    pub fn end_cert_trust(&mut self, key: TabKey, page: &CertPage, confirmed: bool) {
        if let Some(state) = self.pages.get_mut(&key) {
            state.cert_asking = false;
        }
        if confirmed {
            self.keep_cert_exception(key, page);
        } else {
            self.notice = Some("Tarn could not confirm it is you, so the certificate was not trusted.".into());
        }
    }

    /// Keeps this exact certificate for the site in the tab's profile and loads the page again.
    fn keep_cert_exception(&mut self, key: TabKey, page: &CertPage) {
        if self.pages.get(&key).and_then(|p| p.cert.as_ref()) != Some(page) {
            return;
        }
        let (Some(profile), Some(exception)) = (self.model.profile_of(&self.core, key), page.trusted(key.private, self.cert_level(key, page))) else {
            return;
        };
        if let Err(e) = self.core.disk.set_cert_exception(profile, &exception.host, exception.port, &exception.fingerprint) {
            self.notice = Some(format!("The certificate could not be saved ({e})."));
            return;
        }
        if let Some(state) = self.pages.get_mut(&key) {
            state.cert = None;
        }
        self.load_in(key, page.error.url.as_str(), None);
    }

    /// The current profile's exceptions; a private space never has any. Every read also updates
    /// the profile's `Exceptions`, which remembers each site that had one until quit.
    pub fn cert_exceptions(&self) -> Vec<Exception> {
        let Some((false, profile)) = self.model.profile() else { return Vec::new() };
        let list = cert_error::exceptions(&self.core.disk, profile);
        self.cert_seen.borrow_mut().entry(profile).or_default().set(list.clone());
        list
    }

    /// Whether the active page loads, or may still be connected, under a certificate exception:
    /// shown as not secure until quit.
    pub fn active_cert_excepted(&self) -> bool {
        let Some((false, profile)) = self.model.profile() else { return false };
        let Ok(url) = Url::parse(&self.active_url()) else { return false };
        if !self.cert_seen.borrow().contains_key(&profile) {
            self.cert_exceptions();
        }
        self.cert_seen.borrow().get(&profile).is_some_and(|seen| seen.covers(&url))
    }

    /// The host and port of the active page, for its site popover's Remove.
    pub fn remove_active_cert_exception(&mut self) {
        if let Some((host, port)) = Url::parse(&self.active_url()).ok().and_then(|u| cert_error::host_port(&u)) {
            self.remove_cert_exception(&host, port);
        }
    }

    /// The page asks again on its next load.
    pub fn remove_cert_exception(&mut self, host: &str, port: u16) {
        if let Some((false, profile)) = self.model.profile() {
            self.core.disk.delete_cert_exception(profile, host, port).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_event_reads_hex_certificates_within_bounds() {
        let event = json!({ "url": "https://192.168.1.2:8006/", "problem": "untrusted", "chain": ["30a0", "3", "zz", "+f", ""] });
        let cert = parse_error(&event).unwrap();
        assert_eq!((cert.problem, cert.chain), (Problem::Untrusted, vec![vec![0x30, 0xa0]]));
        assert_eq!(parse_error(&json!({ "url": "https://a.test/", "primary": 3 })).unwrap().problem, Problem::Untrusted);
        assert_eq!(parse_error(&json!({ "url": "https://a.test/", "problem": "other" })).unwrap().problem, Problem::Invalid);
        let long = "00".repeat(MAX_CERT_BYTES + 1);
        assert!(parse_error(&json!({ "url": "https://a.test/", "chain": [long] })).unwrap().chain.is_empty());
        let many: Vec<&str> = vec!["01"; MAX_CHAIN + 5];
        assert_eq!(parse_error(&json!({ "url": "https://a.test/", "chain": many })).unwrap().chain.len(), MAX_CHAIN);
        assert!(parse_error(&json!({ "url": "not a url" })).is_none());
    }
}
