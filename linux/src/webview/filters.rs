use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use browser_core::site::Sets;
use engine::nav_policy::Site;
use shield::rules::builtin_rules;
use shield::{Category, ShieldLevel};
use url::Url;
use webkit6::{UserContentFilter, UserContentFilterStore, gio, glib};

use shield::groups as bundle;

type Waiter = Box<dyn FnOnce()>;

enum Groups {
    Idle,
    Loading(Vec<Waiter>),
    Ready(HashMap<String, Vec<UserContentFilter>>),
}

thread_local! {
    static STORE: UserContentFilterStore = UserContentFilterStore::new(&crate::data::root().join("webkit-filters").to_string_lossy());
    static BUILTIN: RefCell<HashMap<ShieldLevel, UserContentFilter>> = RefCell::default();
    static GROUPS: RefCell<Groups> = const { RefCell::new(Groups::Idle) };
    static SETS: RefCell<Sets> = RefCell::default();
    static SET_CATEGORIES: RefCell<Option<Vec<Category>>> = const { RefCell::new(None) };
}

pub fn site(level: ShieldLevel, rules: &browser_core::site::SiteRules, url: &Url) -> Site {
    SETS.with(|sets| browser_core::site::site(level, rules, sets, url))
}

pub fn install_sets(incoming: Sets) {
    let keep = SET_CATEGORIES.with_borrow(|c| c.clone().unwrap_or_default());
    SETS.with_borrow_mut(|sets| sets.install(incoming, &keep));
    super::sets_changed();
}

/// Reads the domain sets of `categories` from the installed bundle once they change; the sets of
/// categories switched off are dropped at once and never read.
pub fn want_sets(categories: Vec<Category>) {
    if SET_CATEGORIES.with_borrow(|c| c.as_ref() == Some(&categories)) {
        return;
    }
    SET_CATEGORIES.set(Some(categories.clone()));
    install_sets(Sets::default());
    if categories.is_empty() {
        return;
    }
    let dir = crate::data::root().join("filters");
    glib::MainContext::default().spawn_local(async move {
        let key = bundle::public_key();
        let read = gio::spawn_blocking(move || shield::cache::read(&dir, &key).map(|b| Sets::from_bundle(&b, &categories)).map_err(|e| format!("{e:?}"))).await;
        match read.unwrap_or_else(|_| Err("reader panicked".into())) {
            Ok(sets) => install_sets(sets),
            Err(e) => eprintln!("tarn: no domain sets installed ({e})"),
        }
    });
}

async fn compile(identifier: String, source: String) -> Result<UserContentFilter, String> {
    let store = STORE.with(Clone::clone);
    store.save_future(&identifier, &glib::Bytes::from_owned(source.into_bytes())).await.map_err(|e| e.to_string())
}

fn ready_groups() {
    let dir = crate::data::root().join("filters");
    glib::MainContext::default().spawn_local(async move {
        let key = bundle::public_key();
        let loaded = gio::spawn_blocking(move || bundle::load(&dir, &key)).await.unwrap_or_else(|_| Err("converter panicked".into()));
        let mut compiled = HashMap::new();
        match loaded {
            Ok(groups) => {
                for group in groups {
                    let mut filters = Vec::new();
                    for (n, chunk) in group.chunks.into_iter().enumerate() {
                        let id = format!("tarn-{}-{}-{n}", group.name, &group.key[..16.min(group.key.len())]);
                        match compile(id, chunk).await {
                            Ok(filter) => filters.push(filter),
                            Err(e) => eprintln!("tarn: filter list {} chunk {n} did not compile: {e}", group.name),
                        }
                    }
                    eprintln!("tarn: filter list {} ready ({} chunks)", group.name, filters.len());
                    compiled.insert(group.name.to_owned(), filters);
                }
            }
            Err(e) => eprintln!("tarn: no filter lists installed ({e})"),
        }
        let waiters = GROUPS.with_borrow_mut(|g| match std::mem::replace(g, Groups::Ready(compiled)) {
            Groups::Loading(waiters) => waiters,
            _ => Vec::new(),
        });
        waiters.into_iter().for_each(|w| w());
    });
}

fn when_groups_ready(then: Waiter) {
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
