#[cfg(target_os = "ios")]
mod component;
mod data;
pub mod ext_scripts;
#[cfg(target_os = "ios")]
mod ext_view;
#[cfg(target_os = "ios")]
mod extpage;
#[cfg(target_os = "ios")]
mod fetch;
#[cfg(target_os = "ios")]
mod find;
#[cfg(target_os = "ios")]
mod handler;
#[cfg(target_os = "ios")]
mod page_root;
#[cfg(target_os = "ios")]
mod scripts;
pub mod store_link;
#[cfg(target_os = "ios")]
mod stores;
#[cfg(target_os = "ios")]
pub mod system;
#[cfg(target_os = "ios")]
pub mod tarn_ext;
#[cfg(target_os = "ios")]
mod transfers;
#[cfg(target_os = "ios")]
mod webkit;
pub mod zoom;

#[cfg(target_os = "ios")]
pub use component::register;
pub use data::{BURNER_PREFIX, Data, PRIVATE_PREFIX, StoreKind, burner_id, store_kind};
