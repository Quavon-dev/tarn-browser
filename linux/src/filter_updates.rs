use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use browser_core::Core;
use browser_core::github::Fetch;
use shield::bundle::{FILTER_BUNDLE_URL, Kind, VerifiedBundle};
use shield::cache::{self, State};
use shield::groups::{self, Group, public_key};
use shield::update::{self, Status, Update, configured, now};
use webkit6::{gio, glib};

use crate::data::Data;
use crate::webview::filters;

thread_local! {
    static STATUS: RefCell<Status> = RefCell::default();
}

pub fn status() -> Status {
    STATUS.with_borrow(Clone::clone)
}

pub fn loaded(bundle: &VerifiedBundle) {
    STATUS.with_borrow_mut(|s| s.installed(bundle));
}

pub fn failed(error: String) {
    STATUS.with_borrow_mut(|s| s.error = Some(error));
}

fn dir() -> PathBuf {
    crate::data::root().join("filters")
}

pub fn start(core: Rc<Core<Data>>) {
    if !configured() {
        return;
    }
    filters::when_groups_ready(Box::new(move || {
        glib::MainContext::default().spawn_local(async move {
            let dir = dir();
            let mut state = cache::load_state(&dir);
            loop {
                let updates = core.disk.setting(update::SETTING).ok().flatten().as_deref() != Some("off");
                STATUS.with_borrow_mut(|s| s.updates = updates);
                if update::due(updates, &state, now()) {
                    state.checked = now();
                    let installed = STATUS.with_borrow(|s| s.installed.is_some());
                    let result = check(&core, &dir, &mut state, installed).await;
                    STATUS.with_borrow_mut(|s| s.checked(result));
                    cache::save_state(&dir, &state).ok();
                    crate::ui::refresh_panels();
                }
                glib::timeout_future_seconds(update::TICK_SECS as u32).await;
            }
        });
    }));
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    gio::spawn_blocking(work).await.unwrap_or_else(|_| Err("the update stopped unexpectedly".into()))
}

async fn check(core: &Core<Data>, dir: &std::path::Path, state: &mut State, installed: bool) -> Result<(), String> {
    let last = update::floor(state, installed);
    let fetched = blocking(move || {
        let net = crate::fetch::Soup::new();
        update::download(|url, max| net.get(url, max).map_err(|e| e.describe()), FILTER_BUNDLE_URL, &public_key(), last, now())
    })
    .await?;
    let Some(Update { manifest, signature, bundle }) = fetched else { return Ok(()) };
    let (converted, bundle): (Vec<Group>, VerifiedBundle) = blocking(move || Ok((groups::of(&bundle), bundle))).await?;
    filters::replace(converted).await?;
    let write_dir = dir.to_owned();
    let bundle = blocking(move || cache::write(&write_dir, &manifest, &signature, &bundle).map(|()| bundle).map_err(|e| e.to_string())).await?;
    state.version = Some(bundle.version);
    loaded(&bundle);
    let revocations = bundle.of(shield::revocations::CATEGORY, Kind::Config).next().and_then(|l| shield::revocations::parse(&l.data));
    if revocations.is_some_and(|list| browser_core::theme_library::merge_revocations(&core.disk, &list)) {
        crate::ui::themes_changed();
    }
    Ok(())
}
