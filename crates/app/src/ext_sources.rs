use std::sync::Arc;
use std::time::Duration;

pub use browser_core::github::*;
use ureq::tls::{RootCerts, TlsConfig};
use url::Url;

/// HTTPS only, no proxy, no credentials or cookies, a fixed `User-Agent`; redirects are followed
/// by hand and every hop must be on `HOSTS`.
pub struct Net(ureq::Agent);

impl Net {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(false)
            .proxy(None)
            .user_agent(USER_AGENT)
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(CONNECT_TIMEOUT_SECS)))
            .timeout_global(Some(Duration::from_secs(TIMEOUT_SECS)))
            .build();
        Net(agent.into())
    }
}

impl Hop for Net {
    fn request(&self, url: &Url, max: u64) -> Result<Reply, FetchError> {
        let mut response = self.0.get(url.as_str()).call().map_err(|e| FetchError::Failed(e.to_string()))?;
        let status = response.status().as_u16();
        let location = response.headers().get("location").and_then(|l| l.to_str().ok()).map(str::to_owned);
        let body =
            if status == 200 { response.body_mut().with_config().limit(max).read_to_vec().map_err(|e| FetchError::Failed(e.to_string()))? } else { Vec::new() };
        Ok(Reply { status, location, body })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn the_transport_refuses_hosts_outside_the_list() {
        assert!(matches!(Net::new().get("https://evil.test/x", 10), Err(FetchError::Refused(_))));
    }

    #[test]
    #[ignore = "asks GitHub"]
    fn ublock_origin_lite_resolves_downloads_and_unpacks() {
        let net = Net::new();
        let r = resolve(&net, &parse_github("uBlockOrigin/uBOL-home").unwrap()).unwrap();
        let asset = r.assets.first().expect("a chromium asset");
        let bytes = browser_core::extensions::download(&net, &asset.url).unwrap();
        let dir = tempfile::tempdir().unwrap();
        crate::ext_package::unpack(&bytes, &dir.path().join("1"), false).unwrap();
        assert!(dir.path().join("1/manifest.json").exists(), "{}", asset.name);
        crate::ext_package::remove_tree(&dir.path().join("1")).unwrap();
    }

    #[test]
    fn every_request_goes_through_the_one_fetch_function() {
        let source = include_str!("ext_sources.rs");
        let code = &source[..source.find("#[cfg(test)]").unwrap()];
        assert_eq!(code.matches(".call()").count(), 1);
        assert!(code.contains("impl Hop for Net"));
        assert_eq!(code.matches("config_builder").count(), 1);
        assert!(!code.contains("ureq::get") && !code.contains("std::net"));
        for installer in [include_str!("extensions.rs"), include_str!("themes.rs"), include_str!("settings/appearance.rs")] {
            assert!(!installer.contains("ureq") && !installer.contains(".call("));
        }
    }
}
