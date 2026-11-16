use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use block2::RcBlock;
use engine::page_scripts::{self, PICKER};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKUserScript, WKUserScriptInjectionTime};
use serde_json::{Value, json};
use shield::ShieldLevel;

use crate::component::Inner;

const CANCEL_PICK: &str = "if (window.tarnPick) window.tarnPick();";
const TAPPABLE: &str = "const tarnRoot = document.documentElement, tarnTap = () => {};\ntarnRoot.addEventListener('click', tarnTap, true);\ntry {\n";
const TAPPABLE_END: &str = "\n} finally { tarnRoot.removeEventListener('click', tarnTap, true); }";
const ZAP_LISTS_KEPT: usize = 8;
const MAX_TEXT: usize = 4 * 1024;

thread_local! {
    static ZAP_LISTS: RefCell<HashMap<String, Retained<WKContentRuleList>>> = RefCell::new(HashMap::new());
}

pub fn host_map(value: Option<&Value>) -> Vec<(String, Value)> {
    value.and_then(Value::as_object).cloned().unwrap_or_default().into_iter().map(|(h, v)| (h.to_ascii_lowercase(), v)).collect()
}

pub(crate) fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

impl Inner {
    fn locked(&self) -> bool {
        let built = self.built.borrow();
        let Some(built) = built.as_ref() else { return true };
        built.level == ShieldLevel::Lockdown || built.web.current_url().is_none_or(|u| self.props.borrow().site(&u) == ShieldLevel::Lockdown)
    }

    fn run(&self, body: Option<String>, done: impl Fn(String) + 'static) {
        let web = self.web().filter(|_| !self.locked());
        let (Some(web), Some(body)) = (web, body) else { return done(String::new()) };
        // SAFETY: main thread (marker); the script runs in the shell's isolated world, not the page's.
        let world = unsafe { WKContentWorld::defaultClientWorld(self.mtm) };
        let block = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
            // SAFETY: WebKit passes the script's result, or null, on the main thread for the duration of the call.
            let text = unsafe { value.as_ref() }.and_then(|v| v.downcast_ref::<NSString>()).map(|s| s.to_string()).unwrap_or_default();
            done(text);
        });
        web.call_async(&NSString::from_str(&body), None, None, &world, Some(&block));
    }

    pub(crate) fn script_command(&self, name: &str, args: &Value) -> bool {
        let seq = args.get("seq").cloned().unwrap_or(Value::Null);
        let url = self.web().and_then(|w| w.current_url());
        let shown = url.as_ref().map(|u| u.to_string()).unwrap_or_default();
        let context = self.context.clone();
        match name {
            "readArticle" => {
                let probe = args.get("probe").and_then(Value::as_bool).unwrap_or(false);
                let body = url.as_ref().map(|u| engine::reader::script(&u.origin().ascii_serialization(), probe));
                self.run(body, move |text| context.emit("article", json!({ "seq": seq, "url": shown, "text": text })));
            }
            "readText" => {
                let max = args.get("maxChars").and_then(Value::as_u64).map_or(MAX_TEXT, |m| (m as usize).min(MAX_TEXT));
                let body = url.as_ref().and_then(|u| page_scripts::page_text_script(u, max));
                self.run(body, move |text| context.emit("pageText", json!({ "seq": seq, "url": shown, "text": text })));
            }
            "findText" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or_default();
                let body = (!query.is_empty()).then(|| page_scripts::find_text_script(query));
                self.run(body, move |text| context.emit("textFound", json!({ "seq": seq, "url": shown, "snippets": page_scripts::find_snippets(&text) })));
            }
            "pick" => self.run(Some(format!("{TAPPABLE}{PICKER}{TAPPABLE_END}")), move |selector| {
                context.emit("picked", json!({ "seq": seq, "url": shown, "selector": selector }))
            }),
            "pickCancel" => {
                if let Some(web) = self.web() {
                    // SAFETY: main thread (marker); the picker lives in the shell's isolated world.
                    let world = unsafe { WKContentWorld::defaultClientWorld(self.mtm) };
                    web.evaluate(&NSString::from_str(CANCEL_PICK), None, &world, None);
                }
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn apply_content(self: &Rc<Self>) {
        let props = self.props.borrow().clone();
        let (styles, zaps) = {
            let built = self.built.borrow();
            let Some(built) = built.as_ref() else { return };
            if built.level == ShieldLevel::Lockdown {
                return;
            }
            let zaps = shield::lists::compile_zaps(props.zaps.iter().map(|(h, s)| (h.as_str(), s.as_str()))).unwrap_or_default();
            let sheets: Vec<(String, String)> = props.theme_styles.iter().chain(&props.styles).cloned().collect();
            ((built.styles.as_ref() != Some(&sheets)).then_some(sheets), (built.zaps.as_ref() != Some(&zaps)).then_some(zaps))
        };
        if let Some(sheets) = styles {
            let mut built = self.built.borrow_mut();
            let Some(built) = built.as_mut() else { return };
            // SAFETY: main thread (marker); the script is a start-of-document user script in the shell's
            // isolated world for the main frame only, holding the style sheets as JS string literals.
            built.site_script = page_scripts::site_styles_script(&sheets).map(|source| unsafe {
                let world = WKContentWorld::defaultClientWorld(self.mtm);
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                    WKUserScript::alloc(self.mtm),
                    &NSString::from_str(&source),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                    &world,
                )
            });
            crate::ext_view::install_scripts(built, self.mtm);
            built.styles = Some(sheets);
        }
        if let Some(zaps) = zaps {
            self.apply_zaps(zaps);
        }
    }

    fn apply_zaps(self: &Rc<Self>, zaps: String) {
        if let Some(built) = self.built.borrow_mut().as_mut() {
            built.zaps = Some(zaps.clone());
        }
        if zaps.is_empty() {
            return self.use_zaps(&zaps, None);
        }
        if let Some(list) = ZAP_LISTS.with(|l| l.borrow().get(&zaps).cloned()) {
            return self.use_zaps(&zaps, Some(list));
        }
        // SAFETY: main thread (marker); the default store exists in an app container.
        let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(self.mtm) }) else { return };
        let this = Rc::downgrade(self);
        let key = zaps.clone();
        let done = RcBlock::new(move |list: *mut WKContentRuleList, _error: *mut NSError| {
            // SAFETY: WebKit passes a valid rule list or null, retained for the duration of the call.
            let Some(list) = (unsafe { Retained::retain(list) }) else { return };
            ZAP_LISTS.with(|l| {
                let mut lists = l.borrow_mut();
                if lists.len() >= ZAP_LISTS_KEPT {
                    lists.clear();
                }
                lists.insert(key.clone(), list.clone());
            });
            if let Some(inner) = this.upgrade() {
                inner.use_zaps(&key, Some(list));
            }
        });
        let id = NSString::from_str(&format!("tarn.zaps.{:016x}", fnv(&zaps)));
        // SAFETY: main thread; the arguments are valid and WebKit copies the block.
        unsafe { store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&id), Some(&NSString::from_str(&zaps)), Some(&done)) };
    }

    fn use_zaps(&self, zaps: &str, list: Option<Retained<WKContentRuleList>>) {
        let mut built = self.built.borrow_mut();
        let Some(built) = built.as_mut().filter(|b| b.zaps.as_deref() == Some(zaps)) else { return };
        // SAFETY: main thread; the controller belongs to this view and takes effect from the next load.
        unsafe {
            if let Some(old) = built.zap_list.take() {
                built.controller.removeContentRuleList(&old);
            }
            if let Some(list) = &list {
                built.controller.addContentRuleList(list);
            }
        }
        built.zap_list = list;
    }
}
