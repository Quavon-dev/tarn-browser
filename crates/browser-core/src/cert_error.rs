//! The certificate error page and the exceptions a user confirms on it (threat model S7, AGENTS.md
//! hard rule 7, "Certificate exceptions"). The engines report the failure and honour the
//! exceptions; what the page says, when it offers a way through and what is kept is decided here.

use shield::ShieldLevel;
use url::Url;
use x509_cert::Certificate;
use x509_cert::der::Decode;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;

pub use engine::cert::{Exception, Problem, fingerprint, host_port};

/// Characters kept of a name read from a certificate; the server chose it.
const MAX_NAME_CHARS: usize = 200;
const MAX_NAMES: usize = 50;

/// A main-frame load stopped by its certificate, shown in place of the page.
#[derive(Clone, Debug, PartialEq)]
pub struct CertError {
    pub url: Url,
    pub problem: Problem,
    /// DER, leaf first, as `engine::cert::bounded_chain` kept it.
    pub chain: Vec<Vec<u8>>,
}

/// How far the user went on the page: the way through sits under Details and needs a confirmation
/// (AGENTS.md hard rule 7, "Certificate exceptions").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Summary,
    Details,
    Confirm,
}

/// The certificate error page a tab shows, as every client keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct CertPage {
    pub error: CertError,
    pub step: Step,
    /// The certificate viewer is open.
    pub viewer: bool,
}

impl CertPage {
    pub fn new(error: CertError) -> Self {
        CertPage { error, step: Step::Summary, viewer: false }
    }

    /// Moves to `step`; a page that may not offer a way through stays on its summary.
    pub fn go(&mut self, step: Step, may_continue: bool) {
        self.step = if may_continue { step } else { Step::Summary };
    }

    /// The step to draw: the summary wherever the way through is not offered.
    pub fn shown(&self, may_continue: bool) -> Step {
        if may_continue { self.step } else { Step::Summary }
    }

    pub fn toggle_viewer(&mut self) {
        self.viewer = !self.viewer;
    }

    /// The exception Trust Certificate keeps: only from the confirmation, and only where the page
    /// may offer it. Ask again after the owner's check, as the tab may have changed meanwhile.
    pub fn trusted(&self, private: bool, level: ShieldLevel) -> Option<Exception> {
        (self.step == Step::Confirm && self.error.may_continue(private, level)).then(|| self.error.exception()).flatten()
    }
}

/// What the certificate viewer shows of one certificate.
#[derive(Clone, Debug, PartialEq)]
pub struct CertDetails {
    pub subject: String,
    pub organization: Option<String>,
    pub issuer: String,
    /// Milliseconds since the Unix epoch.
    pub not_before: i64,
    pub not_after: i64,
    /// DNS names and IP addresses the certificate is issued for.
    pub names: Vec<String>,
    /// Uppercase hex SHA-256, colon-separated pairs.
    pub fingerprint: String,
}

impl CertError {
    /// `host` or `host:port` when the port is not the scheme's default.
    pub fn site(&self) -> String {
        let host = self.url.host_str().unwrap_or_default();
        match self.url.port() {
            Some(port) => crate::text::bounded_origin(&format!("{host}:{port}")),
            None => crate::text::bounded_origin(host),
        }
    }

    pub fn details(&self) -> Vec<CertDetails> {
        self.chain.iter().filter_map(|der| details(der)).collect()
    }

    /// Whether the page may offer a way through: never for a revoked certificate, in a private
    /// space or for a site at Lockdown.
    pub fn may_continue(&self, private: bool, level: ShieldLevel) -> bool {
        self.problem.allows_exception() && !private && level != ShieldLevel::Lockdown && !self.chain.is_empty() && host_port(&self.url).is_some()
    }

    /// The exception confirming this certificate, if the page may offer one.
    pub fn exception(&self) -> Option<Exception> {
        let (host, port) = host_port(&self.url)?;
        Some(Exception { host, port, fingerprint: fingerprint(self.chain.first()?) })
    }

    pub fn heading(&self) -> &'static str {
        match self.problem {
            Problem::Revoked => "This site's certificate was revoked",
            Problem::Refused => "This site's certificate cannot be used",
            _ => "This connection is not private",
        }
    }

    /// Why the page did not load; `computer` names the device whose trust settings apply.
    pub fn reason(&self, computer: &str) -> String {
        let site = self.site();
        let leaf = self.chain.first().and_then(|der| details(der));
        match self.problem {
            Problem::Untrusted => match leaf.filter(|l| l.issuer != l.subject) {
                Some(leaf) => format!("{site} uses a certificate issued by “{}”, which this {computer} does not trust.", in_sentence(&leaf.issuer)),
                None => format!("{site} uses a certificate that it signed itself, which this {computer} does not trust."),
            },
            Problem::Dates => match leaf {
                Some(leaf) if leaf.not_before > crate::now() => {
                    format!("{site}'s certificate is not valid until {}.", crate::shield_card::date(leaf.not_before))
                }
                Some(leaf) => format!("{site}'s certificate expired on {}.", crate::shield_card::date(leaf.not_after)),
                None => format!("{site}'s certificate has expired or is not valid yet."),
            },
            Problem::NameMismatch => match leaf.and_then(|l| l.names.into_iter().next().or(Some(l.subject))).filter(|n| !n.is_empty()) {
                Some(name) => format!("The certificate is for “{}”, not for {site}.", in_sentence(&name)),
                None => format!("The certificate is not for {site}."),
            },
            Problem::Revoked => format!("{site}'s certificate was withdrawn by its issuer, so Tarn will not open this site."),
            Problem::Refused => format!(
                "{site}'s certificate is invalid, does not match the keys this site is known to use, or is known to be used to intercept connections, so Tarn will not open this site."
            ),
            Problem::Invalid => format!("{site}'s certificate is not valid."),
        }
    }

    pub const WARNING: &'static str = "Someone on the network may be pretending to be this site to read what you send, such as passwords. A device on your own network, such as a router or server, often uses a certificate like this.";

    pub fn continue_label(&self) -> String {
        format!("Continue to {}…", self.site())
    }

    /// The text of the confirmation before an exception is kept.
    pub fn confirm_message(&self) -> String {
        format!(
            "Tarn will trust this certificate for {} in this profile and open the site without asking again. Only do this for a site you control or know, such as a device on your own network. You can remove it in Settings › Permissions.",
            self.site()
        )
    }

    /// The reason the system's owner check shows, after "Tarn is trying to".
    pub fn owner_reason(&self) -> String {
        format!("trust the certificate of {}", self.site())
    }
}

/// Reads what the viewer shows; `None` for bytes that are not an X.509 certificate.
pub fn details(der: &[u8]) -> Option<CertDetails> {
    if der.len() > engine::cert::MAX_CERT_BYTES {
        return None;
    }
    let cert = Certificate::from_der(der).ok()?;
    let tbs = cert.tbs_certificate();
    let name = |n: &x509_cert::name::Name| {
        n.common_name().ok().flatten().map(|cn| shown(&cn.value())).filter(|s| !s.is_empty()).unwrap_or_else(|| shown(&n.to_string()))
    };
    let names = match tbs.get_extension::<SubjectAltName>() {
        Ok(Some((_, san))) => san.0.iter().filter_map(general_name).take(MAX_NAMES).collect(),
        _ => Vec::new(),
    };
    let ms = |t: x509_cert::time::Time| i64::try_from(t.to_unix_duration().as_millis()).unwrap_or(i64::MAX);
    Some(CertDetails {
        subject: name(tbs.subject()),
        organization: tbs.subject().organization().ok().flatten().map(|o| shown(&o.value())).filter(|s| !s.is_empty()),
        issuer: name(tbs.issuer()),
        not_before: ms(tbs.validity().not_before),
        not_after: ms(tbs.validity().not_after),
        names,
        fingerprint: colon_hex(&fingerprint(der)),
    })
}

fn general_name(name: &GeneralName) -> Option<String> {
    match name {
        GeneralName::DnsName(dns) => Some(shown(dns.as_ref())),
        GeneralName::IpAddress(ip) => match ip.as_bytes() {
            &[a, b, c, d] => Some(std::net::Ipv4Addr::new(a, b, c, d).to_string()),
            bytes => <[u8; 16]>::try_from(bytes).ok().map(|b| std::net::Ipv6Addr::from(b).to_string()),
        },
        _ => None,
    }
}

/// A name from the certificate, without control, bidi or invisible characters, and bounded.
fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() && !store::is_invisible(*c)).take(MAX_NAME_CHARS).collect()
}

/// A server-chosen name inside Tarn's own sentence: no quotes it could close, and short enough
/// that it cannot pass for more of the sentence; the viewer shows it whole.
fn in_sentence(name: &str) -> String {
    const MAX: usize = 64;
    let kept: String = name.chars().filter(|c| !matches!(c, '"' | '“' | '”' | '„' | '«' | '»' | '\'' | '‘' | '’')).collect();
    if kept.chars().count() > MAX { format!("{}…", kept.chars().take(MAX).collect::<String>()) } else { kept }
}

fn colon_hex(hex: &str) -> String {
    hex.as_bytes().chunks(2).map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase()).collect::<Vec<_>>().join(":")
}

/// The profile's exceptions as the engines take them.
pub fn exceptions(store: &store::Store, profile: store::Id) -> Vec<Exception> {
    store.cert_exceptions(profile).unwrap_or_default().into_iter().map(|(host, port, fingerprint)| Exception { host, port, fingerprint }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DER: a tag, its length and its content.
    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        match content.len() {
            n @ 0..=0x7f => out.push(n as u8),
            n @ 0x80..=0xff => out.extend([0x81, n as u8]),
            n => out.extend([0x82, (n >> 8) as u8, n as u8]),
        }
        out.extend_from_slice(content);
        out
    }

    fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
        tlv(0x30, &parts.concat())
    }

    /// OBJECT IDENTIFIERs written as their DER content bytes.
    const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03]; // 2.5.4.3
    const ORGANIZATION: &[u8] = &[0x55, 0x04, 0x0a]; // 2.5.4.10
    const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11]; // 2.5.29.17
    const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]; // 1.2.840.10045.2.1
    const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]; // 1.2.840.10045.3.1.7
    const ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]; // 1.2.840.10045.4.3.2

    fn name(common: &str, organization: &str) -> Vec<u8> {
        let attribute = |oid: &[u8], value: &str| tlv(0x31, &seq(&[tlv(0x06, oid), tlv(0x0c, value.as_bytes())]));
        seq(&[attribute(COMMON_NAME, common), attribute(ORGANIZATION, organization)])
    }

    /// A self-signed-looking certificate for pve.local, 192.168.101.101 and ::1, valid 2024-01-01
    /// to 2034-01-01. Its key and signature are placeholders: parsing never checks them.
    fn pve() -> Vec<u8> {
        let subject = name("pve.local", "Proxmox Virtual Environment");
        let algorithm = seq(&[tlv(0x06, ECDSA_SHA256)]);
        let validity = seq(&[tlv(0x17, b"240101000000Z"), tlv(0x17, b"340101000000Z")]);
        let key = seq(&[seq(&[tlv(0x06, EC_PUBLIC_KEY), tlv(0x06, PRIME256V1)]), tlv(0x03, &[[0x00, 0x04].as_slice(), &[7; 64]].concat())]);
        let mut ipv6 = [0u8; 16];
        ipv6[15] = 1;
        let names = seq(&[tlv(0x82, b"pve.local"), tlv(0x87, &[192, 168, 101, 101]), tlv(0x87, &ipv6)]);
        let extensions = tlv(0xa3, &seq(&[seq(&[tlv(0x06, SUBJECT_ALT_NAME), tlv(0x04, &names)])]));
        let version = tlv(0xa0, &tlv(0x02, &[2]));
        let tbs = seq(&[version, tlv(0x02, &[1]), algorithm.clone(), subject.clone(), validity, subject, key, extensions]);
        seq(&[tbs, algorithm, tlv(0x03, &[0x00, 0x30, 0x00])])
    }

    fn error(problem: Problem) -> CertError {
        CertError { url: Url::parse("https://192.168.101.101:8006/#v1:0").unwrap(), problem, chain: vec![pve()] }
    }

    #[test]
    fn a_certificate_shows_its_names_dates_and_fingerprint() {
        let d = details(&pve()).unwrap();
        assert_eq!(d.subject, "pve.local");
        assert_eq!(d.issuer, "pve.local");
        assert_eq!(d.organization.as_deref(), Some("Proxmox Virtual Environment"));
        assert_eq!(d.names, ["pve.local", "192.168.101.101", "::1"]);
        assert_eq!(crate::shield_card::date(d.not_before), "2024-01-01");
        assert_eq!(crate::shield_card::date(d.not_after), "2034-01-01");
        assert_eq!(d.fingerprint.len(), 32 * 3 - 1);
        assert_eq!(d.fingerprint.replace(':', "").to_ascii_lowercase(), fingerprint(&pve()));
        assert!(details(b"not a certificate").is_none());
        assert!(details(&vec![0x30; engine::cert::MAX_CERT_BYTES + 1]).is_none());
    }

    #[test]
    fn names_lose_control_and_invisible_characters_and_are_bounded() {
        assert_eq!(shown("a\u{202E}b\u{200B}c\n"), "abc");
        assert_eq!(shown(&"x".repeat(500)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn the_way_through_is_offered_only_where_the_rule_allows() {
        let e = error(Problem::Untrusted);
        assert!(e.may_continue(false, ShieldLevel::Standard));
        assert!(e.may_continue(false, ShieldLevel::Strict));
        assert!(!e.may_continue(true, ShieldLevel::Standard));
        assert!(!e.may_continue(false, ShieldLevel::Lockdown));
        assert!(!error(Problem::Revoked).may_continue(false, ShieldLevel::Standard));
        assert!(!CertError { chain: Vec::new(), ..e.clone() }.may_continue(false, ShieldLevel::Standard));
        let ex = e.exception().unwrap();
        assert_eq!((ex.host.as_str(), ex.port, ex.fingerprint), ("192.168.101.101", 8006, fingerprint(&pve())));
    }

    #[test]
    fn the_page_reaches_its_confirmation_only_where_the_rule_allows() {
        let mut page = CertPage::new(error(Problem::Untrusted));
        assert_eq!(page.trusted(false, ShieldLevel::Standard), None, "not from the summary");
        page.go(Step::Details, true);
        assert_eq!(page.trusted(false, ShieldLevel::Standard), None, "not from the details");
        page.go(Step::Confirm, true);
        assert_eq!(page.trusted(false, ShieldLevel::Standard), error(Problem::Untrusted).exception());
        assert_eq!(page.trusted(true, ShieldLevel::Standard), None, "never in a private space");
        assert_eq!(page.trusted(false, ShieldLevel::Lockdown), None, "never at Lockdown");
        assert_eq!(page.shown(false), Step::Summary);
        page.go(Step::Confirm, false);
        assert_eq!(page.step, Step::Summary);
        let mut revoked = CertPage { step: Step::Confirm, ..CertPage::new(error(Problem::Revoked)) };
        assert_eq!(revoked.trusted(false, ShieldLevel::Standard), None);
        revoked.toggle_viewer();
        assert!(revoked.viewer);
    }

    #[test]
    fn the_page_names_the_site_and_the_problem() {
        let e = error(Problem::Untrusted);
        assert_eq!(e.site(), "192.168.101.101:8006");
        assert_eq!(e.reason("Mac"), "192.168.101.101:8006 uses a certificate that it signed itself, which this Mac does not trust.");
        assert_eq!(error(Problem::NameMismatch).reason("Mac"), "The certificate is for “pve.local”, not for 192.168.101.101:8006.");
        assert_eq!(in_sentence("X”, which is safe. Continue “Y"), "X, which is safe. Continue Y");
        assert_eq!(in_sentence(&"a".repeat(80)).chars().count(), 65);
        assert!(error(Problem::Revoked).reason("Mac").contains("will not open"));
        assert_eq!(CertError { url: Url::parse("https://a.test/").unwrap(), ..e }.site(), "a.test");
    }
}
