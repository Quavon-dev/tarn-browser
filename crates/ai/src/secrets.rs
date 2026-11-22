use crate::claude_api::ApiToken;
use crate::provider::AiError;

const SERVICE: &str = "de.quavon.tarn";
const ACCOUNT: &str = "anthropic-api-token";

/// The platform's secret store. Implementations never fall back to a plain file.
pub trait Secrets: Send + Sync {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError>;
    /// Succeeds when nothing is stored.
    fn delete(&self, service: &str, account: &str) -> Result<(), AiError>;
}

pub fn store_token(secrets: &dyn Secrets, token: &ApiToken) -> Result<(), AiError> {
    secrets.set(SERVICE, ACCOUNT, token.expose())
}

pub fn load_token(secrets: &dyn Secrets) -> Result<Option<ApiToken>, AiError> {
    Ok(secrets.get(SERVICE, ACCOUNT)?.map(ApiToken::new))
}

pub fn delete_token(secrets: &dyn Secrets) -> Result<(), AiError> {
    secrets.delete(SERVICE, ACCOUNT)
}

/// The Keychain on macOS and iOS, the Credential Manager on Windows, the Secret Service on Linux.
/// Android's store lives in its Java host (`tarn_mobile_android::Keystore`).
#[cfg(any(target_os = "macos", target_os = "ios", windows, target_os = "linux"))]
#[derive(Debug, Default)]
pub struct Keyring;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    use super::{Keyring, Secrets};
    use crate::provider::AiError;

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    impl Secrets for Keyring {
        fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError> {
            match get_generic_password(service, account) {
                Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| AiError::Secrets("stored secret is not UTF-8".into())),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(AiError::Secrets(e.to_string())),
            }
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError> {
            set_generic_password(service, account, secret.as_bytes()).map_err(|e| AiError::Secrets(e.to_string()))
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), AiError> {
            match delete_generic_password(service, account) {
                Err(e) if e.code() != ERR_SEC_ITEM_NOT_FOUND => Err(AiError::Secrets(e.to_string())),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{Keyring, Secrets};
    use crate::provider::AiError;

    fn target(service: &str, account: &str) -> String {
        format!("{service}/{account}")
    }

    impl Secrets for Keyring {
        fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError> {
            match platform::credentials::read(&target(service, account)) {
                Ok(Some(bytes)) => String::from_utf8(bytes).map(Some).map_err(|_| AiError::Secrets("stored secret is not UTF-8".into())),
                Ok(None) => Ok(None),
                Err(e) => Err(AiError::Secrets(e.to_string())),
            }
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError> {
            platform::credentials::write(&target(service, account), secret.as_bytes()).map_err(|e| AiError::Secrets(e.to_string()))
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), AiError> {
            platform::credentials::delete(&target(service, account)).map_err(|e| AiError::Secrets(e.to_string()))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;

    use futures_lite::future::block_on;
    use oo7::dbus::{Collection, Service};

    use super::{Keyring, Secrets};
    use crate::provider::AiError;

    fn failed(e: oo7::dbus::Error) -> AiError {
        AiError::Secrets(e.to_string())
    }

    fn attributes<'a>(service: &'a str, account: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([("service", service), ("account", account)])
    }

    async fn collection(service: &Service<'static>) -> Result<Collection<'static>, oo7::dbus::Error> {
        let collection = service.default_collection().await?;
        if collection.is_locked().await? {
            collection.unlock(None).await?;
        }
        Ok(collection)
    }

    impl Secrets for Keyring {
        fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError> {
            block_on(async {
                let dbus = Service::new().await?;
                let Some(item) = collection(&dbus).await?.search_items(&attributes(service, account)).await?.into_iter().next() else {
                    return Ok(None);
                };
                if item.is_locked().await? {
                    item.unlock(None).await?;
                }
                Ok::<_, oo7::dbus::Error>(Some(item.secret().await?))
            })
            .map_err(failed)?
            .map(|secret| String::from_utf8(secret.as_bytes().to_vec()).map_err(|_| AiError::Secrets("stored secret is not UTF-8".into())))
            .transpose()
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError> {
            block_on(async {
                let dbus = Service::new().await?;
                collection(&dbus).await?.create_item("Tarn", &attributes(service, account), secret, true, None).await.map(drop)
            })
            .map_err(failed)
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), AiError> {
            block_on(async {
                let dbus = Service::new().await?;
                for item in collection(&dbus).await?.search_items(&attributes(service, account)).await? {
                    item.delete(None).await?;
                }
                Ok::<_, oo7::dbus::Error>(())
            })
            .map_err(failed)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub(crate) struct Memory(Mutex<HashMap<(String, String), String>>);

    impl Secrets for Memory {
        fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError> {
            Ok(self.0.lock().unwrap().get(&(service.into(), account.into())).cloned())
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError> {
            self.0.lock().unwrap().insert((service.into(), account.into()), secret.into());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), AiError> {
            self.0.lock().unwrap().remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    #[test]
    fn token_round_trip() {
        let secrets = Memory::default();
        assert!(load_token(&secrets).unwrap().is_none());
        store_token(&secrets, &ApiToken::new("sk-ant-secret-test".into())).unwrap();
        assert_eq!(load_token(&secrets).unwrap().unwrap().expose(), "sk-ant-secret-test");
        assert_eq!(secrets.get(SERVICE, ACCOUNT).unwrap().as_deref(), Some("sk-ant-secret-test"));
        delete_token(&secrets).unwrap();
        delete_token(&secrets).unwrap();
        assert!(load_token(&secrets).unwrap().is_none());
    }

    #[cfg(any(target_os = "macos", target_os = "ios", windows, target_os = "linux"))]
    #[test]
    #[ignore = "touches the user's keychain"]
    fn keyring_round_trip() {
        store_token(&Keyring, &ApiToken::new("sk-ant-keychain-test".into())).unwrap();
        assert_eq!(load_token(&Keyring).unwrap().unwrap().expose(), "sk-ant-keychain-test");
        delete_token(&Keyring).unwrap();
        assert!(load_token(&Keyring).unwrap().is_none());
    }
}
