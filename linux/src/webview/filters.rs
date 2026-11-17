use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use browser_core::site::Sets;
use shield::domains::DomainSet;
use shield::rules::{builtin_rules, youtube_rules};
use shield::youtube;
use shield::{Category, ShieldLevel};
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
    pub(super) static MALWARE: RefCell<Option<Arc<DomainSet>>> = const { RefCell::new(None) };
    static YOUTUBE_SCRIPT: RefCell<String> = RefCell::new(youtube::script(&youtube::Config::default()));
}

pub fn youtube_script() -> String {
    YOUTUBE_SCRIPT.with_borrow(Clone::clone)
}

async fn compile(identifier: String, source: String) -> Result<UserContentFilter, String> {
    let store = STORE.with(Clone::clone);
    store.save_future(&identifier, &glib::Bytes::from_owned(source.into_bytes())).await.map_err(|e| e.to_string())
}

fn ready_groups() {
    let dir = crate::data::root().join("filters");
    glib::MainContext::default().spawn_local(async move {
        let key = bundle::public_key();
        let loaded = gio::spawn_blocking(move || {
            let cached = shield::cache::read(&dir, &key).map_err(|e| format!("{e:?}"))?;
            Ok((bundle::of(&cached), Sets::from_bundle(&cached, &[Category::Malware]).malware, youtube::config(&cached)))
        })
        .await
        .unwrap_or_else(|_| Err(String::from("converter panicked")));
        let mut compiled = HashMap::new();
        let rules = youtube_rules();
        let id = format!("tarn-{}-{}-0", bundle::YOUTUBE_GROUP, &bundle::hex(shield::lists::fingerprint(&[&rules]))[..16]);
        if let Ok(filter) = compile(id, rules).await.inspect_err(|e| eprintln!("tarn: YouTube rules did not compile: {e}")) {
            compiled.insert(bundle::YOUTUBE_GROUP.to_owned(), vec![filter]);
        }
        match loaded {
            Ok((groups, malware, config)) => {
                MALWARE.with_borrow_mut(|m| *m = malware);
                YOUTUBE_SCRIPT.with_borrow_mut(|s| *s = youtube::script(&config));
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

pub fn malware() -> Option<Arc<DomainSet>> {
    MALWARE.with_borrow(Clone::clone)
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
