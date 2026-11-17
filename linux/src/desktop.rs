use std::cell::OnceCell;

use webkit6::gio;
use webkit6::gio::prelude::*;

pub const DESKTOP_ID: &str = "de.quavon.tarn.desktop";
const HANDLED: [&str; 4] = ["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"];

thread_local! {
    static INSTALLED: OnceCell<bool> = const { OnceCell::new() };
}

fn ours() -> Option<gio::AppInfo> {
    gio::AppInfo::all().into_iter().find(|app| app.id().as_deref() == Some(DESKTOP_ID))
}

pub fn is_default_browser() -> Option<bool> {
    let installed = INSTALLED.with(|i| *i.get_or_init(|| !std::path::Path::new("/.flatpak-info").exists() && ours().is_some()));
    installed.then(|| HANDLED.iter().all(|kind| gio::AppInfo::default_for_type(kind, false).and_then(|app| app.id()).as_deref() == Some(DESKTOP_ID)))
}

pub fn make_default_browser() -> Result<(), String> {
    let app = ours().ok_or_else(|| format!("{DESKTOP_ID} is not installed"))?;
    HANDLED.iter().try_for_each(|kind| app.set_as_default_for_type(kind)).map_err(|e| e.to_string())
}
