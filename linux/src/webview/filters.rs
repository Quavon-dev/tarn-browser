use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use shield::ShieldLevel;
use shield::rules::builtin_rules;
use webkit6::{UserContentFilter, UserContentFilterStore, gio, glib};

use shield::cache;
use shield::groups::{self as bundle, Group};
use shield::update::{Status, describe};

pub type Waiter = Box<dyn FnOnce()>;

enum Groups {
    Idle,
    Loading(Vec<Waiter>),
    Ready(HashMap<String, Vec<UserContentFilter>>),
}

thread_local! {
    static STORE: UserContentFilterStore = UserContentFilterStore::new(&crate::data::root().join("webkit-filters").to_string_lossy());
    static BUILTIN: RefCell<HashMap<ShieldLevel, UserContentFilter>> = RefCell::default();
    static GROUPS: RefCell<Groups> = const { RefCell::new(Groups::Idle) };
    static LOADED: RefCell<Status> = RefCell::default();
}

async fn compile(identifier: String, source: String) -> Result<UserContentFilter, String> {
    let store = STORE.with(Clone::clone);
    store.save_future(&identifier, &glib::Bytes::from_owned(source.into_bytes())).await.map_err(|e| e.to_string())
}

async fn compile_group(group: Group) -> Result<Vec<UserContentFilter>, String> {
    let mut filters = Vec::new();
    for (n, chunk) in group.chunks.into_iter().enumerate() {
        let id = format!("tarn-{}-{}-{n}", group.name, &group.key[..16.min(group.key.len())]);
        filters.push(compile(id, chunk).await.map_err(|e| format!("{}: {e}", group.name))?);
    }
    Ok(filters)
}

fn ready_groups() {
    let dir = crate::data::root().join("filters");
    glib::MainContext::default().spawn_local(async move {
        let key = bundle::public_key();
        let floor = cache::load_state(&dir).version;
        let loaded = gio::spawn_blocking(move || -> Result<(Vec<Group>, shield::bundle::VerifiedBundle), String> {
            let verified = cache::read(&dir, &key).map_err(|e| describe(&e))?;
            cache::raise_floor(&dir, verified.version).ok();
            Ok((bundle::of(&verified), verified))
        })
        .await
        .unwrap_or_else(|_| Err("converter panicked".into()));
        let mut compiled = HashMap::new();
        match loaded {
            Ok((groups, verified)) => {
                LOADED.with_borrow_mut(|s| s.installed(&verified));
                for group in groups {
                    let name = group.name;
                    match compile_group(group).await {
                        Ok(filters) => {
                            eprintln!("tarn: filter list {name} ready ({} chunks)", filters.len());
                            compiled.insert(name.to_owned(), filters);
                        }
                        Err(e) => eprintln!("tarn: filter list did not compile: {e}"),
                    }
                }
            }
            Err(e) => {
                eprintln!("tarn: no filter lists installed ({e})");
                if floor.is_some() {
                    LOADED.with_borrow_mut(|s| s.error = Some(e));
                }
            }
        }
        let waiters = GROUPS.with_borrow_mut(|g| match std::mem::replace(g, Groups::Ready(compiled)) {
            Groups::Loading(waiters) => waiters,
            _ => Vec::new(),
        });
        waiters.into_iter().for_each(|w| w());
    });
}

pub fn loaded() -> Status {
    LOADED.with_borrow(Clone::clone)
}

pub fn replace(groups: Vec<Group>, done: impl FnOnce(Result<(), String>) + 'static) {
    glib::MainContext::default().spawn_local(async move { done(replace_groups(groups).await) });
}

async fn replace_groups(groups: Vec<Group>) -> Result<(), String> {
    let mut first_error = None;
    for group in groups {
        let name = group.name;
        match compile_group(group).await {
            Ok(filters) => {
                let old = GROUPS.with_borrow_mut(|g| match g {
                    Groups::Ready(compiled) => compiled.insert(name.to_owned(), filters.clone()).unwrap_or_default(),
                    _ => Vec::new(),
                });
                super::swap_group(name, &old, &filters);
                let kept: Vec<_> = filters.iter().filter_map(UserContentFilter::identifier).collect();
                let store = STORE.with(Clone::clone);
                for id in old.iter().filter_map(UserContentFilter::identifier).filter(|id| !kept.contains(id)) {
                    store.remove_future(&id).await.ok();
                }
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub fn when_groups_ready(then: Waiter) {
    let run_now = GROUPS.with_borrow_mut(|g| match g {
        Groups::Ready(_) => Some(then),
        Groups::Loading(waiters) => {
            waiters.push(then);
            None
        }
        Groups::Idle => {
            *g = Groups::Loading(vec![then]);
            ready_groups();
            None
        }
    });
    if let Some(then) = run_now {
        then();
    }
}

pub fn with_filters(level: ShieldLevel, groups: Vec<String>, then: impl FnOnce(Result<Vec<UserContentFilter>, String>) + 'static) {
    let then = Rc::new(RefCell::new(Some(then)));
    when_groups_ready(Box::new(move || {
        let finish = move |builtin: Result<UserContentFilter, String>| {
            let Some(then) = then.borrow_mut().take() else { return };
            then(builtin.map(|b| {
                let mut all = vec![b];
                GROUPS.with_borrow(|g| {
                    if let Groups::Ready(compiled) = g {
                        all.extend(groups.iter().filter_map(|n| compiled.get(n)).flatten().cloned());
                    }
                });
                all
            }));
        };
        if let Some(filter) = BUILTIN.with_borrow(|b| b.get(&level).cloned()) {
            return finish(Ok(filter));
        }
        glib::MainContext::default().spawn_local(async move {
            let result = compile(format!("tarn-builtin-{}", level.as_str()), builtin_rules(&level.policy())).await;
            if let Ok(filter) = &result {
                BUILTIN.with_borrow_mut(|b| b.insert(level, filter.clone()));
            }
            finish(result);
        });
    }));
}

pub fn compile_one(identifier: String, source: String, then: impl FnOnce(UserContentFilter) + 'static) {
    glib::MainContext::default().spawn_local(async move {
        match compile(identifier, source).await {
            Ok(filter) => then(filter),
            Err(e) => eprintln!("tarn: content rules did not compile: {e}"),
        }
    });
}
