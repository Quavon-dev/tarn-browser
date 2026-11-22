pub mod capture;
pub mod data;
pub mod events;
pub mod extensions;
pub mod features;
pub mod media;
pub mod platform;
pub mod scripts;
pub mod shell;
pub mod theme;
pub mod themes;
pub mod ui;

pub use ui::App;

use nsh_core::prelude::*;

#[native_module("tarn-mobile/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.assets(assets!());
}

#[cfg(target_os = "ios")]
#[native_components(ios)]
fn components(components: &nsh_platform_ios::Components) {
    tarn_mobile_ios::register(components);
}
