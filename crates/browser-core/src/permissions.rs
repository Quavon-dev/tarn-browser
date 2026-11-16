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
fn site_name(origin: &str) -> String {
    let host = |o: &str| Url::parse(o).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| o.to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;

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
        set(&store, profile, "https://b.test", POPUPS, Some(true)).unwrap();
        set(&store, profile, "https://meet.test", "camera+microphone", None).unwrap();
        let rows = list(&store, profile);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].allowed);
    }
}
