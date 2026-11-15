use std::rc::Rc;

use browser_core::WebsiteData;
use engine::Event;
use engine::cookies::Cookie;

pub const PRIVATE_PREFIX: &str = "private:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Persistent(String),
    Ephemeral(String),
}

pub fn store_kind(profile: &str) -> StoreKind {
    let dashes = [8, 13, 18, 23];
    let shaped = profile.len() == 36 && profile.char_indices().all(|(i, c)| if dashes.contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    let uuid = shaped && profile.chars().any(|c| c != '0' && c != '-');
    if uuid { StoreKind::Persistent(profile.to_ascii_lowercase()) } else { StoreKind::Ephemeral(profile.to_owned()) }
}

pub const BURNER_PREFIX: &str = "burner:";

pub fn burner_id(id: &str) -> String {
    id.strip_prefix(PRIVATE_PREFIX).map_or_else(|| id.to_owned(), |session| format!("{BURNER_PREFIX}{session}"))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Data {
    id: String,
}

impl Data {
    pub fn store_id(&self) -> String {
        self.id.clone()
    }

    pub fn is_ephemeral(&self) -> bool {
        matches!(store_kind(&self.id), StoreKind::Ephemeral(_))
    }
}

impl Drop for Data {
    fn drop(&mut self) {
        #[cfg(target_os = "ios")]
        if self.is_ephemeral() {
            crate::stores::release(&self.id);
            crate::stores::release(&burner_id(&self.id));
        }
    }
}

impl WebsiteData for Data {
    type RuleLists = ();

    fn persistent(uuid: &str, _: &str, _: Rc<dyn Fn(Event)>) -> Option<Self> {
        match store_kind(uuid) {
            StoreKind::Persistent(id) => Some(Data { id }),
            StoreKind::Ephemeral(_) => None,
        }
    }

    fn ephemeral(_: Rc<dyn Fn(Event)>) -> Option<Self> {
        Some(Data { id: format!("{PRIVATE_PREFIX}{}", uuid::Uuid::new_v4().simple()) })
    }

    fn wipe_persistent(uuid: &str) {
        #[cfg(target_os = "ios")]
        if let StoreKind::Persistent(uuid) = store_kind(uuid) {
            crate::stores::wipe(uuid);
        }
        #[cfg(not(target_os = "ios"))]
        let _ = uuid;
    }

    fn import_cookies(&self, cookies: Vec<Cookie>, _: i64, done: Box<dyn FnOnce(usize, usize)>) {
        done(0, cookies.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloads() -> Rc<dyn Fn(Event)> {
        Rc::new(|_| {})
    }

    #[test]
    fn every_ephemeral_store_has_a_new_private_id() {
        let a = Data::ephemeral(downloads()).unwrap();
        let b = Data::ephemeral(downloads()).unwrap();
        assert_ne!(a, b);
        assert!(a.is_ephemeral());
        let session = a.store_id().strip_prefix(PRIVATE_PREFIX).unwrap().to_owned();
        assert!(session.len() == 32 && session.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "{session}");
    }

    #[test]
    fn a_burner_is_an_ephemeral_store_of_its_own() {
        let data = Data::ephemeral(downloads()).unwrap();
        let burner = burner_id(&data.store_id());
        assert!(burner.starts_with(BURNER_PREFIX));
        assert_eq!(store_kind(&burner), StoreKind::Ephemeral(burner.clone()));
        assert_ne!(store_kind(&burner), store_kind(&data.store_id()), "never the private space's store");
        assert_ne!(burner_id(&Data::ephemeral(downloads()).unwrap().store_id()), burner);
    }

    #[test]
    fn only_a_uuid_opens_a_persistent_store() {
        let uuid = "2FDA0ABA-1C3E-4E0B-9C51-4F7F1B2D3E4A";
        assert_eq!(store_kind(uuid), StoreKind::Persistent(uuid.to_ascii_lowercase()));
        assert_eq!(Data::persistent(uuid, uuid, downloads()).unwrap().store_id(), uuid.to_ascii_lowercase());
        for ephemeral in [
            "private:6f1c2d",
            "lockdown:3:abc",
            "",
            "00000000-0000-0000-0000-000000000000",
            "2fda0aba1c3e4e0b9c514f7f1b2d3e4a0000",
            "../../2fda0aba-1c3e-4e0b-9c51",
            "2fda0aba-1c3e4-e0b-9c51-4f7f1b2d3e4a",
        ] {
            assert_eq!(store_kind(ephemeral), StoreKind::Ephemeral(ephemeral.to_owned()), "{ephemeral}");
            assert!(Data::persistent(ephemeral, ephemeral, downloads()).is_none(), "{ephemeral}");
        }
    }
}
