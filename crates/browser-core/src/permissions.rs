use store::{Id, Store};
use url::Url;

use crate::site::POPUPS;

/// A site permission remembered for a profile: who asked, for what, and the answer.
#[derive(Clone, Debug, PartialEq)]
pub struct SitePermission {
    pub origin: String,
    pub kind: String,
    /// The host that asked, and the page it was embedded in when it was a frame of another site.
    pub site: String,
    pub label: String,
    pub allowed: bool,
}

pub fn kind_label(kind: &str) -> String {
    match kind {
        "camera" => "Camera".into(),
        "microphone" => "Microphone".into(),
        "camera+microphone" => "Camera and Microphone".into(),
        "geolocation" => "Location".into(),
        POPUPS => "Pop-ups".into(),
        "extension" => "Extension access".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// Stored origins are `https://a.test` or, for a frame, `https://frame.test in https://page.test`.
/// A name keeps a port and a scheme other than https, so two origins never read the same.
fn site_name(origin: &str) -> String {
    let host = |o: &str| {
        let Some(url) = Url::parse(o).ok().filter(|u| u.host_str().is_some()) else { return o.to_owned() };
        let scheme = if url.scheme() == "https" { String::new() } else { format!("{}://", url.scheme()) };
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        format!("{scheme}{}{port}", url.host_str().unwrap_or_default())
    };
    match origin.split_once(" in ") {
        Some((frame, page)) => format!("{} in {}", host(frame), host(page)),
        None => host(origin),
    }
}

/// Every permission the profile remembers, by site and then kind.
pub fn list(store: &Store, profile_id: Id) -> Vec<SitePermission> {
    let mut rows: Vec<SitePermission> = store
        .permissions(profile_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(origin, kind, decision)| SitePermission { site: site_name(&origin), label: kind_label(&kind), allowed: decision == "allow", origin, kind })
        .collect();
    rows.sort_by(|a, b| (&a.site, &a.label).cmp(&(&b.site, &b.label)));
    rows
}

/// Allows, blocks or (with `None`) forgets a remembered permission.
pub fn set(store: &Store, profile_id: Id, origin: &str, kind: &str, allow: Option<bool>) -> Result<(), store::Error> {
    match allow {
        Some(allow) => store.set_permission(profile_id, origin, kind, if allow { "allow" } else { "deny" }),
        None => store.delete_permission(profile_id, origin, kind),
    }
}

/// The answers a site's permission row offers, as stored and as labelled.
pub const DECISIONS: [(&str, &str); 3] = [("allow", "Allow"), ("ask", "Ask"), ("deny", "Deny")];

/// A remembered permission for the page's origin or for a frame embedded in it.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRow {
    pub origin: String,
    pub kind: String,
    pub label: String,
    pub decision: String,
}

/// The rows stored for `page` itself, then those of frames embedded in it, named by their host.
pub fn permission_rows(stored: Vec<(String, String, String)>, page: &str) -> Vec<PermissionRow> {
    let embedded = format!(" in {page}");
    let mut rows: Vec<(bool, PermissionRow)> = stored
        .into_iter()
        .filter_map(|(origin, kind, decision)| {
            let frame = match origin.strip_suffix(&embedded) {
                Some(frame) => Some(frame.to_owned()),
                None if origin == page => None,
                None => return None,
            };
            let label = match &frame {
                Some(frame) => format!("{} — {}", kind_label(&kind), Url::parse(frame).ok()?.host_str()?),
                None => kind_label(&kind),
            };
            Some((frame.is_some(), PermissionRow { origin, kind, label: crate::text::bounded_title(&label), decision }))
        })
        .collect();
    rows.sort_by_key(|(embedded, _)| *embedded);
    rows.into_iter().map(|(_, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(origin: &str, kind: &str, decision: &str) -> (String, String, String) {
        (origin.into(), kind.into(), decision.into())
    }

    #[test]
    fn rows_belong_to_the_page_and_its_frames() {
        let rows = permission_rows(
            vec![
                stored("https://meet.test in https://a.test", "camera", "allow"),
                stored("https://a.test", "microphone", "deny"),
                stored("https://other.test", "camera", "allow"),
                stored("https://a.test in https://other.test", "camera", "allow"),
                stored("https://a.test", "camera+microphone", "ask"),
            ],
            "https://a.test",
        );
        let labels: Vec<(&str, &str)> = rows.iter().map(|r| (r.label.as_str(), r.decision.as_str())).collect();
        assert_eq!(labels, [("Microphone", "deny"), ("Camera and Microphone", "ask"), ("Camera — meet.test", "allow")]);
        assert_eq!(rows[2].origin, "https://meet.test in https://a.test");
    }

    #[test]
    fn permissions_are_listed_by_site_with_readable_names() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "uuid", false).unwrap().id;
        store.set_permission(profile, "https://meet.test", "camera+microphone", "allow").unwrap();
        store.set_permission(profile, "https://b.test", POPUPS, "deny").unwrap();
        store.set_permission(profile, "https://map.test in https://a.test", "geolocation", "allow").unwrap();
        let rows = list(&store, profile);
        let got: Vec<(&str, &str, bool)> = rows.iter().map(|r| (r.site.as_str(), r.label.as_str(), r.allowed)).collect();
        assert_eq!(got, [("b.test", "Pop-ups", false), ("map.test in a.test", "Location", true), ("meet.test", "Camera and Microphone", true)]);
        assert_eq!(kind_label("notifications"), "Notifications");
        assert_eq!(site_name("https://localhost:3000"), "localhost:3000");
        assert_eq!(site_name("http://localhost:8443 in https://a.test"), "http://localhost:8443 in a.test");
        set(&store, profile, "https://b.test", POPUPS, Some(true)).unwrap();
        set(&store, profile, "https://meet.test", "camera+microphone", None).unwrap();
        let rows = list(&store, profile);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].allowed);
    }
}
