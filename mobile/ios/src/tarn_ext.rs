//! The engine side of Tarn extensions for the shell (`mobile-hooks`). The shell calls these on its
//! own thread; WebKit lives on the main thread, so each call hands owned data to the main queue,
//! and a frame's question to the gate travels as the `extensionFrame` event and comes back here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use engine::tarn_ext::{FrameNav, Resources, World, WorldScripts};
use objc2_foundation::NSOperationQueue;
use serde_json::Value;
use url::Url;

use crate::component::Inner;

pub use engine::tarn_ext::MAX_MESSAGE_BYTES;

pub type Gate = Rc<dyn Fn(&str, &str, &World, &FrameNav<'_>) -> Vec<usize>>;

thread_local! {
    static GATE: RefCell<Option<Gate>> = const { RefCell::new(None) };
    static SCRIPTS: RefCell<HashMap<String, Vec<Rc<WorldScripts>>>> = RefCell::new(HashMap::new());
    static RULES: RefCell<HashMap<String, Vec<(String, String)>>> = RefCell::new(HashMap::new());
    static SERVED: RefCell<HashMap<String, Resources>> = RefCell::new(HashMap::new());
    static VIEWS: RefCell<Vec<Weak<Inner>>> = const { RefCell::new(Vec::new()) };
}

fn on_main(work: impl FnOnce() + Send + 'static) {
    let work = Cell::new(Some(work));
    let block = RcBlock::new(move || {
        if let Some(work) = work.take() {
            work();
        }
    });
    // SAFETY: the main queue runs the block once on the main thread; it captures only owned data
    // that may move between threads.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
}

/// Registered on the shell's thread, where `frame_event` asks it.
pub fn set_gate(gate: Gate) {
    GATE.with(|g| *g.borrow_mut() = Some(gate));
}

pub fn set_scripts(store_id: &str, worlds: Vec<Rc<WorldScripts>>) {
    let (store, worlds): (String, Vec<WorldScripts>) = (store_id.to_owned(), worlds.iter().map(|w| (**w).clone()).collect());
    on_main(move || {
        SCRIPTS.with(|s| s.borrow_mut().insert(store.clone(), worlds.into_iter().map(Rc::new).collect()));
        refresh(&store);
    });
}

pub fn set_rule_lists(store_id: &str, lists: Vec<(String, String)>) {
    let store = store_id.to_owned();
    on_main(move || {
        RULES.with(|r| r.borrow_mut().insert(store.clone(), lists));
        refresh(&store);
    });
}

pub fn serve(store_id: &str, resources: Resources) {
    let store = store_id.to_owned();
    on_main(move || {
        SERVED.with(|s| s.borrow_mut().insert(store, resources));
    });
}

pub fn worlds_supported() -> bool {
    true
}

/// Answers a view's `extensionFrame` question on the shell's thread with the gate, then lets the
/// held navigation go on with what the gate approved.
pub fn frame_event(data: &Value) {
    let text = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or_default();
    let (Some(request), Ok(url), Ok(top)) = (data.get("request").and_then(Value::as_u64), Url::parse(text("url")), Url::parse(text("top"))) else { return };
    let main = data.get("main").and_then(Value::as_bool).unwrap_or(false);
    let gate = GATE.with(|g| g.borrow().clone());
    let nav = FrameNav { url: &url, top: &top, main_frame: main };
    let runs: Vec<(String, Vec<usize>)> = data
        .get("worlds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| World::parse(w.as_str()?))
        .map(|world| {
            let run = gate.as_ref().map(|g| g(text("tab"), text("store"), &world, &nav)).unwrap_or_default();
            (world.name().to_owned(), run)
        })
        .collect();
    on_main(move || {
        let views: Vec<Rc<Inner>> = VIEWS.with(|v| v.borrow().iter().filter_map(Weak::upgrade).collect());
        for inner in views {
            if inner.frame_answer(request, &runs) {
                break;
            }
        }
    });
}

pub(crate) fn scripts(store_id: &str) -> Vec<Rc<WorldScripts>> {
    SCRIPTS.with(|s| s.borrow().get(store_id).cloned().unwrap_or_default())
}

pub(crate) fn rules(store_id: &str) -> Vec<(String, String)> {
    RULES.with(|r| r.borrow().get(store_id).cloned().unwrap_or_default())
}

pub(crate) fn resources(store_id: &str) -> Option<Resources> {
    SERVED.with(|s| s.borrow().get(store_id).cloned())
}

pub(crate) fn register(inner: &Rc<Inner>) {
    VIEWS.with(|v| {
        let mut views = v.borrow_mut();
        views.retain(|w| w.strong_count() > 0);
        views.push(Rc::downgrade(inner));
    });
}

fn refresh(store_id: &str) {
    let views: Vec<Rc<Inner>> = VIEWS.with(|v| v.borrow().iter().filter_map(Weak::upgrade).collect());
    for inner in views.into_iter().filter(|i| i.profile() == store_id) {
        inner.apply_extensions();
    }
}
