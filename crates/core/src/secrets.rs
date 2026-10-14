use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use uuid::Uuid;

use crate::error::{CoreError, Result};

const SERVICE: &str = "curlybrace";

/// Storage for connection secrets. Values must never be logged or put in errors.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    /// Idempotent: deleting a missing key succeeds.
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    SshPassword,
    SshPassphrase,
}

pub fn secret_key(id: Uuid, kind: SecretKind) -> String {
    let suffix = match kind {
        SecretKind::Password => "password",
        SecretKind::SshPassword => "ssh-password",
        SecretKind::SshPassphrase => "ssh-passphrase",
    };
    format!("conn:{id}:{suffix}")
}

/// In-memory store for tests and fallbacks.
#[derive(Default)]
pub struct MemorySecretStore {
    entries: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, String>>> {
        self.entries
            .lock()
            .map_err(|_| CoreError::Secret("memory store poisoned".into()))
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.lock()?.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.lock()?.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.lock()?.remove(key);
        Ok(())
    }
}

/// macOS Keychain-backed store (service `curlybrace`).
#[derive(Default)]
pub struct KeyringSecretStore;

impl KeyringSecretStore {
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(|e| CoreError::Secret(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Secret(e.to_string())),
        }
    }
}

impl SecretKind {
    pub const ALL: [SecretKind; 3] = [
        SecretKind::Password,
        SecretKind::SshPassword,
        SecretKind::SshPassphrase,
    ];
}

/// Copies every secret stored for `from` to `to`. All-or-nothing: if any step fails, the
/// secrets already written under `to` are removed (best effort) and the first error returns.
pub fn copy_all(store: &dyn SecretStore, from: Uuid, to: Uuid) -> Result<()> {
    let copy_one = |kind: SecretKind| -> Result<()> {
        if let Some(value) = store.get(&secret_key(from, kind))? {
            store.set(&secret_key(to, kind), &value)?;
        }
        Ok(())
    };
    for kind in SecretKind::ALL {
        if let Err(error) = copy_one(kind) {
            for written in SecretKind::ALL {
                let _ = store.delete(&secret_key(to, written));
            }
            return Err(error);
        }
    }
    Ok(())
}

/// A secret value that cannot be printed by accident.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip_and_delete() {
        let s = MemorySecretStore::new();
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", "p@ss:/%word").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("p@ss:/%word"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        s.delete("k").unwrap(); // idempotent
    }

    #[test]
    fn keys_are_namespaced_per_connection_and_kind() {
        let id = Uuid::new_v4();
        assert_ne!(
            secret_key(id, SecretKind::Password),
            secret_key(id, SecretKind::SshPassword)
        );
        assert_ne!(
            secret_key(id, SecretKind::Password),
            secret_key(Uuid::new_v4(), SecretKind::Password)
        );
        assert!(secret_key(id, SecretKind::Password).starts_with("conn:"));
    }

    #[test]
    #[ignore = "touches the real macOS Keychain"]
    fn keyring_round_trip_on_macos() {
        let s = KeyringSecretStore::new();
        let key = format!("test:{}", Uuid::new_v4());
        assert_eq!(s.get(&key).unwrap(), None);
        s.set(&key, "p@ss:/%word").unwrap();
        assert_eq!(s.get(&key).unwrap().as_deref(), Some("p@ss:/%word"));
        s.delete(&key).unwrap();
        assert_eq!(s.get(&key).unwrap(), None);
        s.delete(&key).unwrap();
    }

    /// Wraps a store and fails `set` for one key.
    struct FailingSet {
        inner: MemorySecretStore,
        fail_key: String,
    }
    impl SecretStore for FailingSet {
        fn get(&self, key: &str) -> Result<Option<String>> {
            self.inner.get(key)
        }
        fn set(&self, key: &str, value: &str) -> Result<()> {
            if key == self.fail_key {
                return Err(CoreError::Secret("boom".into()));
            }
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key)
        }
    }

    #[test]
    fn copy_all_copies_present_secrets_only() {
        let s = MemorySecretStore::new();
        let (from, to) = (Uuid::new_v4(), Uuid::new_v4());
        s.set(&secret_key(from, SecretKind::Password), "pw")
            .unwrap();
        copy_all(&s, from, to).unwrap();
        assert_eq!(
            s.get(&secret_key(to, SecretKind::Password))
                .unwrap()
                .as_deref(),
            Some("pw")
        );
        assert_eq!(
            s.get(&secret_key(to, SecretKind::SshPassword)).unwrap(),
            None
        );
        assert_eq!(
            s.get(&secret_key(from, SecretKind::Password))
                .unwrap()
                .as_deref(),
            Some("pw"),
            "the source keeps its secret"
        );
    }

    #[test]
    fn copy_all_rolls_back_when_a_later_copy_fails() {
        let (from, to) = (Uuid::new_v4(), Uuid::new_v4());
        let store = FailingSet {
            inner: MemorySecretStore::new(),
            fail_key: secret_key(to, SecretKind::SshPassword),
        };
        store
            .inner
            .set(&secret_key(from, SecretKind::Password), "pw")
            .unwrap();
        store
            .inner
            .set(&secret_key(from, SecretKind::SshPassword), "ssh")
            .unwrap();
        let err = copy_all(&store, from, to).unwrap_err();
        assert_eq!(err, CoreError::Secret("boom".into()));
        for kind in SecretKind::ALL {
            assert_eq!(
                store.get(&secret_key(to, kind)).unwrap(),
                None,
                "{kind:?} left behind"
            );
        }
    }

    #[test]
    fn secret_string_never_prints_its_value() {
        let s = SecretString::new("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose(), "hunter2");
    }
}
