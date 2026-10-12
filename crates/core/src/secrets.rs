use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use uuid::Uuid;

use crate::error::{CoreError, Result};

const SERVICE: &str = "mongostudio";

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

/// macOS Keychain-backed store (service `mongostudio`).
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
}
