use std::path::PathBuf;
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Space {
    Normal,
    Private { session: String },
}

#[derive(Debug)]
pub struct Host {
    pub space: Space,
    pub data_dir: PathBuf,
}

struct Bridge {
    vm: JavaVM,
    class: GlobalRef,
}

static HOST: OnceLock<Host> = OnceLock::new();
static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn host() -> Option<&'static Host> {
    HOST.get()
}

pub fn space() -> Space {
    host().map_or(Space::Normal, |h| h.space.clone())
}

pub fn valid_session(session: &str) -> bool {
    session.len() == 32 && session.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn start(env: &mut JNIEnv, class: &JClass, data_dir: String, session: Option<String>) -> bool {
    let space = match session {
        Some(s) if valid_session(&s) => Space::Private { session: s },
        Some(_) => return false,
        None => Space::Normal,
    };
    let (Ok(vm), Ok(class)) = (env.get_java_vm(), env.new_global_ref(class)) else {
        return false;
    };
    let host = Host { space, data_dir: PathBuf::from(data_dir) };
    let same = HOST.get().is_none_or(|h| h.space == host.space && h.data_dir == host.data_dir);
    HOST.get_or_init(|| host);
    BRIDGE.get_or_init(|| Bridge { vm, class });
    same
}

fn call(method: &str, signature: &str, args: impl for<'local> FnOnce(&mut JNIEnv<'local>) -> Option<Vec<JObject<'local>>>) -> Option<i32> {
    let bridge = BRIDGE.get()?;
    let mut env = bridge.vm.attach_current_thread_permanently().ok()?;
    let result = env
        .with_local_frame(16, |env| -> jni::errors::Result<Option<i32>> {
            let Some(objects) = args(env) else {
                return Ok(None);
            };
            let values: Vec<JValue> = objects.iter().map(JValue::Object).collect();
            let class: &JClass = bridge.class.as_obj().into();
            let value = env.call_static_method(class, method, signature, &values)?;
            Ok(Some(match value {
                jni::objects::JValueGen::Int(i) => i,
                jni::objects::JValueGen::Bool(b) => i32::from(b),
                _ => 0,
            }))
        })
        .ok()
        .flatten();
    if env.exception_check().unwrap_or(false) {
        env.exception_clear().ok();
        return None;
    }
    result
}

fn string<'local>(env: &mut JNIEnv<'local>, value: &str) -> Option<JObject<'local>> {
    env.new_string(value).ok().map(JObject::from)
}

fn strings<'local>(env: &mut JNIEnv<'local>, values: &[String]) -> Option<JObject<'local>> {
    let array = env.new_object_array(i32::try_from(values.len()).ok()?, "java/lang/String", JObject::null()).ok()?;
    for (i, value) in values.iter().enumerate() {
        let item = env.new_string(value).ok()?;
        env.set_object_array_element(&array, i32::try_from(i).ok()?, &item).ok()?;
        env.delete_local_ref(item).ok()?;
    }
    Some(JObject::from(array))
}

pub fn wipe_store(store: &str) -> bool {
    call("wipeStore", "(Ljava/lang/String;)V", |env| Some(vec![string(env, store)?])).is_some()
}

pub fn import_cookies(store: &str, urls: &[String], cookies: &[String]) -> usize {
    call("importCookies", "(Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)I", |env| {
        Some(vec![string(env, store)?, strings(env, urls)?, strings(env, cookies)?])
    })
    .and_then(|n| usize::try_from(n).ok())
    .unwrap_or(0)
}

pub(crate) fn call_string(method: &str, value: &str) -> bool {
    call(method, "(Ljava/lang/String;)V", |env| Some(vec![string(env, value)?])).is_some()
}

pub(crate) fn call_int(method: &str) -> Option<i32> {
    call(method, "()I", |_| Some(Vec::new()))
}

pub(crate) fn call_bool(method: &str, value: &str) -> bool {
    call(method, "(Ljava/lang/String;)Z", |env| Some(vec![string(env, value)?])) == Some(1)
}

pub type Fetched = Result<(u16, Option<String>, Vec<u8>), String>;

fn read_reply(env: &mut JNIEnv, reply: &JObject) -> Option<Fetched> {
    let array: &JObjectArray = reply.into();
    let text = |env: &mut JNIEnv, i: i32| -> Option<Option<String>> {
        let item = env.get_object_array_element(array, i).ok()?;
        if item.is_null() {
            return Some(None);
        }
        Some(Some(env.get_string(&JString::from(item)).ok()?.into()))
    };
    if env.get_array_length(array).ok()? == 1 {
        return Some(Err(text(env, 0)?.unwrap_or_default()));
    }
    let status = text(env, 0)??.parse().ok()?;
    let location = text(env, 1)?;
    let body = env.get_object_array_element(array, 2).ok()?;
    let body = env.convert_byte_array(JByteArray::from(body)).ok()?;
    Some(Ok((status, location, body)))
}

pub fn fetch(url: &str, max: u64, agent: &str, stall_ms: u64, total_ms: u64) -> Fetched {
    let unavailable = || "The network is not available.".to_owned();
    let bridge = BRIDGE.get().ok_or_else(unavailable)?;
    let mut env = bridge.vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let result = env
        .with_local_frame(16, |env| -> jni::errors::Result<Option<Fetched>> {
            let (url, agent) = (env.new_string(url)?, env.new_string(agent)?);
            let ms = |v: u64| JValue::Int(i32::try_from(v).unwrap_or(i32::MAX));
            let args = [JValue::Object(&url), JValue::Long(i64::try_from(max).unwrap_or(i64::MAX)), JValue::Object(&agent), ms(stall_ms), ms(total_ms)];
            let class: &JClass = bridge.class.as_obj().into();
            let reply = env.call_static_method(class, "fetch", "(Ljava/lang/String;JLjava/lang/String;II)[Ljava/lang/Object;", &args)?.l()?;
            Ok(read_reply(env, &reply))
        })
        .ok()
        .flatten();
    if env.exception_check().unwrap_or(false) {
        env.exception_clear().ok();
    }
    result.unwrap_or_else(|| Err(unavailable()))
}

pub fn release_store(store: &str) -> bool {
    call_string("releaseStore", store)
}

pub fn open_private_space() -> bool {
    call("openPrivateSpace", "()V", |_| Some(Vec::new())).is_some()
}

pub fn close_private_space() -> bool {
    call("closePrivateSpace", "()V", |_| Some(Vec::new())).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_32_lowercase_hex_digits() {
        assert!(valid_session("0123456789abcdef0123456789abcdef"));
        assert!(!valid_session("0123456789ABCDEF0123456789abcdef"));
        assert!(!valid_session("../../0123456789abcdef0123456789"));
        assert!(!valid_session("0123456789abcdef"));
        assert!(!valid_session(""));
    }

    #[test]
    fn without_a_java_host_calls_report_failure() {
        assert!(host().is_none());
        assert_eq!(space(), Space::Normal);
        assert!(!wipe_store("tarn-x"));
        assert!(!open_private_space());
        assert!(fetch("https://api.github.com/", 1, "a", 1, 1).is_err());
    }
}
