//! Bitwarden's biometric unlock as its Safari app extension does it: Touch ID, then the user key
//! that the Bitwarden desktop app keeps in the login keychain once its own Touch ID unlock is on.

use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use dispatch2::{DispatchQoS, DispatchQueue, GlobalQueueIdentifier};
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::get_generic_password;

const SERVICE: &str = "Bitwarden_biometric";

/// One Touch ID prompt at a time, so a request loop cannot stack prompts for the user to approve.
static PROMPTING: AtomicBool = AtomicBool::new(false);

/// Bitwarden's `BiometricsStatus` values.
pub(crate) const AVAILABLE: i32 = 0;
pub(crate) const HARDWARE_UNAVAILABLE: i32 = 2;
pub(crate) const NOT_ENABLED_IN_DESKTOP_APP: i32 = 8;

fn account(user: &str) -> Option<String> {
    let valid = !user.is_empty() && user.len() <= 64 && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then(|| format!("{user}_user_biometric"))
}

fn hardware() -> bool {
    // SAFETY: a fresh context queried on the calling thread.
    unsafe { LAContext::new().canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }.is_ok()
}

fn has_key(user: &str) -> bool {
    let Some(account) = account(user) else { return false };
    let mut search = ItemSearchOptions::new();
    search.class(ItemClass::generic_password()).service(SERVICE).account(&account).load_attributes(true).limit(Limit::Max(1));
    search.search().is_ok_and(|found| !found.is_empty())
}

/// Looks off the main thread, since a keychain search can wait on the keychain; `done` runs on the main queue.
pub(crate) fn status(user: Option<String>, done: impl FnOnce(i32) + Send + 'static) {
    DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(DispatchQoS::UserInitiated)).exec_async(move || {
        let status = match user {
            _ if !hardware() => HARDWARE_UNAVAILABLE,
            Some(user) if !has_key(&user) => NOT_ENABLED_IN_DESKTOP_APP,
            _ => AVAILABLE,
        };
        DispatchQueue::main().exec_async(move || done(status));
    });
}

/// Asks for Touch ID on behalf of `app`, then reads the user key; `done` runs on the main queue with it, base64.
pub(crate) fn unlock(user: &str, app: &str, done: impl FnOnce(Option<String>) + Send + 'static) {
    let Some(account) = account(user) else {
        DispatchQueue::main().exec_async(move || done(None));
        return;
    };
    evaluate(app, move |ok| if ok { read_key(&account) } else { None }, done);
}

fn read_key(account: &str) -> Option<String> {
    let bytes = get_generic_password(SERVICE, account).ok()?;
    let key = String::from_utf8(bytes).ok()?.replace('"', "");
    (!key.is_empty()).then_some(key)
}

/// Clears `PROMPTING` once the keychain read after Touch ID is over, on every way out.
struct Prompting;

impl Drop for Prompting {
    fn drop(&mut self) {
        PROMPTING.store(false, Ordering::SeqCst);
    }
}

/// `work` runs off the main thread once Touch ID answered, so a keychain prompt cannot block the UI.
pub(crate) fn evaluate<T: Send + 'static>(app: &str, work: impl FnOnce(bool) -> T + Send + 'static, done: impl FnOnce(T) + Send + 'static) {
    ask(LAPolicy::DeviceOwnerAuthenticationWithBiometrics, &format!("unlock {app}"), work, done);
}

/// Asks the Mac's owner to confirm with Touch ID or their login password; `done` runs on the
/// main thread with the answer. `reason` completes "Tarn is trying to …".
pub fn confirm_owner(reason: &str, done: impl FnOnce(bool) + Send + 'static) {
    ask(LAPolicy::DeviceOwnerAuthentication, reason, |ok| ok, done);
}

fn ask<T: Send + 'static>(policy: LAPolicy, reason: &str, work: impl FnOnce(bool) -> T + Send + 'static, done: impl FnOnce(T) + Send + 'static) {
    if PROMPTING.swap(true, Ordering::SeqCst) {
        let result = work(false);
        DispatchQueue::main().exec_async(move || done(result));
        return;
    }
    let pending = std::sync::Mutex::new(Some((work, done)));
    // SAFETY: a fresh context; LocalAuthentication calls the block once, on a queue of its own.
    let context = unsafe { LAContext::new() };
    let evaluating = context.clone();
    let reply = RcBlock::new(move |ok: Bool, _: *mut NSError| {
        // A released context cancels its evaluation, so the block keeps it until the answer.
        let _ = &evaluating;
        let _prompting = Prompting;
        let Some((work, done)) = pending.lock().ok().and_then(|mut p| p.take()) else { return };
        let result = work(ok.as_bool());
        DispatchQueue::main().exec_async(move || done(result));
    });
    // SAFETY: LocalAuthentication copies the block, which holds the context until it replies.
    unsafe { context.evaluatePolicy_localizedReason_reply(policy, &NSString::from_str(reason), &reply) };
}

#[cfg(test)]
mod tests {
    use super::account;

    #[test]
    fn keychain_accounts_come_only_from_plain_user_ids() {
        assert_eq!(account("811f043e-1c16-404c-b5ce-f205d0a6291f").as_deref(), Some("811f043e-1c16-404c-b5ce-f205d0a6291f_user_biometric"));
        assert_eq!(account(""), None);
        assert_eq!(account("a\"b"), None);
        assert_eq!(account(&"a".repeat(65)), None);
    }
}
