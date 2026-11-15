#[cfg(target_os = "ios")]
mod component;
mod data;
#[cfg(target_os = "ios")]
mod fetch;
pub mod find;
#[cfg(target_os = "ios")]
mod handler;
#[cfg(target_os = "ios")]
mod scripts;
#[cfg(target_os = "ios")]
mod stores;
#[cfg(target_os = "ios")]
pub mod system;
#[cfg(target_os = "ios")]
mod transfers;
#[cfg(target_os = "ios")]
mod webkit;
pub mod zoom;

#[cfg(target_os = "ios")]
pub use component::register;
pub use data::{BURNER_PREFIX, Data, PRIVATE_PREFIX, StoreKind, burner_id, store_kind};
