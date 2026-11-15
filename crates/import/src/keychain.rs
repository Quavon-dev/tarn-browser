use security_framework::passwords::get_generic_password;
use zeroize::Zeroizing;

use crate::{Error, Result};

const ITEM_NOT_FOUND: i32 = -25300;
const USER_CANCELED: i32 = -128;
const AUTH_FAILED: i32 = -25293;
const INTERACTION_NOT_ALLOWED: i32 = -25308;

/// The browser's Safe Storage password. macOS asks the user the first time Tarn reads it.
pub(crate) fn password(service: &str, account: &str) -> Result<Zeroizing<Vec<u8>>> {
    get_generic_password(service, account).map(Zeroizing::new).map_err(|e| match e.code() {
        ITEM_NOT_FOUND => Error::NotFound,
        USER_CANCELED | AUTH_FAILED | INTERACTION_NOT_ALLOWED => Error::KeychainDenied,
        code => Error::Keychain(code),
    })
}
