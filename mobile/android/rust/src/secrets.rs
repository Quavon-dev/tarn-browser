use ai::{AiError, Secrets};

/// Secrets sealed with an AES key that never leaves the Android Keystore (`Secrets.java`).
#[derive(Debug, Default)]
pub struct Keystore;

fn name(service: &str, account: &str) -> String {
    format!("{service}/{account}")
}

impl Secrets for Keystore {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, AiError> {
        crate::host::secret("secretGet", "(Ljava/lang/String;)Ljava/lang/String;", &[&name(service, account)]).map_err(AiError::Secrets)
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), AiError> {
        crate::host::secret("secretSet", "(Ljava/lang/String;Ljava/lang/String;)V", &[&name(service, account), secret]).map(drop).map_err(AiError::Secrets)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), AiError> {
        crate::host::secret("secretDelete", "(Ljava/lang/String;)V", &[&name(service, account)]).map(drop).map_err(AiError::Secrets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_java_host_the_store_fails_instead_of_answering_empty() {
        assert!(matches!(Keystore.get("s", "a"), Err(AiError::Secrets(_))));
        assert!(matches!(Keystore.set("s", "a", "v"), Err(AiError::Secrets(_))));
        assert!(matches!(Keystore.delete("s", "a"), Err(AiError::Secrets(_))));
    }
}
