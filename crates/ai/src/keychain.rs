#[cfg(target_os = "macos")]
use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

use crate::claude_api::ApiToken;
use crate::provider::AiError;

const SERVICE: &str = "de.quavon.tarn";
const ACCOUNT: &str = "anthropic-api-token";
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

fn token(bytes: Vec<u8>) -> Result<Option<ApiToken>, AiError> {
    String::from_utf8(bytes).map(|s| Some(ApiToken::new(s))).map_err(|_| AiError::Keychain("stored token is not UTF-8".into()))
}

#[cfg(target_os = "macos")]
pub fn store_token(token: &ApiToken) -> Result<(), AiError> {
    set_generic_password(SERVICE, ACCOUNT, token.expose().as_bytes()).map_err(|e| AiError::Keychain(e.to_string()))
}

#[cfg(target_os = "macos")]
pub fn load_token() -> Result<Option<ApiToken>, AiError> {
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => token(bytes),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(e) => Err(AiError::Keychain(e.to_string())),
    }
}

#[cfg(target_os = "macos")]
pub fn delete_token() -> Result<(), AiError> {
    match delete_generic_password(SERVICE, ACCOUNT) {
        Err(e) if e.code() != ERR_SEC_ITEM_NOT_FOUND => Err(AiError::Keychain(e.to_string())),
        _ => Ok(()),
    }
}

#[cfg(windows)]
fn target() -> String {
    format!("{SERVICE}/{ACCOUNT}")
}

#[cfg(windows)]
pub fn store_token(token: &ApiToken) -> Result<(), AiError> {
    platform::credentials::write(&target(), token.expose().as_bytes()).map_err(|e| AiError::Keychain(e.to_string()))
}

#[cfg(windows)]
pub fn load_token() -> Result<Option<ApiToken>, AiError> {
    match platform::credentials::read(&target()) {
        Ok(Some(bytes)) => token(bytes),
        Ok(None) => Ok(None),
        Err(e) => Err(AiError::Keychain(e.to_string())),
    }
}

#[cfg(windows)]
pub fn delete_token() -> Result<(), AiError> {
    platform::credentials::delete(&target()).map_err(|e| AiError::Keychain(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "touches the login keychain"]
    fn round_trip() {
        store_token(&ApiToken::new("sk-ant-keychain-test".into())).unwrap();
        assert_eq!(load_token().unwrap().unwrap().expose(), "sk-ant-keychain-test");
        delete_token().unwrap();
        assert!(load_token().unwrap().is_none());
    }
}
