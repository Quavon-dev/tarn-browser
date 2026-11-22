//! Android pieces of the Tarn mobile app: the JNI calls `tarn.webview` makes for navigation and
//! Shield decisions, the bridge back to the Java host, and `Data`, the `WebsiteData` for Android.

mod data;
mod host;
mod policy;
mod secrets;
pub mod system;
pub mod tarn_ext;
mod transport;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jdouble, jdoubleArray, jint, jlong, jobjectArray, jstring};

pub use data::{Data, PRIVATE_PREFIX};
pub use host::{Host, Space, close_private_space, host, open_private_space, space};
pub use secrets::Keystore;
pub use transport::HostTransport;

/// Filter lists for every view in this process; `None` switches them off.
pub fn install_filters(matcher: Option<shield::matcher::Matcher>) {
    policy::install_filters(matcher);
}

fn string(env: &mut JNIEnv, value: &JString) -> Option<String> {
    if value.is_null() {
        return None;
    }
    env.get_string(value).ok().map(Into::into)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_start(mut env: JNIEnv, _: JClass, host_class: JClass, data_dir: JString, session: JString) -> jboolean {
    let session = string(&mut env, &session);
    let Some(data_dir) = string(&mut env, &data_dir) else {
        return 0;
    };
    host::start(&mut env, &host_class, data_dir, session).into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_warm(_: JNIEnv, _: JClass) -> jint {
    jint::try_from(policy::warm()).unwrap_or(jint::MAX)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_strongKey(_: JNIEnv, _: JClass, kind: jint, bits: jint) -> jboolean {
    let kind = browser_core::tls::KeyKind::from_code(kind);
    u8::from(u32::try_from(bits).is_ok_and(|bits| browser_core::tls::strong_key(kind, bits)))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_decide(mut env: JNIEnv, _: JClass, url: JString, source: jint) -> jint {
    string(&mut env, &url).map_or(policy::BLOCK, |url| policy::decide(&url, source))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_levelAction(
    mut env: JNIEnv,
    _: JClass,
    url: JString,
    site: jint,
    view: jint,
    from_shell: jboolean,
) -> jint {
    string(&mut env, &url).map_or(policy::BLOCK_INSECURE, |url| policy::level_action(&url, site, view, from_shell != 0))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_intoPrivateNetwork(
    mut env: JNIEnv,
    _: JClass,
    target: JString,
    from: JString,
    user_started: jboolean,
    level: jint,
) -> jboolean {
    let from = string(&mut env, &from);
    string(&mut env, &target).is_none_or(|target| policy::into_private_network(&target, from.as_deref(), user_started != 0, level)).into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_verdict(mut env: JNIEnv, _: JClass, url: JString, top: JString, kind: jint, level: jint) -> jint {
    let top = string(&mut env, &top);
    string(&mut env, &url).map_or(policy::VERDICT_BLOCK, |url| policy::verdict(&url, top.as_deref(), kind, level))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_mayAskCredentials(
    mut env: JNIEnv,
    _: JClass,
    challenge: JString,
    page: JString,
    failures: jint,
) -> jboolean {
    let page = string(&mut env, &page);
    string(&mut env, &challenge).is_some_and(|challenge| policy::may_ask_credentials(&challenge, page.as_deref(), failures)).into()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_popupBurst(_: JNIEnv, _: JClass) -> jint {
    jint::try_from(engine::view_policy::POPUP_BURST).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_downloadBurst(_: JNIEnv, _: JClass) -> jint {
    jint::try_from(engine::view_policy::DOWNLOAD_BURST).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_burstWindowMs(_: JNIEnv, _: JClass) -> jlong {
    jlong::try_from(engine::view_policy::BURST_WINDOW.as_millis()).unwrap_or(jlong::MAX)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_downloadName(mut env: JNIEnv, _: JClass, suggested: JString) -> jstring {
    let name = engine::downloads::sanitize_filename(&string(&mut env, &suggested).unwrap_or_default());
    env.new_string(name).map_or(std::ptr::null_mut(), |s| s.into_raw())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_storeInstall(mut env: JNIEnv, _: JClass, url: JString) -> jstring {
    let id = string(&mut env, &url).and_then(|u| url::Url::parse(&u).ok()).and_then(|u| engine::tarn_ext::store_install(&u));
    java_string(&mut env, id)
}

fn java_string(env: &mut JNIEnv, value: Option<String>) -> jstring {
    value.and_then(|v| env.new_string(v).ok()).map_or(std::ptr::null_mut(), |s| s.into_raw())
}

fn string_array(env: &mut JNIEnv, values: &JObjectArray) -> Vec<String> {
    let len = env.get_array_length(values).unwrap_or(0);
    (0..len)
        .filter_map(|i| {
            let item = env.get_object_array_element(values, i).ok()?;
            string(env, &JString::from(item))
        })
        .collect()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_readerScript(mut env: JNIEnv, _: JClass, page: JString, probe: jboolean) -> jstring {
    let script = string(&mut env, &page).and_then(|p| url::Url::parse(&p).ok()).map(|u| engine::reader::script(&u.origin().ascii_serialization(), probe != 0));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_pageTextScript(mut env: JNIEnv, _: JClass, page: JString, max_chars: jint) -> jstring {
    let max = usize::try_from(max_chars).unwrap_or(0).min(MAX_PAGE_TEXT);
    let script = string(&mut env, &page).and_then(|p| url::Url::parse(&p).ok()).and_then(|u| engine::page_scripts::page_text_script(&u, max));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_iconLinksScript(mut env: JNIEnv, _: JClass, page: JString) -> jstring {
    let script = string(&mut env, &page).and_then(|p| url::Url::parse(&p).ok()).and_then(|u| engine::page_scripts::icon_links_script(&u));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_fetchIconScript(mut env: JNIEnv, _: JClass, page: JString, urls: JObjectArray) -> jstring {
    let urls = engine::favicon::fetch_urls(string_array(&mut env, &urls).iter().map(String::as_str));
    let script = string(&mut env, &page)
        .and_then(|p| url::Url::parse(&p).ok())
        .and_then(|u| engine::page_scripts::fetch_icon_script(&u, &urls, engine::favicon::MAX_BYTES));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_picker(mut env: JNIEnv, _: JClass) -> jstring {
    java_string(&mut env, Some(engine::page_scripts::PICKER.to_owned()))
}

/// `insert_text_script` of `text`; null for a text longer than a snippet.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_insertTextScript(mut env: JNIEnv, _: JClass, text: JString) -> jstring {
    let script = string(&mut env, &text).filter(|t| t.len() <= browser_core::snippets::MAX_SNIPPET).map(|t| engine::page_scripts::insert_text_script(&t));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_inserted(mut env: JNIEnv, _: JClass, answer: JString) -> jboolean {
    jboolean::from(browser_core::snippets::inserted(&string(&mut env, &answer).unwrap_or_default()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_pickRect(mut env: JNIEnv, _: JClass) -> jstring {
    java_string(&mut env, Some(engine::page_scripts::pick_rect_script()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_selectedTextScript(mut env: JNIEnv, _: JClass) -> jstring {
    java_string(&mut env, Some(engine::page_scripts::SELECTED_TEXT.to_owned()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_mediaStateScript(mut env: JNIEnv, _: JClass) -> jstring {
    java_string(&mut env, Some(engine::page_scripts::media_state_script()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_mediaCommandScript(mut env: JNIEnv, _: JClass, action: JString, seconds: jdouble) -> jstring {
    let args = serde_json::json!({ "action": string(&mut env, &action), "seconds": seconds });
    java_string(&mut env, browser_core::page_ops::media_command(&args).map(engine::page_scripts::media_command_script))
}

/// `[x, y, width, height]` of an accepted snapshot rectangle, empty for the full page, null when refused.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_snapshotArea(
    env: JNIEnv,
    _: JClass,
    full_page: jboolean,
    x: jdouble,
    y: jdouble,
    width: jdouble,
    height: jdouble,
) -> jdoubleArray {
    use engine::view_policy::SnapshotArea;
    let args = serde_json::json!({ "fullPage": full_page != 0, "x": x, "y": y, "width": width, "height": height });
    let values = match browser_core::page_ops::snapshot_area(&args) {
        None => return std::ptr::null_mut(),
        Some(SnapshotArea::FullPage) => Vec::new(),
        Some(SnapshotArea::Rect { x, y, width, height }) => vec![x, y, width, height],
    };
    let Ok(array) = env.new_double_array(values.len() as i32) else { return std::ptr::null_mut() };
    if env.set_double_array_region(&array, 0, &values).is_err() {
        return std::ptr::null_mut();
    }
    array.into_raw()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_siteStylesScript(
    mut env: JNIEnv,
    _: JClass,
    host: JString,
    css: JString,
    zaps: JObjectArray,
    theme: JString,
) -> jstring {
    let theme = theme_sheets(&mut env, &theme);
    let script = string(&mut env, &host).and_then(|host| {
        let css = string(&mut env, &css).unwrap_or_default();
        let zaps = string_array(&mut env, &zaps);
        site_sheet(&host, &css, &zaps, &theme)
    });
    java_string(&mut env, script)
}

fn theme_sheets(env: &mut JNIEnv, json: &JString) -> Vec<(String, String)> {
    string(env, json).and_then(|j| serde_json::from_str(&j).ok()).map(|v| engine::page_scripts::theme_sheets(&v)).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_themeStylesScript(mut env: JNIEnv, _: JClass, theme: JString) -> jstring {
    let theme = theme_sheets(&mut env, &theme);
    java_string(&mut env, engine::page_scripts::site_styles_script(&theme))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_themeOrigins(mut env: JNIEnv, _: JClass, theme: JString) -> jobjectArray {
    let origins = theme_origins(&theme_sheets(&mut env, &theme));
    java_strings(&mut env, &origins)
}

fn java_strings(env: &mut JNIEnv, values: &[String]) -> jobjectArray {
    let Ok(array) = env.new_object_array(values.len() as i32, "java/lang/String", JObject::null()) else { return std::ptr::null_mut() };
    for (i, value) in values.iter().enumerate() {
        if let Ok(s) = env.new_string(value) {
            env.set_object_array_element(&array, i as i32, s).ok();
        }
    }
    array.into_raw()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_setWorldsSupported(_: JNIEnv, _: JClass, supported: jboolean) {
    tarn_ext::set_worlds_supported(supported != 0);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_extensionWorlds(mut env: JNIEnv, _: JClass, store: JString) -> jobjectArray {
    let worlds = string(&mut env, &store).map(|s| tarn_ext::world_names(&s)).unwrap_or_default();
    java_strings(&mut env, &worlds)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_extensionLoader(mut env: JNIEnv, _: JClass) -> jstring {
    java_string(&mut env, Some(tarn_ext::loader()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_extensionPageScript(mut env: JNIEnv, _: JClass, host: JString, bootstrap: JString) -> jstring {
    let host = string(&mut env, &host);
    let script = string(&mut env, &bootstrap).zip(host).map(|(b, h)| tarn_ext::page_script(&h, &b));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_extensionVerdict(
    mut env: JNIEnv,
    _: JClass,
    store: JString,
    url: JString,
    top: JString,
    level: jint,
    next: JString,
    next_level: jint,
    kind: jint,
) -> jint {
    let (top, next) = (string(&mut env, &top), string(&mut env, &next));
    let mut pages = vec![(top.as_deref(), level)];
    pages.extend(next.as_deref().map(|n| (Some(n), next_level)));
    match (string(&mut env, &store), string(&mut env, &url)) {
        (Some(store), Some(url)) => tarn_ext::verdict(&store, &url, kind, &pages),
        _ => policy::VERDICT_ALLOW,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_extensionResource(mut env: JNIEnv, _: JClass, store: JString, url: JString) -> jobjectArray {
    let resource = match (string(&mut env, &store), string(&mut env, &url)) {
        (Some(store), Some(url)) => tarn_ext::resource(&store, &url),
        _ => tarn_ext::resource("", ""),
    };
    resource_array(&mut env, resource).unwrap_or(std::ptr::null_mut())
}

fn resource_array(env: &mut JNIEnv, (resource, csp): (engine::tarn_ext::Resource, Option<String>)) -> Option<jobjectArray> {
    let array = env.new_object_array(4, "java/lang/Object", JObject::null()).ok()?;
    let texts = [Some(resource.status.to_string()), Some(resource.mime.to_owned()), csp];
    for (i, text) in texts.into_iter().enumerate() {
        if let Some(text) = text {
            let item = env.new_string(text).ok()?;
            env.set_object_array_element(&array, i as i32, item).ok()?;
        }
    }
    let body = env.byte_array_from_slice(&resource.body).ok()?;
    env.set_object_array_element(&array, 3, body).ok()?;
    Some(array.into_raw())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_downloadFolder(mut env: JNIEnv, _: JClass, name: JString) -> jstring {
    let folder = string(&mut env, &name).and_then(|n| engine::downloads::folder_for(&n)).map(str::to_owned);
    java_string(&mut env, folder)
}

const MAX_PAGE_TEXT: usize = 4096;
const MAX_FIND_TEXT: usize = 1_000_000;
const FIND_CONTEXT: usize = 60;
const FIND_QUERY_MAX: usize = 200;

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_findTextScript(mut env: JNIEnv, _: JClass, page: JString) -> jstring {
    let script = string(&mut env, &page).and_then(|p| url::Url::parse(&p).ok()).and_then(|u| engine::page_scripts::page_text_script(&u, MAX_FIND_TEXT));
    java_string(&mut env, script)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_tarn_android_Native_findSnippets<'local>(
    mut env: JNIEnv<'local>,
    _: JClass,
    text: JString,
    query: JString,
) -> JObjectArray<'local> {
    let text = string(&mut env, &text).unwrap_or_default();
    let query = string(&mut env, &query).unwrap_or_default();
    let (snippets, matched) = find_snippets(&text, &query);
    let lines: Vec<String> = std::iter::once(matched.to_string()).chain(snippets).collect();
    let array = env.new_object_array(lines.len() as i32, "java/lang/String", jni::objects::JObject::null());
    let Ok(array) = array else { return JObjectArray::from(jni::objects::JObject::null()) };
    for (i, line) in lines.iter().enumerate() {
        if let Ok(item) = env.new_string(line) {
            env.set_object_array_element(&array, i as i32, &item).ok();
        }
    }
    array
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

pub fn find_snippets(text: &str, query: &str) -> (Vec<String>, usize) {
    let text: Vec<char> = text.chars().take(MAX_FIND_TEXT).collect();
    let query: String = query.chars().take(FIND_QUERY_MAX).map(lower).collect();
    if query.is_empty() {
        return (Vec::new(), 0);
    }
    let mut folded = String::with_capacity(text.len());
    let mut starts = Vec::with_capacity(text.len());
    for c in &text {
        starts.push(folded.len());
        folded.push(lower(*c));
    }
    let width = query.chars().count();
    let mut snippets = Vec::new();
    let mut matched = 0;
    for (byte, _) in folded.match_indices(&query) {
        let Ok(at) = starts.binary_search(&byte) else { continue };
        matched += 1;
        if snippets.len() < engine::page_scripts::FIND_MATCHES {
            let start = at.saturating_sub(FIND_CONTEXT);
            let end = (at + width + FIND_CONTEXT).min(text.len());
            let snippet: String = text[start..end].iter().collect();
            snippets.push(snippet.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    (snippets, matched)
}

pub fn site_sheet(host: &str, css: &str, zaps: &[String], theme: &[(String, String)]) -> Option<String> {
    let zapped = shield::lists::zap_css(zaps.iter().map(String::as_str));
    let sheet = [Some(css.to_owned()).filter(|c| !c.is_empty()), zapped].into_iter().flatten().collect::<Vec<_>>().join("\n");
    if sheet.is_empty() {
        return None;
    }
    let sheets: Vec<(String, String)> = theme.iter().cloned().chain([(host.to_owned(), sheet)]).collect();
    engine::page_scripts::site_styles_script(&sheets)
}

pub fn theme_origins(theme: &[(String, String)]) -> Vec<String> {
    let mut origins = std::collections::BTreeSet::new();
    for pattern in theme.iter().flat_map(|(hosts, _)| hosts.split(',')) {
        if pattern == "*" {
            origins.insert("*".to_owned());
            continue;
        }
        let host = pattern.strip_prefix("*.").unwrap_or(pattern);
        for scheme in ["https", "http"] {
            origins.insert(format!("{scheme}://{host}"));
            if pattern.starts_with("*.") {
                origins.insert(format!("{scheme}://*.{host}"));
            }
        }
    }
    origins.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_snippets_match_case_insensitively_with_context_and_count_all() {
        let text = format!("{} Quokka here {} quokka there quokka", "a".repeat(100), "b".repeat(10));
        let (snippets, matched) = find_snippets(&text, "QUOKKA");
        assert_eq!(matched, 3);
        assert_eq!(snippets.len(), 3);
        assert!(snippets[0].starts_with('a') && snippets[0].contains("Quokka here"));
        assert!(snippets[0].chars().count() <= 2 * FIND_CONTEXT + 6);
        assert_eq!(find_snippets("abc", ""), (Vec::new(), 0));
        assert_eq!(find_snippets("İstanbul", "i̇").1, 0);
        assert_eq!(find_snippets("ÄÖ äö", "äö").1, 2);
        assert_eq!(find_snippets("aaaa", "aa").1, 2, "matches do not overlap");
        let long = "x".repeat(MAX_FIND_TEXT - 1) + "y";
        assert_eq!(find_snippets(&long, &"x".repeat(FIND_QUERY_MAX)).1, (MAX_FIND_TEXT - 1) / FIND_QUERY_MAX);
    }

    #[test]
    fn a_site_sheet_holds_only_its_hosts_css_and_valid_zaps() {
        assert!(site_sheet("a.test", "", &[], &[]).is_none());
        let script = site_sheet("a.test", "body{color:red}", &["#ad".into(), "</style>".into()], &[]).unwrap();
        assert!(script.contains("a.test") && script.contains("body{color:red}") && script.contains("#ad {"));
        assert!(!script.contains("</style>"), "an invalid selector never reaches the page");
        assert!(site_sheet("a.test", "", &["#ad".into()], &[]).unwrap().contains("display: none"));
    }

    #[test]
    fn theme_sheets_come_before_the_hosts_own_and_reach_only_their_origins() {
        let theme = vec![("*".to_owned(), "p{color:#111}".to_owned()), ("b.test,*.c.test".to_owned(), "a{color:#222}".to_owned())];
        let script = site_sheet("a.test", "body{color:red}", &[], &theme).unwrap();
        let (first, own) = (script.find("p{color:#111}").unwrap(), script.find("body{color:red}").unwrap());
        assert!(first < own, "the user's CSS comes last so it wins");
        assert_eq!(theme_origins(&theme), ["*", "http://*.c.test", "http://b.test", "http://c.test", "https://*.c.test", "https://b.test", "https://c.test"]);
        assert_eq!(theme_origins(&theme[1..]).len(), 6);
        assert!(theme_origins(&[]).is_empty());
    }
}
