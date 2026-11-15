use std::cell::RefCell;
use std::collections::HashMap;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::{NSDate, NSOperationQueue, NSString, NSUUID};
use objc2_web_kit::{WKContentRuleList, WKWebsiteDataStore};
use shield::ShieldLevel;

use crate::data::{StoreKind, store_kind};

thread_local! {
    static SESSIONS: RefCell<HashMap<String, Retained<WKWebsiteDataStore>>> = RefCell::new(HashMap::new());
    static RULES: RefCell<HashMap<ShieldLevel, Retained<WKContentRuleList>>> = RefCell::new(HashMap::new());
}

pub fn store_for(profile: &str, mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
    match store_kind(profile) {
        StoreKind::Persistent(uuid) => match NSUUID::from_string(&NSString::from_str(&uuid)) {
            // SAFETY: main thread (marker); `id` is a valid, non-zero UUID (`store_kind`).
            Some(id) => unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&id, mtm) },
            None => ephemeral(profile, mtm),
        },
        StoreKind::Ephemeral(session) => ephemeral(&session, mtm),
    }
}

pub fn live_store(profile: &str, mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
    match store_kind(profile) {
        StoreKind::Ephemeral(session) => SESSIONS
            .with(|s| s.borrow().get(&session).cloned())
            // SAFETY: main thread (marker); the store is never registered, so it goes with its last user.
            .unwrap_or_else(|| unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) }),
        StoreKind::Persistent(_) => store_for(profile, mtm),
    }
}

fn ephemeral(session: &str, mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
    SESSIONS.with(|s| {
        let mut sessions = s.borrow_mut();
        if let Some(store) = sessions.get(session) {
            return store.clone();
        }
        // SAFETY: main thread (marker).
        let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
        sessions.insert(session.to_owned(), store.clone());
        store
    })
}

pub fn release(session: &str) {
    let session = session.to_owned();
    let block = RcBlock::new(move || {
        let _ = SESSIONS.try_with(|s| s.borrow_mut().remove(&session));
    });
    // SAFETY: the stores live in the main thread's map, so the block runs on the main queue; it captures only an owned string.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
}

pub fn rule_list(level: ShieldLevel) -> Option<Retained<WKContentRuleList>> {
    RULES.with(|r| r.borrow().get(&level).cloned())
}

pub fn keep_rule_list(level: ShieldLevel, list: Retained<WKContentRuleList>) {
    RULES.with(|r| r.borrow_mut().insert(level, list));
}

pub fn wipe(uuid: String) {
    let block = RcBlock::new(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let store = store_for(&uuid, mtm);
        let done = RcBlock::new(|| {});
        // SAFETY: main thread (marker); WebKit copies the block and the type set.
        unsafe { store.removeDataOfTypes_modifiedSince_completionHandler(&WKWebsiteDataStore::allWebsiteDataTypes(mtm), &NSDate::distantPast(), &done) };
    });
    // SAFETY: the main queue runs the block on the main thread; it captures only an owned string.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
}
