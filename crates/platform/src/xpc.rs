//! The few libxpc calls the CLI helper needs: a service listener and a client with one
//! request/reply round trip of string-keyed dictionaries.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use block2::RcBlock;

type Object = *mut c_void;

unsafe extern "C" {
    static _xpc_type_dictionary: c_void;
    fn xpc_connection_create(name: *const c_char, queue: *mut c_void) -> Object;
    fn xpc_connection_set_event_handler(connection: Object, handler: &block2::DynBlock<dyn Fn(Object)>);
    fn xpc_connection_resume(connection: Object);
    fn xpc_connection_cancel(connection: Object);
    fn xpc_connection_send_message(connection: Object, message: Object);
    fn xpc_connection_send_message_with_reply_sync(connection: Object, message: Object) -> Object;
    fn xpc_connection_set_peer_code_signing_requirement(connection: Object, requirement: *const c_char) -> c_int;
    fn xpc_dictionary_create(keys: *const *const c_char, values: *const Object, count: usize) -> Object;
    fn xpc_dictionary_create_reply(original: Object) -> Object;
    fn xpc_dictionary_set_string(dictionary: Object, key: *const c_char, value: *const c_char);
    fn xpc_dictionary_get_string(dictionary: Object, key: *const c_char) -> *const c_char;
    fn xpc_dictionary_get_remote_connection(dictionary: Object) -> Object;
    fn xpc_get_type(object: Object) -> *const c_void;
    fn xpc_release(object: Object);
    fn xpc_retain(object: Object) -> Object;
    fn xpc_main(handler: extern "C" fn(Object)) -> !;
    fn sandbox_check(pid: c_int, operation: *const c_char, filter: c_int, ...) -> c_int;
}

/// Whether the calling process runs inside an App Sandbox.
pub fn sandboxed() -> bool {
    // SAFETY: `sandbox_check` with a null operation and no filter only queries this process.
    unsafe { sandbox_check(std::process::id() as c_int, std::ptr::null(), 0) != 0 }
}

/// A dictionary message; values are strings.
pub struct Message(Object);

impl Message {
    pub fn new() -> Self {
        // SAFETY: an empty dictionary from null key and value arrays of length 0.
        Message(unsafe { xpc_dictionary_create(std::ptr::null(), std::ptr::null(), 0) })
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), std::ffi::NulError> {
        let (key, value) = (CString::new(key)?, CString::new(value)?);
        // SAFETY: `self.0` is a live dictionary; libxpc copies both strings.
        unsafe { xpc_dictionary_set_string(self.0, key.as_ptr(), value.as_ptr()) };
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let key = CString::new(key).ok()?;
        // SAFETY: `self.0` is a live dictionary; the returned pointer is owned by it and copied here.
        let value = unsafe { xpc_dictionary_get_string(self.0, key.as_ptr()) };
        if value.is_null() {
            return None;
        }
        // SAFETY: non-null values are NUL-terminated strings inside the dictionary.
        unsafe { CStr::from_ptr(value) }.to_str().ok().map(str::to_owned)
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        // SAFETY: each `Message` owns one reference.
        unsafe { xpc_release(self.0) };
    }
}

/// libxpc refuses to release a connection that was never resumed, so even one about to be
/// cancelled gets a handler and is resumed.
fn activate(connection: Object) {
    let handler = RcBlock::new(|_event: Object| {});
    // SAFETY: live connection; libxpc copies the block.
    unsafe {
        xpc_connection_set_event_handler(connection, &handler);
        xpc_connection_resume(connection);
    }
}

fn is_dictionary(object: Object) -> bool {
    // SAFETY: reading the type of a live object and the address of a libxpc type constant.
    unsafe { std::ptr::eq(xpc_get_type(object), &_xpc_type_dictionary) }
}

/// Developer ID intermediate and leaf markers; debug builds are signed with Apple Development
/// certificates, which release builds must refuse.
const DEVELOPER_ID: &str =
    if cfg!(debug_assertions) { "" } else { " and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13]" };

/// The code signing requirement for `identifier` signed by Apple-issued certificates of team
/// `team`, Developer ID ones in release builds; `None` for anything but a ten-character team ID
/// and a bundle identifier, so no caller can widen it.
fn requirement(team: &str, identifier: &str) -> Option<CString> {
    let team_ok = team.len() == 10 && team.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    let id_ok = !identifier.is_empty() && identifier.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    (team_ok && id_ok)
        .then(|| format!(r#"anchor apple generic and certificate leaf[subject.OU] = "{team}" and identifier "{identifier}"{DEVELOPER_ID}"#))
        .and_then(|r| CString::new(r).ok())
}

/// A connection to a bundled XPC service of the calling app.
pub struct Client(Object);

impl Client {
    /// The service must be signed by `team` with the bundle identifier `identifier`.
    pub fn connect(service: &str, team: &str, identifier: &str) -> Option<Self> {
        let (name, requirement) = (CString::new(service).ok()?, requirement(team, identifier)?);
        // SAFETY: a null queue selects libxpc's default target queue.
        let connection = unsafe { xpc_connection_create(name.as_ptr(), std::ptr::null_mut()) };
        if connection.is_null() {
            return None;
        }
        // SAFETY: live, not yet resumed connection; libxpc copies the string.
        let pinned = unsafe { xpc_connection_set_peer_code_signing_requirement(connection, requirement.as_ptr()) } == 0;
        activate(connection);
        let client = Client(connection);
        pinned.then_some(client)
    }

    /// Sends `message` and waits for the reply; `None` on a connection error.
    pub fn request(&self, message: &Message) -> Option<Message> {
        // SAFETY: live connection and message; the reply is returned retained.
        let reply = unsafe { xpc_connection_send_message_with_reply_sync(self.0, message.0) };
        if reply.is_null() {
            return None;
        }
        if !is_dictionary(reply) {
            // SAFETY: we own the returned reference.
            unsafe { xpc_release(reply) };
            return None;
        }
        Some(Message(reply))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: cancel then release the reference `xpc_connection_create` returned.
        unsafe {
            xpc_connection_cancel(self.0);
            xpc_release(self.0);
        }
    }
}

type Handler = fn(&Message, &Message);
static HANDLER: std::sync::OnceLock<Handler> = std::sync::OnceLock::new();
static PEER_REQUIREMENT: std::sync::OnceLock<CString> = std::sync::OnceLock::new();

extern "C" fn on_connection(peer: Object) {
    let handler = RcBlock::new(|event: Object| {
        if !is_dictionary(event) {
            return;
        }
        let Some(handle) = HANDLER.get() else { return };
        // SAFETY: `event` is a live dictionary from a peer, so it can be replied to; we take our
        // own reference to it so `Message`'s drop balances, and the reply is ours.
        unsafe {
            let reply = xpc_dictionary_create_reply(event);
            if reply.is_null() {
                return;
            }
            let (request, reply) = (Message(xpc_retain(event)), Message(reply));
            handle(&request, &reply);
            let remote = xpc_dictionary_get_remote_connection(event);
            if !remote.is_null() {
                xpc_connection_send_message(remote, reply.0);
            }
        }
    });
    // SAFETY: `peer` is the new connection libxpc hands to the listener; it must be resumed.
    unsafe {
        let requirement = PEER_REQUIREMENT.get().expect("serve sets the requirement before xpc_main");
        if xpc_connection_set_peer_code_signing_requirement(peer, requirement.as_ptr()) != 0 {
            activate(peer);
            xpc_connection_cancel(peer);
            return;
        }
        xpc_connection_set_event_handler(peer, &handler);
        xpc_connection_resume(peer);
    }
}

/// Runs this process as an XPC service; `handler` fills the reply to each request from a caller
/// signed by `team` with the bundle identifier `identifier`. Never returns.
pub fn serve(team: &str, identifier: &str, handler: Handler) -> ! {
    let _ = HANDLER.set(handler);
    // A service that cannot check its callers must not run at all.
    let requirement = requirement(team, identifier).expect("a team ID and a bundle identifier");
    let _ = PEER_REQUIREMENT.set(requirement);
    // SAFETY: called once from the service's main; libxpc owns the process from here.
    unsafe { xpc_main(on_connection) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requirement_always_pins_the_team_and_cannot_be_widened() {
        let r = super::requirement("ABCDE12345", "dev.tarn.helper").unwrap();
        let pinned = r#"anchor apple generic and certificate leaf[subject.OU] = "ABCDE12345" and identifier "dev.tarn.helper""#;
        let developer_id = " and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13]";
        let expected = if cfg!(debug_assertions) { pinned.to_owned() } else { format!("{pinned}{developer_id}") };
        assert_eq!(r.to_str().unwrap(), expected);
        for (team, id) in [
            ("ABCDE1234", "a.b"),
            ("abcde12345", "a.b"),
            ("ABCDE12345", ""),
            ("ABCDE12345", r#"a" or anchor apple"#),
            (r#"ABCDE12345" or ""#, "a.b"),
            ("ABCDE12345", "a b"),
        ] {
            assert!(super::requirement(team, id).is_none(), "{team} {id}");
        }
    }

    #[test]
    fn test_process_is_not_sandboxed_and_messages_roundtrip() {
        assert!(!sandboxed());
        let m = Message::new();
        m.set("k", "v").unwrap();
        assert_eq!(m.get("k").as_deref(), Some("v"));
        assert!(m.set("k", "a\0b").is_err());
        assert_eq!(m.get("missing"), None);
    }
}
