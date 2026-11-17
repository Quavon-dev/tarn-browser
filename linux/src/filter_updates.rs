use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use browser_core::Core;
use browser_core::github::Fetch;
use browser_core::site::Sets;
use futures_channel::oneshot;
use nsh_core::prelude::*;
use shield::bundle::{FILTER_BUNDLE_URL, Kind, VerifiedBundle};
use shield::cache::{self, State};
use shield::groups::{self, Group, public_key};
use shield::update::{self, Status, Update, configured, now};
use shield::youtube;

use crate::data::Data;
use crate::webview::filters;

thread_local! {
    static STATUS: RefCell<Status> = RefCell::default();
}

pub fn status() -> Status {
    STATUS.with_borrow(Clone::clone)
}

pub fn set_status(status: Status) {
    STATUS.set(status);
}

fn dir() -> PathBuf {
    crate::data::root().join("filters")
}

async fn on_main<T: Send + 'static>(work: impl FnOnce(oneshot::Sender<T>) + Send + 'static) -> Result<T, String> {
    let (tx, rx) = oneshot::channel();
    run_on_main_thread(move || work(tx)).await.map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "the web view stopped unexpectedly".to_owned())
}

async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    blocking(work).await.unwrap_or_else(|e| Err(e.to_string()))
}

pub fn start(core: Rc<Core<Data>>) {
    if !configured() {
        return;
    }
    Owner::detached().run(|| {
        drop(spawn(async move {
            if let Ok(loaded) = on_main(|tx| filters::when_groups_ready(Box::new(move || drop(tx.send(filters::loaded()))))).await {
                set_status(loaded);
            }
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
                timer(Duration::from_secs(update::TICK_SECS)).await;
            }
        }));
    });
}

async fn check(core: &Core<Data>, dir: &Path, state: &mut State, installed: bool) -> Result<(), String> {
    let last = update::floor(state, installed);
    let fetched = off_thread(move || {
        let net = crate::fetch::Soup::new();
        update::download(|url, max| net.get(url, max).map_err(|e| e.describe()), FILTER_BUNDLE_URL, &public_key(), last, now())
    })
    .await?;
    let Some(Update { manifest, signature, bundle }) = fetched else { return Ok(()) };
    let (converted, bundle): (Vec<Group>, VerifiedBundle) = off_thread(move || Ok((groups::of(&bundle), bundle))).await?;
    on_main(move |tx| filters::replace(converted, move |result| drop(tx.send(result)))).await??;
    let write_dir = dir.to_owned();
    let bundle = off_thread(move || cache::write(&write_dir, &manifest, &signature, &bundle).map(|()| bundle).map_err(|e| e.to_string())).await?;
    let categories = on_main(|tx| drop(tx.send(filters::wanted_categories()))).await?;
    let (sets, script, bundle) = off_thread(move || Ok((Sets::from_bundle(&bundle, &categories), youtube::script(&youtube::config(&bundle)), bundle))).await?;
    on_main(move |tx| filters::install_update(sets, script, move || tx.send(()).unwrap_or_default())).await?;
    state.version = Some(bundle.version);
    STATUS.with_borrow_mut(|s| s.installed(&bundle));
    let revocations = bundle.of(shield::revocations::CATEGORY, Kind::Config).next().and_then(|l| shield::revocations::parse(&l.data));
    if revocations.is_some_and(|list| browser_core::theme_library::merge_revocations(&core.disk, &list)) {
        crate::ui::themes_changed();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_engine_thread_reaches_glib_only_through_the_main_thread() {
        let code = include_str!("filter_updates.rs");
        let body = &code[..code.find("#[cfg(test)]").unwrap()];
        for main_only in ["glib::", "gio::", "spawn_local", "webkit6"] {
            assert!(!body.contains(main_only), "{main_only} must run on the GTK main thread, inside on_main");
        }
        for line in body.lines().filter(|l| l.contains("filters::") && !l.starts_with("use ")) {
            assert!(line.contains("on_main("), "the web view's filters are reached only on the main thread: {line}");
        }
    }
}
