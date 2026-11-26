pub mod agent_scripts;
pub mod agent_socket;
pub mod cert;
#[cfg(unix)]
pub mod chromium_owner;
pub mod content_rules;
pub mod cookies;
pub mod downloads;
pub mod events;
pub mod favicon;
#[cfg(all(any(target_os = "macos", target_os = "ios"), feature = "foundation-models"))]
pub mod foundation_models;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod handover;
pub mod lifecycle;
pub mod nav_policy;
pub mod page_scripts;
pub mod qr;
pub mod reader;
pub mod request_filter;
pub mod startup;
pub mod tarn_ext;
pub mod translation;
#[cfg(target_os = "linux")]
pub mod trim;
pub mod view_policy;

#[cfg(target_os = "macos")]
mod biometrics;
#[cfg(target_os = "macos")]
pub use biometrics::confirm_owner;

/// Windows Hello is not wired yet (docs/port-backlog.md), so the page's own confirmation stands alone.
#[cfg(windows)]
pub fn confirm_owner(_reason: &str, done: impl FnOnce(bool) + Send + 'static) {
    done(true);
}
#[cfg(any(windows, all(target_os = "macos", feature = "chromium")))]
pub mod chromium;
#[cfg(target_os = "macos")]
#[cfg_attr(not(feature = "chromium"), allow(dead_code))]
mod cookies_webkit;
#[cfg(target_os = "macos")]
mod data_store;
#[cfg(target_os = "macos")]
mod delegates;
#[cfg(target_os = "macos")]
pub mod extensions;
#[cfg(target_os = "macos")]
pub mod menu;
#[cfg(any(target_os = "macos", windows))]
mod menu_item;
#[cfg(any(target_os = "macos", windows))]
mod page;
#[cfg(any(target_os = "macos", windows))]
mod prompts;
#[cfg(target_os = "macos")]
pub mod share;
#[cfg(target_os = "macos")]
pub mod speech;
#[cfg(target_os = "macos")]
mod tarn_webkit;
#[cfg(target_os = "macos")]
mod view;
#[cfg(target_os = "macos")]
mod webview;

#[cfg(windows)]
mod data_store_windows;
#[cfg(windows)]
#[path = "extensions_windows.rs"]
pub mod extensions;
#[cfg(windows)]
mod host_windows;
#[cfg(windows)]
#[path = "instance_windows.rs"]
pub mod instance;
#[cfg(windows)]
#[path = "menu_windows.rs"]
pub mod menu;
#[cfg(windows)]
#[path = "share_windows.rs"]
pub mod share;
#[cfg(windows)]
#[path = "speech_windows.rs"]
pub mod speech;
#[cfg(windows)]
mod tarn_page_windows;
#[cfg(windows)]
mod view_windows;

#[cfg(windows)]
pub use chromium::runtime::set_swipe_navigation;
#[cfg(target_os = "macos")]
pub use data_store::{DataStore, FilterLists, builtin_rules_failed};
#[cfg(windows)]
pub use data_store_windows::{DataStore, FilterLists, builtin_rules_failed};
pub use events::{Event, Protection};
#[cfg(windows)]
pub use host_windows::{
    Host, MIN_ZOOM, Popup, alert, app_active, bundle_identifier, is_default_browser, local_app_data, make_default_browser, open_external, option_held,
    reduce_motion, refresh_browser_registration, restrict_to_user, reveal_in_finder, shift_held,
};
pub use nav_policy::{Features, SiteFor};
#[cfg(any(target_os = "macos", windows))]
pub use prompts::{Prompt, PromptKind};
#[cfg(windows)]
pub use tarn_page_windows::ExtensionPage;
#[cfg(target_os = "macos")]
pub use tarn_webkit::ExtensionPage;
#[cfg(target_os = "macos")]
pub use view::WebView;
#[cfg(windows)]
pub use view_windows::WebView;
#[cfg(target_os = "macos")]
pub use webview::{
    Host, MIN_ZOOM, Popup, alert, app_active, bundle_identifier, is_default_browser, make_default_browser, open_external, option_held, reduce_motion,
    reveal_in_finder, shift_held,
};
