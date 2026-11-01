//! API keys: an env var named in the config wins; otherwise the OS keyring (Credential Manager,
//! Keychain or Secret Service) under service [`KEYRING_SERVICE`] and the provider's name.
//! Keys are never stored in the config and never logged; [`ApiKey`] prints as `<redacted>`.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::provider::ProviderError;

/// Keyring service name all Hece keys are stored under.
pub const KEYRING_SERVICE: &str = "hece";

/// A secret API key. `Debug` never shows the value.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key; surrounding whitespace (a pasted newline) is dropped.
    #[must_use]
    pub fn new(key: &str) -> Self {
        Self(key.trim().to_owned())
    }

    /// The key itself, for the one request header that needs it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Failure of a [`SecretStore`]; never carries secret material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The store is locked or refused access.
    #[error("keyring is locked")]
    Locked,
    /// Any other failure, as a short kind.
    #[error("keyring error: {0}")]
    Other(String),
}

/// Where keys are kept. Calls may block (D-Bus, Credential Manager).
pub trait SecretStore: Send + Sync + std::fmt::Debug {
    /// Reads the key stored for `account`; `Ok(None)` if there is none.
    fn get(&self, account: &str) -> Result<Option<ApiKey>, StoreError>;
    /// Stores `key` for `account`, replacing any previous one.
    fn set(&self, account: &str, key: &ApiKey) -> Result<(), StoreError>;
    /// Removes the key for `account`; succeeds when there was none.
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

/// The platform keyring.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyringStore;

fn store_error(e: &keyring::Error) -> StoreError {
    use keyring::Error as E;
    match e {
        E::NoStorageAccess(_) => StoreError::Locked,
        E::PlatformFailure(_) => StoreError::Other("platform_failure".to_owned()),
        E::NoDefaultStore => StoreError::Other("no_store".to_owned()),
        E::BadEncoding(_) | E::BadDataFormat(..) => StoreError::Other("bad_data".to_owned()),
        E::Ambiguous(_) => StoreError::Other("ambiguous".to_owned()),
        _ => StoreError::Other("other".to_owned()),
    }
}

fn entry(account: &str) -> Result<keyring::Entry, StoreError> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| store_error(&e))
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<ApiKey>, StoreError> {
        match entry(account)?.get_password() {
            Ok(key) => Ok(Some(ApiKey::new(&key)).filter(|k| !k.expose().is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn set(&self, account: &str, key: &ApiKey) -> Result<(), StoreError> {
        entry(account)?
            .set_password(key.expose())
            .map_err(|e| store_error(&e))
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(&e)),
        }
    }
}

/// Looks up an environment variable.
pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Resolves and caches the key of one provider. The keyring is read on first use (not at
/// startup), so a keyring that unlocks after login still works.
pub struct KeySource {
    provider: String,
    env_var: Option<String>,
    store: Arc<dyn SecretStore>,
    env: EnvLookup,
    cached: Mutex<Option<ApiKey>>,
}

impl std::fmt::Debug for KeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeySource")
            .field("provider", &self.provider)
            .field("env_var", &self.env_var)
            .finish_non_exhaustive()
    }
}

impl KeySource {
    /// Key for `provider`: env var `env_var` if set and non-empty, else the keyring entry named
    /// after the provider.
    #[must_use]
    pub fn new(provider: &str, env_var: Option<String>, store: Arc<dyn SecretStore>) -> Self {
        Self {
            provider: provider.to_owned(),
            env_var,
            store,
            env: Arc::new(|name| std::env::var(name).ok()),
            cached: Mutex::new(None),
        }
    }

    /// Replaces the environment lookup (for tests).
    #[must_use]
    pub fn with_env(mut self, env: EnvLookup) -> Self {
        self.env = env;
        self
    }

    /// The key, read once and then cached.
    pub async fn get(&self) -> Result<ApiKey, ProviderError> {
        let mut cached = self.cached.lock().await;
        if let Some(key) = cached.as_ref() {
            return Ok(key.clone());
        }
        let key = self.resolve().await?;
        *cached = Some(key.clone());
        Ok(key)
    }

    /// Drops the cached key, e.g. after the provider rejected it, so the next request reads
    /// the env var or keyring again.
    pub async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }

    async fn resolve(&self) -> Result<ApiKey, ProviderError> {
        if let Some(var) = &self.env_var
            && let Some(value) = (self.env)(var)
            && !value.trim().is_empty()
        {
            return Ok(ApiKey::new(&value));
        }
        let store = Arc::clone(&self.store);
        let account = self.provider.clone();
        let looked_up = tokio::task::spawn_blocking(move || store.get(&account))
            .await
            .map_err(|_| self.keyring_error("task_failed"))?;
        match looked_up {
            Ok(Some(key)) => Ok(key),
            Ok(None) => Err(ProviderError::KeyMissing {
                provider: self.provider.clone(),
            }),
            Err(StoreError::Locked) => Err(ProviderError::KeyringLocked {
                provider: self.provider.clone(),
            }),
            Err(StoreError::Other(kind)) => Err(self.keyring_error(&kind)),
        }
    }

    fn keyring_error(&self, kind: &str) -> ProviderError {
        tracing::warn!(provider = %self.provider, kind, "keyring lookup failed");
        ProviderError::Keyring {
            provider: self.provider.clone(),
            kind: kind.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Debug, Default)]
    struct MemoryStore {
        keys: StdMutex<HashMap<String, String>>,
        locked: bool,
        reads: AtomicUsize,
    }

    impl SecretStore for MemoryStore {
        fn get(&self, account: &str) -> Result<Option<ApiKey>, StoreError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.locked {
                return Err(StoreError::Locked);
            }
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(account)
                .map(|k| ApiKey::new(k)))
        }

        fn set(&self, account: &str, key: &ApiKey) -> Result<(), StoreError> {
            self.keys
                .lock()
                .unwrap()
                .insert(account.to_owned(), key.expose().to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            self.keys.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn no_env() -> EnvLookup {
        Arc::new(|_| None)
    }

    #[tokio::test]
    async fn env_var_wins_over_keyring() {
        let store = Arc::new(MemoryStore::default());
        store.set("openai", &ApiKey::new("from-keyring")).unwrap();
        let src = KeySource::new("openai", Some("HECE_TEST_KEY".into()), store).with_env(Arc::new(
            |name| (name == "HECE_TEST_KEY").then(|| "from-env\n".to_owned()),
        ));
        assert_eq!(src.get().await.unwrap().expose(), "from-env");
    }

    #[tokio::test]
    async fn empty_env_var_falls_back_to_keyring() {
        let store = Arc::new(MemoryStore::default());
        store.set("openai", &ApiKey::new("from-keyring")).unwrap();
        let src = KeySource::new("openai", Some("HECE_TEST_KEY".into()), store)
            .with_env(Arc::new(|_| Some("  ".to_owned())));
        assert_eq!(src.get().await.unwrap().expose(), "from-keyring");
    }

    #[tokio::test]
    async fn missing_key_names_the_provider() {
        let src = KeySource::new("groq", None, Arc::new(MemoryStore::default())).with_env(no_env());
        let e = src.get().await.unwrap_err();
        assert_eq!(e.code(), "provider_key_missing");
        assert!(e.to_string().contains("groq"));
    }

    #[tokio::test]
    async fn locked_keyring_is_a_clear_error() {
        let store = Arc::new(MemoryStore {
            locked: true,
            ..MemoryStore::default()
        });
        let src = KeySource::new("deepgram", None, store).with_env(no_env());
        let e = src.get().await.unwrap_err();
        assert_eq!(e.code(), "keyring_locked");
        assert!(e.to_string().contains("deepgram"));
    }

    #[tokio::test]
    async fn key_is_cached_until_invalidated() {
        let store = Arc::new(MemoryStore::default());
        store.set("openai", &ApiKey::new("k1")).unwrap();
        let src = KeySource::new("openai", None, store.clone()).with_env(no_env());
        src.get().await.unwrap();
        src.get().await.unwrap();
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
        store.set("openai", &ApiKey::new("k2")).unwrap();
        src.invalidate().await;
        assert_eq!(src.get().await.unwrap().expose(), "k2");
    }

    #[test]
    fn key_debug_is_redacted() {
        assert_eq!(
            format!("{:?}", ApiKey::new("sk-secret")),
            "ApiKey(<redacted>)"
        );
    }
}
