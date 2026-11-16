#[cfg(unix)]
pub mod chromium_owner;
pub mod content_rules;
pub mod cookies;
pub mod downloads;
pub mod events;
pub mod favicon;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod handover;
pub mod lifecycle;
pub mod nav_policy;
pub mod page_scripts;
pub mod qr;
pub mod reader;
pub mod request_filter;
pub mod startup;
pub mod translation;
pub mod view_policy;

#[cfg(target_os = "macos")]
mod biometrics;
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
    Host, MIN_ZOOM, Popup, alert, app_active, bundle_identifier, is_default_browser, make_default_browser, open_external, option_held, reduce_motion,
    reveal_in_finder, shift_held,
};
pub use nav_policy::SiteFor;
#[cfg(any(target_os = "macos", windows))]
pub use prompts::{Prompt, PromptKind};
#[cfg(target_os = "macos")]
pub use view::WebView;
#[cfg(windows)]
pub use view_windows::WebView;
#[cfg(target_os = "macos")]
pub use webview::{
    Host, MIN_ZOOM, Popup, alert, app_active, bundle_identifier, is_default_browser, make_default_browser, open_external, option_held, reduce_motion,
    reveal_in_finder, shift_held,
};
