use std::path::Path;

use cef::*;

use super::{runtime, view};
use crate::chromium_owner;

/// Deletes everything Chromium keeps when `profile` owns it, and moves ownership to `to`.
/// Callers close the profile's Chromium views first; the cookie bridge is suspended here. A CEF
/// that never started loses its folder now; a live one has cookies, HTTP cache, sign-ins,
/// certificate exceptions and connections cleared, opens no new view, and its folder goes once
/// CEF is gone: at quit, or before CEF starts at the next launch.
pub fn wipe(root: &Path, profile: &str, to: &str) {
    if !chromium_owner::owns(root, profile) {
        return;
    }
    super::cookies::suspend_chromium(None);
    chromium_owner::transfer(root, profile, to);
    chromium_owner::take_clears(root);
    if runtime::idle() {
        chromium_owner::prepare(root, &[to.to_owned()]);
        return;
    }
    runtime::retire();
    view::close_all();
    runtime::with_context(|context| {
        if let Some(cookies) = context.cookie_manager(None) {
            cookies.delete_cookies(None, None, None);
        }
        context.clear_http_cache(None);
        context.clear_http_auth_credentials(None);
        context.clear_certificate_exceptions(None);
        context.close_all_connections(None);
    });
}

/// Removes `host`'s data from Chromium when `profile` owns it: cookies of the host, its parent
/// domains and its subdomains, storage of its origins on the default ports and of any open tab
/// on that host, and the whole HTTP cache, which Chromium cannot clear per site. While CEF is
/// not running or no Chromium tab is open, the host is recorded and cleared when CEF starts and
/// before the next tab's first load.
pub fn clear_site(root: &Path, profile: &str, host: &str) {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) || !chromium_owner::owns(root, profile) {
        return;
    }
    super::cookies::suspend_chromium(Some(&host));
    if runtime::running() {
        let cookie_host = host.clone();
        runtime::with_context(move |context| clear_cookies(&context, vec![cookie_host]));
        if view::clear_site_storage("", &host) {
            return;
        }
    }
    chromium_owner::record_clear(root, &host);
}

/// Forgets the certificates Chromium was told to continue past, so a removed exception asks again.
pub fn clear_cert_decisions() {
    if runtime::running() {
        runtime::with_context(|context| {
            context.clear_certificate_exceptions(None);
            context.close_all_connections(None);
            view::forget_continued("");
        });
    }
}

pub fn clear_cache(root: &Path, profile: &str) {
    if runtime::running() && chromium_owner::owns(root, profile) {
        runtime::with_context(|context| context.clear_http_cache(None));
    }
}

/// The recorded hosts, taken from the record: their storage is cleared now, and a host whose
/// clear fails goes back with `record`.
pub(crate) fn take_pending_sites() -> Vec<String> {
    runtime::root().map(|root| chromium_owner::take_clears(&root)).unwrap_or_default()
}

pub(crate) fn record(host: &str) {
    if let Some(root) = runtime::root() {
        chromium_owner::record_clear(&root, host);
    }
}

pub(crate) fn clear_pending_cookies() {
    let hosts = runtime::root().map(|root| chromium_owner::clears(&root)).unwrap_or_default();
    if let (false, Some(context)) = (hosts.is_empty(), request_context_get_global_context()) {
        clear_cookies(&context, hosts);
    }
}

fn clear_cookies(context: &RequestContext, hosts: Vec<String>) {
    if let Some(cookies) = context.cookie_manager(None) {
        cookies.visit_all_cookies(Some(&mut SiteCookies::new(hosts)));
    }
    context.clear_http_cache(None);
}

wrap_cookie_visitor! {
    struct SiteCookies {
        hosts: Vec<String>,
    }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&Cookie>, _count: i32, _total: i32, delete_cookie: Option<&mut i32>) -> i32 {
            if let (Some(cookie), Some(delete)) = (cookie, delete_cookie) {
                let domain = cookie.domain.to_string();
                let domain = domain.trim_start_matches('.');
                let matches = |host: &String| crate::page::site_matches(host, domain) || crate::page::site_matches(domain, host);
                if self.hosts.iter().any(matches) {
                    *delete = 1;
                }
            }
            1
        }
    }
}
