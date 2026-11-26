#![no_main]

use browser_core::cert_error::{CertError, details};
use engine::cert::{MAX_CERT_BYTES, Problem, bounded_chain};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(d) = details(data) {
        assert!(data.len() <= MAX_CERT_BYTES);
        assert!(!d.subject.chars().any(char::is_control) && !d.issuer.chars().any(char::is_control));
        assert_eq!(d.fingerprint.len(), 95);
    }
    let chain = bounded_chain(data.chunks(97).map(<[u8]>::to_vec));
    let page = CertError { url: url::Url::parse("https://192.168.1.2:8006/").unwrap(), problem: Problem::Untrusted, chain };
    let _ = (page.details(), page.reason("Mac"));
});
