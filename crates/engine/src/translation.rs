//! On-device page translation with Apple's Translation framework (macOS 26 and later), through
//! the Swift shim in `translation.swift`. Only language packs already installed in System Settings
//! are used; the session cannot download one, so translating never leaves the Mac.

/// Text nodes read from one page, and characters in each node and in all of them.
pub const MAX_NODES: usize = 2_000;
pub const MAX_NODE_CHARS: usize = 2_000;
pub const MAX_CHARS: usize = 100_000;
/// Nodes sent to the framework at once, so the first part of a long page is written back early.
pub const BATCH: usize = 64;

/// BCP 47 codes for undetermined, multiple, special or no linguistic content.
const NOT_A_LANGUAGE: [&str; 4] = ["und", "mul", "mis", "zxx"];

/// The primary subtag of a BCP 47 tag (`de` for `de-CH`), lowercase; `None` for anything else.
pub fn language(tag: &str) -> Option<String> {
    let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    ((2..=3).contains(&primary.len()) && primary.bytes().all(|b| b.is_ascii_lowercase()) && !NOT_A_LANGUAGE.contains(&primary.as_str())).then_some(primary)
}

/// The page's language when it is declared and none of the user's preferred languages.
pub fn offer(page: &str, preferred: &[String]) -> Option<String> {
    let page = language(page)?;
    (!preferred.is_empty() && !preferred.iter().any(|p| language(p).as_ref() == Some(&page))).then_some(page)
}

/// What the client world does on the page: report the declared language, read the text nodes
/// worth translating (remembered in the client world, so the page never sees the list), or write
/// translations back into them as text, never as markup.
pub enum Step {
    Language,
    Read,
    Write { start: usize, texts: Vec<String> },
}

impl Step {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn script(&self) -> String {
        match self {
            Step::Language => "return (document.documentElement.lang || '').slice(0, 35);".into(),
            Step::Read => format!(
                r"if (Array.isArray(window.tarnTranslation)) return '[]';
                const skip = 'script,style,noscript,textarea,input,select,code,pre,svg,math,[contenteditable],[translate=no],.notranslate';
                const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT, {{
                    acceptNode: n => {{
                        const p = n.parentElement;
                        if (!p || p.closest(skip) || (p.checkVisibility && !p.checkVisibility())) return NodeFilter.FILTER_REJECT;
                        return /\p{{L}}/u.test(n.data) && n.data.length <= {MAX_NODE_CHARS} ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
                    }}
                }});
                const nodes = [];
                let total = 0;
                while (nodes.length < {MAX_NODES} && walker.nextNode()) {{
                    total += walker.currentNode.data.length;
                    if (total > {MAX_CHARS}) break;
                    nodes.push(walker.currentNode);
                }}
                window.tarnTranslation = nodes;
                window.tarnOriginal = nodes.map(n => n.data.trim());
                return JSON.stringify(window.tarnOriginal);"
            ),
            Step::Write { start, texts } => {
                let texts = serde_json::to_string(texts).unwrap_or_else(|_| "[]".into());
                format!(
                    r"const nodes = Array.isArray(window.tarnTranslation) ? window.tarnTranslation : [];
                    const original = Array.isArray(window.tarnOriginal) ? window.tarnOriginal : [];
                    {texts}.forEach((t, i) => {{
                        const n = nodes[{start} + i];
                        // Text the page changed since it was read keeps the page's version.
                        if (typeof t !== 'string' || !n || n.nodeType !== Node.TEXT_NODE || !n.isConnected || n.data.trim() !== original[{start} + i]) return;
                        const d = n.data, head = d.length - d.trimStart().length, tail = d.trimEnd().length;
                        n.data = d.slice(0, head) + t + d.slice(Math.max(head, tail));
                    }});
                    return '';"
                )
            }
        }
    }
}

pub fn batches(texts: Vec<String>) -> Vec<(usize, Vec<String>)> {
    texts.chunks(BATCH).enumerate().map(|(i, c)| (i * BATCH, c.to_vec())).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum TranslateError {
    NotInstalled,
    Failed,
}

#[cfg(all(target_os = "macos", feature = "translation"))]
mod ffi {
    use std::ffi::{CStr, CString, c_char, c_void};

    use super::TranslateError;

    type Done = Box<dyn FnOnce(Result<Vec<String>, TranslateError>) + Send>;

    unsafe extern "C" {
        fn tarn_translate(
            texts: *const c_char,
            source: *const c_char,
            target: *const c_char,
            context: *mut c_void,
            done: extern "C" fn(*mut c_void, *const c_char),
        );
    }

    extern "C" fn finished(context: *mut c_void, result: *const c_char) {
        // SAFETY: `context` is the box `translate` leaked, and Swift calls this exactly once.
        let done = unsafe { Box::from_raw(context.cast::<Done>()) };
        // SAFETY: Swift passes a NUL-terminated string that lives for this call.
        let text = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        done(super::parse(&text));
    }

    /// Translates `texts` from `source` to `target` (language tags); `done` runs once on a
    /// background thread.
    pub fn translate(texts: &[String], source: &str, target: &str, done: impl FnOnce(Result<Vec<String>, TranslateError>) + Send + 'static) {
        let json = serde_json::to_string(texts).ok().and_then(|j| CString::new(j).ok());
        let (Some(json), Some(source), Some(target)) =
            (json, super::language(source).and_then(|l| CString::new(l).ok()), super::language(target).and_then(|l| CString::new(l).ok()))
        else {
            return done(Err(TranslateError::Failed));
        };
        let done: Box<Done> = Box::new(Box::new(done));
        // SAFETY: the strings outlive the call (Swift copies them first); `finished` takes back
        // the leaked box exactly once.
        unsafe { tarn_translate(json.as_ptr(), source.as_ptr(), target.as_ptr(), Box::into_raw(done).cast(), finished) };
    }
}

#[cfg(all(target_os = "macos", feature = "translation"))]
pub use ffi::translate;

/// The user's preferred languages, first the one pages are translated into.
#[cfg(target_os = "macos")]
pub fn preferred_languages() -> Vec<String> {
    objc2_foundation::NSLocale::preferredLanguages().iter().map(|l| l.to_string()).collect()
}

/// `TranslationSession(installedSource:target:)` exists from macOS 26.
#[cfg(target_os = "macos")]
pub fn supported() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion { majorVersion: 26, minorVersion: 0, patchVersion: 0 })
}

/// `code`'s name in the user's language, such as "German" for `de`.
#[cfg(target_os = "macos")]
pub fn language_name(code: &str) -> String {
    objc2_foundation::NSLocale::currentLocale()
        .localizedStringForLanguageCode(&objc2_foundation::NSString::from_str(code))
        .map_or_else(|| code.to_owned(), |name| name.to_string())
}

/// The shim's answer: a JSON array of translations, or an error text.
#[cfg_attr(not(feature = "translation"), allow(dead_code))]
fn parse(text: &str) -> Result<Vec<String>, TranslateError> {
    match serde_json::from_str(text) {
        Ok(texts) => Ok(texts),
        Err(_) if text == "not installed" => Err(TranslateError::NotInstalled),
        Err(_) => Err(TranslateError::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_only_a_declared_language_that_differs() {
        let user = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(offer("de-CH", &user(&["en-US"])), Some("de".into()));
        assert_eq!(offer("EN", &user(&["en-GB"])), None);
        assert_eq!(offer("en", &user(&["de-DE", "en-US"])), None);
        assert_eq!(offer("fr", &user(&["de-DE", "en-US"])), Some("fr".into()));
        assert_eq!(offer("", &user(&["en"])), None);
        assert_eq!(offer("de", &[]), None);
        assert_eq!(offer("x-klingon", &user(&["en"])), None);
        assert_eq!(offer("und", &user(&["en"])), None);
        assert_eq!(offer("zh_Hant", &user(&["en"])), Some("zh".into()));
        assert_eq!(language("1234"), None);
    }

    #[test]
    fn write_back_is_text_and_bounded_to_read_nodes() {
        let script = Step::Write { start: 64, texts: vec!["<b>x</b>".into(), "\u{2028}'".into()] }.script();
        assert!(script.contains("n.data = ") && script.contains("Node.TEXT_NODE") && !script.contains("innerHTML"));
        assert!(script.contains(r#"["<b>x</b>","#), "{script}");
        assert!(script.contains("nodes[64 + i]"));
    }

    #[test]
    fn batches_keep_their_offsets() {
        let texts: Vec<String> = (0..BATCH + 3).map(|i| i.to_string()).collect();
        let b = batches(texts);
        assert_eq!(b.len(), 2);
        assert_eq!((b[1].0, b[1].1.len()), (BATCH, 3));
    }

    #[test]
    fn shim_answers_parse() {
        assert_eq!(parse(r#"["Hallo"]"#), Ok(vec!["Hallo".to_owned()]));
        assert_eq!(parse("not installed"), Err(TranslateError::NotInstalled));
        assert_eq!(parse("failed"), Err(TranslateError::Failed));
    }
}
