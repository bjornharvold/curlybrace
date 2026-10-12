# Core Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A macOS egui desktop app that connects to MongoDB (SCRAM/X.509/LDAP/Kerberos best-effort, TLS, SSH tunnel), browses databases/collections, queries with filter/sort/projection/skip/limit, shows documents as tree/table/JSON with paging, and supports document CRUD.

**Architecture:** Cargo workspace. `mongostudio-core` (no UI deps) wraps the `mongodb` driver behind a `DocumentStore` trait, owns connection config, secrets, SSH tunnel and typed errors. `mongostudio-ui` holds egui views plus pure view-model functions. `mongostudio` (bin) wires a background Tokio runtime to the UI over `mpsc` channels.

**Tech Stack:** Rust (stable), `mongodb` + `bson` (driver), `tokio`, `async-trait`, `serde`/`serde_json`, `thiserror`, `uuid`, `keyring` (macOS Keychain), `russh` + `russh-keys` (SSH), `eframe`/`egui`/`egui_extras`, `tempfile` (tests), `testcontainers` (integration tests).

**Spec:** `docs/superpowers/specs/2026-10-12-core-browser-design.md`

> Dependency versions: run `cargo add <crate>` to take the latest compatible release when a task first needs it, then `cargo check`. Where an external API (russh, keyring, egui_extras) differs from the code shown, adapt to the installed version's docs (`cargo doc --open` or docs.rs) while keeping the **Interfaces** of the task unchanged.

## Global Constraints

- Platform: macOS only. UI toolkit: egui (pure Rust). No decompilation or porting of any proprietary binary.
- Workspace crates: `mongostudio-core`, `mongostudio-ui`, `mongostudio` (bin). `core` MUST NOT depend on egui/eframe.
- Page size: 50 documents per page.
- Secrets (passwords, SSH keys/passphrases) live only in the macOS Keychain; config on disk holds references only; secrets never appear in logs or errors.
- Every user-facing string resolves through an i18n key in the hand-edited nested `crates/ui/locales/en.json`; no literal English in UI code. Report new keys at the end of each UI task.
- Immutability: new values, never in-place mutation of shared state or documents.
- Every failure maps to a typed `CoreError`; nothing silently swallowed.
- Validate all user input (URI, JSON, edited documents, tunnel settings) at the boundary.
- TDD for every task (red, green, refactor). `core` coverage >= 80%. Prefer real `bson` objects from `mock_objects`; mock only the driver, SSH and keychain boundaries.
- Files < 800 lines (aim 200-400); functions < 50 lines; nesting <= 4.
- Commits: conventional format (`feat(scope): ...`, lowercase, imperative), each ending with
  `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Sub-agents default to the Sonnet model.

## Review Focus

1. **Non-ObjectId `_id`** (string, int, UUID/Binary, compound document): edit and delete must hit exactly the intended document. Pinned in Task 9.
2. **Exotic BSON types round-trip without type loss** (Int32 vs Int64, Decimal128, Binary, DateTime, NaN, MinKey/MaxKey) through the editor JSON. Pinned in Task 8.
3. **Bad query-bar input** (empty = `{}`, trailing comma, array instead of object, deeply nested): typed error, no panic, previous results stay visible. Pinned in Task 7 and Task 14.
4. **Failure paths** (unreachable host, wrong password, tunnel/server drop mid-session): bounded timeout, localized error, UI never hangs. Pinned in Task 4 and Task 10.
5. **Awkward names and secrets** (passwords containing `@ : / %`, db/collection names with dots, spaces or unicode): must not break connection building or sidebar/namespace handling. Pinned in Task 4 and Task 6.

---

## File Structure

```
Cargo.toml                                  workspace
crates/core/
  Cargo.toml                                feature `test-support`, feature `gssapi`
  src/lib.rs
  src/error.rs                              CoreError, Result, i18n_key()
  src/connection.rs                         ConnectionConfig + related types, validate()
  src/secrets.rs                            SecretStore trait, MemorySecretStore, KeyringSecretStore
  src/client.rs                             build_client_options()
  src/store.rs                              Namespace, Query, Page, DocumentStore trait
  src/mongo_store.rs                        MongoStore (driver impl)
  src/query.rs                              Query::parse
  src/ejson.rs                              editor/view JSON conversion
  src/crud.rs                               DocumentService
  src/tunnel.rs                             SSH tunnel + host key policy
  src/runtime.rs                            Command, Event, spawn_runtime()
  src/test_support.rs                       mock_objects + InMemoryStore (feature test-support)
  tests/                                    integration tests (containers)
crates/ui/
  Cargo.toml
  locales/en.json
  src/lib.rs
  src/i18n.rs
  src/viewmodel.rs                          table/tree/page pure functions
  src/app.rs                                MongoStudioApp (eframe::App)
  src/views/{mod,connections,sidebar,findbar,tree,table,json,editor}.rs
crates/app/
  Cargo.toml
  src/main.rs
docs/superpowers/notes/gssapi-spike.md      spike result
```

---

### Task 1: Workspace scaffold, CoreError, git

**Files:**
- Create: `Cargo.toml`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/core/src/error.rs`, `.gitignore`

**Interfaces:**
- Produces: `mongostudio_core::error::{CoreError, Result<T>}`; `CoreError::i18n_key(&self) -> &'static str`

- [ ] **Step 1: Init repo and workspace**

```bash
cd path/to/the-mongodb-gui
git init
printf '/target\n.DS_Store\n' > .gitignore
cargo new --lib crates/core --name mongostudio-core
```

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/core"]
```
(UI and app crates are added to `members` in Tasks 11 and 16.)

```bash
cd crates/core && cargo add thiserror serde --features serde/derive && cargo add serde_json bson uuid --features uuid/v4,uuid/serde
```

- [ ] **Step 2: Write the failing test** in `crates/core/src/error.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_distinct_i18n_key() {
        let errors = [
            CoreError::InvalidUri("x".into()),
            CoreError::InvalidJson("x".into()),
            CoreError::InvalidDocument("x".into()),
            CoreError::Secret("x".into()),
            CoreError::Ssh("x".into()),
            CoreError::Unsupported("x".into()),
            CoreError::Database("x".into()),
            CoreError::NotFound,
        ];
        let mut keys: Vec<_> = errors.iter().map(CoreError::i18n_key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), errors.len());
        assert!(keys.iter().all(|k| k.starts_with("errors.")));
    }
}
```

- [ ] **Step 3: Run, expect FAIL** — `cargo test -p mongostudio-core error` -> compile error: `CoreError` not found.

- [ ] **Step 4: Implement** (above the test module)

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CoreError {
    #[error("invalid connection uri: {0}")]
    InvalidUri(String),
    #[error("invalid json: {0}")]
    InvalidJson(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    #[error("secret store: {0}")]
    Secret(String),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("database: {0}")]
    Database(String),
    #[error("not found")]
    NotFound,
}

impl CoreError {
    /// Key into the UI's `en.json`. The detail text is logged, never shown raw.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::InvalidUri(_) => "errors.invalid_uri",
            Self::InvalidJson(_) => "errors.invalid_json",
            Self::InvalidDocument(_) => "errors.invalid_document",
            Self::Secret(_) => "errors.secret_store",
            Self::Ssh(_) => "errors.ssh",
            Self::Unsupported(_) => "errors.unsupported",
            Self::Database(_) => "errors.database",
            Self::NotFound => "errors.not_found",
        }
    }
}
```

`lib.rs`: `pub mod error;`

- [ ] **Step 5: Run, expect PASS** — `cargo test -p mongostudio-core error`

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "chore: scaffold workspace and typed core error

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Connection model and validation

**Files:**
- Create: `crates/core/src/connection.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod connection;`)

**Interfaces:**
- Consumes: `CoreError`, `Result`
- Produces:
```rust
pub enum AuthMechanism { None, ScramSha1, ScramSha256, X509, Gssapi, Plain }
pub struct TlsConfig { pub enabled: bool, pub ca_file: Option<PathBuf>, pub client_cert_key_file: Option<PathBuf>, pub allow_invalid_certs: bool }
pub enum SshAuth { Password, PrivateKey { path: PathBuf } }
pub struct SshConfig { pub host: String, pub port: u16, pub username: String, pub auth: SshAuth }
pub struct ConnectionConfig { pub id: Uuid, pub name: String, pub uri: String, pub auth: AuthMechanism, pub username: Option<String>, pub auth_source: Option<String>, pub tls: TlsConfig, pub ssh: Option<SshConfig> }
impl ConnectionConfig { pub fn new(name: &str, uri: &str) -> Self; pub fn validate(&self) -> Result<()> }
```
All types derive `Clone, Debug, PartialEq, Serialize, Deserialize`.

- [ ] **Step 1: Failing tests** (append `#[cfg(test)] mod tests` in `connection.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_and_srv_uris() {
        assert!(ConnectionConfig::new("a", "mongodb://localhost:27017").validate().is_ok());
        assert!(ConnectionConfig::new("a", "mongodb+srv://cluster0.example.com/db").validate().is_ok());
    }

    #[test]
    fn rejects_wrong_scheme_and_empty() {
        for bad in ["", "http://x", "localhost:27017"] {
            assert!(matches!(
                ConnectionConfig::new("a", bad).validate(),
                Err(CoreError::InvalidUri(_))
            ), "{bad}");
        }
    }

    #[test]
    fn rejects_embedded_password() {
        let err = ConnectionConfig::new("a", "mongodb://bob:secret@h/db").validate().unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn rejects_blank_name_and_bad_ssh_port() {
        assert!(ConnectionConfig::new("  ", "mongodb://h").validate().is_err());
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.ssh = Some(SshConfig { host: "b".into(), port: 0, username: "u".into(), auth: SshAuth::Password });
        assert!(c.validate().is_err());
    }

    #[test]
    fn x509_needs_client_cert_and_tls() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.auth = AuthMechanism::X509;
        assert!(c.validate().is_err());
        c.tls.enabled = true;
        c.tls.client_cert_key_file = Some("/tmp/c.pem".into());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn serde_round_trip_contains_no_password_field() {
        let c = ConnectionConfig::new("a", "mongodb://h");
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.to_lowercase().contains("password"));
        assert_eq!(serde_json::from_str::<ConnectionConfig>(&json).unwrap(), c);
    }
}
```

(`SshAuth::Password` is a unit variant, so "password" appears nowhere in a default config's JSON because `ssh` is `null`.)

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p mongostudio-core connection` (types not defined).

- [ ] **Step 3: Implement**

```rust
use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMechanism { None, ScramSha1, ScramSha256, X509, Gssapi, Plain }

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub ca_file: Option<PathBuf>,
    pub client_cert_key_file: Option<PathBuf>,
    pub allow_invalid_certs: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SshAuth { Password, PrivateKey { path: PathBuf } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SshConfig { pub host: String, pub port: u16, pub username: String, pub auth: SshAuth }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: Uuid,
    pub name: String,
    /// URI WITHOUT credentials; the password is stored in the Keychain.
    pub uri: String,
    pub auth: AuthMechanism,
    pub username: Option<String>,
    pub auth_source: Option<String>,
    pub tls: TlsConfig,
    pub ssh: Option<SshConfig>,
}

impl ConnectionConfig {
    pub fn new(name: &str, uri: &str) -> Self {
        Self {
            id: Uuid::new_v4(), name: name.into(), uri: uri.into(),
            auth: AuthMechanism::None, username: None, auth_source: None,
            tls: TlsConfig::default(), ssh: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(CoreError::InvalidUri("blank connection name".into()));
        }
        self.validate_uri()?;
        if self.auth == AuthMechanism::X509
            && !(self.tls.enabled && self.tls.client_cert_key_file.is_some())
        {
            return Err(CoreError::InvalidUri("x509 requires tls and a client certificate".into()));
        }
        if let Some(ssh) = &self.ssh {
            if ssh.host.trim().is_empty() || ssh.username.trim().is_empty() || ssh.port == 0 {
                return Err(CoreError::Ssh("host, username and a non-zero port are required".into()));
            }
        }
        Ok(())
    }

    fn validate_uri(&self) -> Result<()> {
        let rest = self.uri.strip_prefix("mongodb+srv://")
            .or_else(|| self.uri.strip_prefix("mongodb://"))
            .ok_or_else(|| CoreError::InvalidUri("scheme must be mongodb:// or mongodb+srv://".into()))?;
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        if authority.is_empty() {
            return Err(CoreError::InvalidUri("missing host".into()));
        }
        // userinfo is everything before the last '@' in the authority
        if let Some((userinfo, _)) = authority.rsplit_once('@') {
            if userinfo.contains(':') {
                return Err(CoreError::InvalidUri("password must not be embedded in the uri".into()));
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(core): add connection config model and validation`

---

### Task 3: Secret store (Keychain)

**Files:**
- Create: `crates/core/src/secrets.rs`; Modify: `lib.rs`

**Interfaces:**
- Produces:
```rust
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}
pub struct MemorySecretStore;           // Default + new(); tests and fallbacks
pub struct KeyringSecretStore;          // service "mongostudio"
pub fn secret_key(id: Uuid, kind: SecretKind) -> String   // "conn:<id>:password" | ":ssh-password" | ":ssh-passphrase"
pub enum SecretKind { Password, SshPassword, SshPassphrase }
```

- [ ] **Step 1: Failing tests**

```rust
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
        assert_ne!(secret_key(id, SecretKind::Password), secret_key(id, SecretKind::SshPassword));
        assert_ne!(secret_key(id, SecretKind::Password), secret_key(Uuid::new_v4(), SecretKind::Password));
        assert!(secret_key(id, SecretKind::Password).starts_with("conn:"));
    }
}
```

- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** `MemorySecretStore` wraps `Mutex<HashMap<String,String>>`. `KeyringSecretStore` uses `keyring::Entry::new("mongostudio", key)`; map `keyring::Error::NoEntry` to `Ok(None)` / idempotent delete, every other error to `CoreError::Secret(e.to_string())` (never include the value). `cargo add keyring --features apple-native`.

```rust
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
```

- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Add an `#[ignore]` test** `keyring_round_trip_on_macos` that does set/get/delete against the real Keychain with a random key; run once manually: `cargo test -p mongostudio-core keyring -- --ignored`.
- [ ] **Step 6: Commit** `feat(core): add secret store backed by macos keychain`

---

### Task 4: Driver options builder (auth, TLS, timeouts)

**Files:**
- Create: `crates/core/src/client.rs`; Modify: `lib.rs`, `Cargo.toml` (`cargo add mongodb tokio --features tokio/full`)

**Interfaces:**
- Consumes: `ConnectionConfig`, `AuthMechanism`, `CoreError`
- Produces:
```rust
pub const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(8);
pub async fn build_client_options(cfg: &ConnectionConfig, password: Option<&str>, tunnel: Option<SocketAddr>) -> Result<mongodb::options::ClientOptions>;
pub async fn connect(cfg: &ConnectionConfig, password: Option<&str>, tunnel: Option<SocketAddr>) -> Result<mongodb::Client>;  // builds options, creates Client, runs `ping`
```
When `tunnel` is `Some`, the host list is replaced by that address and `direct_connection = true`.

- [ ] **Step 1: Failing tests** (`#[tokio::test]`)

```rust
#[tokio::test]
async fn password_with_special_chars_is_set_on_credential_not_in_uri() {
    let mut c = ConnectionConfig::new("a", "mongodb://localhost:27017/admin");
    c.auth = AuthMechanism::ScramSha256;
    c.username = Some("bob".into());
    let o = build_client_options(&c, Some("p@ss:/%w ord"), None).await.unwrap();
    let cred = o.credential.unwrap();
    assert_eq!(cred.username.as_deref(), Some("bob"));
    assert_eq!(cred.password.as_deref(), Some("p@ss:/%w ord"));
    assert_eq!(cred.mechanism, Some(mongodb::options::AuthMechanism::ScramSha256));
}

#[tokio::test]
async fn timeout_is_bounded() {
    let o = build_client_options(&ConnectionConfig::new("a", "mongodb://localhost"), None, None).await.unwrap();
    assert_eq!(o.server_selection_timeout, Some(SERVER_SELECTION_TIMEOUT));
}

#[tokio::test]
async fn tunnel_overrides_hosts_and_forces_direct() {
    let addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let o = build_client_options(&ConnectionConfig::new("a", "mongodb://db.internal:27017"), None, Some(addr)).await.unwrap();
    assert_eq!(o.hosts.len(), 1);
    assert_eq!(o.hosts[0].to_string(), "127.0.0.1:50000");
    assert_eq!(o.direct_connection, Some(true));
}

#[tokio::test]
async fn x509_and_plain_map_to_driver_mechanisms() {
    let mut x = ConnectionConfig::new("a", "mongodb://h");
    x.auth = AuthMechanism::X509;
    x.tls.enabled = true;
    x.tls.client_cert_key_file = Some("/tmp/c.pem".into());
    let cred = build_client_options(&x, None, None).await.unwrap().credential.unwrap();
    assert_eq!(cred.mechanism, Some(mongodb::options::AuthMechanism::MongoDbX509));
    assert_eq!(cred.source.as_deref(), Some("$external"));

    let mut p = ConnectionConfig::new("a", "mongodb://h");
    p.auth = AuthMechanism::Plain;
    p.username = Some("ldapuser".into());
    let cred = build_client_options(&p, Some("pw"), None).await.unwrap().credential.unwrap();
    assert_eq!(cred.mechanism, Some(mongodb::options::AuthMechanism::Plain));
    assert_eq!(cred.source.as_deref(), Some("$external"));

    let none = ConnectionConfig::new("a", "mongodb://h");
    assert!(build_client_options(&none, None, None).await.unwrap().credential.is_none());
}

#[tokio::test]
async fn gssapi_without_feature_is_unsupported() {
    // under #[cfg(not(feature = "gssapi"))]
    let mut c = ConnectionConfig::new("a", "mongodb://h");
    c.auth = AuthMechanism::Gssapi;
    assert!(matches!(build_client_options(&c, None, None).await, Err(CoreError::Unsupported(_))));
}

#[tokio::test]
async fn unreachable_host_fails_within_bound() {
    let c = ConnectionConfig::new("a", "mongodb://127.0.0.1:1"); // closed port
    let start = std::time::Instant::now();
    let err = connect(&c, None, None).await.unwrap_err();
    assert!(matches!(err, CoreError::Database(_)));
    assert!(start.elapsed() < SERVER_SELECTION_TIMEOUT + Duration::from_secs(3));
}
```
Write the `x509_and_plain...` body fully (three assertions) rather than leaving it as a comment.

- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** Steps: `cfg.validate()?`; `ClientOptions::parse(&cfg.uri).await` mapped to `InvalidUri`; set `server_selection_timeout` and `connect_timeout`; build `Credential` from `cfg.auth` (None -> leave `credential` as parsed); set `tls` via `Tls::Enabled(TlsOptions { ca_file_path, cert_key_file_path, allow_invalid_certificates })` when `cfg.tls.enabled`; if `tunnel` is set, `options.hosts = vec![ServerAddress::Tcp { host: ip.to_string(), port: Some(port) }]` and `direct_connection = Some(true)`; for `Gssapi`, `#[cfg(feature = "gssapi")]` sets `AuthMechanism::Gssapi`, otherwise return `Unsupported`. `connect` maps driver errors to `CoreError::Database(e.to_string())` after `client.database("admin").run_command(doc!{"ping":1})`.
- [ ] **Step 4: Run, expect PASS** (the unreachable-host test needs no server).
- [ ] **Step 5: Commit** `feat(core): build driver client options for auth, tls and tunnel`

---

### Task 5: GSSAPI feasibility spike

**Files:**
- Create: `docs/superpowers/notes/gssapi-spike.md`; Modify: `crates/core/Cargo.toml` (feature `gssapi`)

**Interfaces:** Produces the `gssapi` cargo feature (forwarding to the driver's GSSAPI feature if one exists) and a written verdict. Throwaway code is labeled as such.

- [ ] **Step 1:** Check the installed driver version's feature list (`cargo metadata --format-version 1 | jq` on the `mongodb` package, or docs.rs "Feature flags") for a GSSAPI/Kerberos feature.
- [ ] **Step 2:** If present, add `gssapi = ["mongodb/<feature-name>"]` to `[features]`, run `cargo check -p mongostudio-core --features gssapi`, and confirm `AuthMechanism::Gssapi` compiles in Task 4's builder. If a KDC is available (e.g. a krb5 test container), attempt a real auth; otherwise note it untested.
- [ ] **Step 3:** Write `gssapi-spike.md` with: driver version, feature name (or "none"), compile result, runtime result, system library requirement (macOS ships GSS.framework), and a verdict: **supported** / **supported-untested** / **unsupported**.
- [ ] **Step 4:** If unsupported, leave the `gssapi` feature absent; Task 4's `Unsupported` error already covers it, and Task 12's connection form must hide the Kerberos option via `cfg!(feature = "gssapi")` exposed as `mongostudio_core::GSSAPI_AVAILABLE: bool` (add this const in `lib.rs` now).
- [ ] **Step 5: Commit** `docs: record gssapi spike verdict`

---

### Task 6: Store trait, Namespace, Paging types, test support

**Files:**
- Create: `crates/core/src/store.rs`, `crates/core/src/test_support.rs`; Modify: `lib.rs`, `Cargo.toml` (`[features] test-support = []`, `cargo add async-trait futures`)

**Interfaces:**
- Produces:
```rust
pub const PAGE_SIZE: u64 = 50;
pub struct Namespace { pub db: String, pub collection: String }       // Clone, Debug, PartialEq, Eq, Hash
pub struct Query { pub filter: Document, pub sort: Option<Document>, pub projection: Option<Document>, pub skip: u64, pub limit: u64 }
impl Default for Query  // empty filter, skip 0, limit PAGE_SIZE
pub struct Page { pub docs: Vec<Document>, pub skip: u64, pub has_more: bool }
#[async_trait] pub trait DocumentStore: Send + Sync {
    async fn list_databases(&self) -> Result<Vec<String>>;
    async fn list_collections(&self, db: &str) -> Result<Vec<String>>;
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page>;
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson>;
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()>;   // NotFound if no match
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()>;                   // NotFound if no match
}
// test_support (feature = "test-support" or cfg(test))
pub mod mock_objects { pub fn mock_doc() -> Document; pub fn mock_doc_with_id(id: impl Into<Bson>) -> Document; pub fn mock_namespace() -> Namespace; pub fn exotic_doc() -> Document; pub fn mock_connection() -> ConnectionConfig; }
pub struct InMemoryStore;   // new(), with_docs(ns, Vec<Document>), implements DocumentStore faithfully (filter by equality on top-level fields, sort, skip, limit, projection inclusion)
```
`exotic_doc()` contains: Int32, Int64, Double, NaN, Decimal128, Binary, DateTime, MinKey, MaxKey, nested doc, array, regex, ObjectId, Timestamp.

- [ ] **Step 1: Failing tests** for `InMemoryStore`:
  - `find` returns at most `limit` docs, `has_more == true` when more remain (fetch `limit + 1` internally).
  - `skip` offsets; `sort` `{n: -1}` orders descending; `projection` `{a: 1}` keeps `a` and `_id`.
  - `insert` returns the `_id` (generating an ObjectId if absent); `replace`/`delete` return `NotFound` for an unknown id.
  - namespaces with dots/spaces/unicode (`"my db"`, `"col.with.dots"`, `"коллекция"`) round-trip through `list_collections`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** the types, trait and `InMemoryStore` (a `Mutex<BTreeMap<(String,String), Vec<Document>>>`; every mutation clones and replaces the Vec).
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(core): add document store trait, paging types and test fixtures`

---

### Task 7: Query parsing from user text

**Files:**
- Create: `crates/core/src/query.rs`; Modify: `lib.rs`

**Interfaces:**
- Consumes: `Query`, `PAGE_SIZE`, `CoreError`
- Produces:
```rust
pub struct QueryText { pub filter: String, pub sort: String, pub projection: String, pub skip: String, pub limit: String }
impl Default for QueryText  // all empty
pub fn parse_query(t: &QueryText) -> Result<Query>
pub fn next_page(q: &Query) -> Query      // skip += limit
pub fn prev_page(q: &Query) -> Query      // skip saturating_sub(limit)
```
Empty text means: filter `{}`, sort/projection `None`, skip `0`, limit `PAGE_SIZE`. Limit is clamped to 1..=1000.

- [ ] **Step 1: Failing tests**
  - `""` -> default query; `{"a": 1}` -> filter doc; `{"_id": {"$oid": "507f1f77bcf86cd799439011"}}` -> typed ObjectId; `{"d": {"$date": "2020-01-01T00:00:00Z"}}` -> DateTime.
  - Errors (all `CoreError::InvalidJson`, none panic): `{"a":1,}`, `[1,2]`, `"str"`, `{`, `{"a": }`, nested depth 10_000 (`"[".repeat(10_000)`), skip `-1`, skip `abc`, limit `0` clamps to 1, limit `5000` clamps to 1000.
  - `next_page`/`prev_page` never underflow.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** Parse with `serde_json::from_str::<serde_json::Value>`; guard recursion depth by pre-checking max bracket nesting (reject > 128 before parsing, since `serde_json` itself has a 128 recursion limit and returns an error, never a panic); convert with `bson::Bson::try_from(value)`; require `Bson::Document`.

```rust
fn parse_doc(label: &str, text: &str) -> Result<Option<Document>> {
    if text.trim().is_empty() { return Ok(None); }
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| CoreError::InvalidJson(format!("{label}: {e}")))?;
    match Bson::try_from(value).map_err(|e| CoreError::InvalidJson(format!("{label}: {e}")))? {
        Bson::Document(d) => Ok(Some(d)),
        _ => Err(CoreError::InvalidJson(format!("{label}: expected an object"))),
    }
}
```
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(core): parse filter, sort and projection text into queries`

---

### Task 8: Extended-JSON conversion for view and editor

**Files:**
- Create: `crates/core/src/ejson.rs`; Modify: `lib.rs`

**Interfaces:**
- Produces:
```rust
pub fn doc_to_view_json(doc: &Document) -> String      // relaxed extjson, pretty
pub fn doc_to_editor_json(doc: &Document) -> String    // canonical extjson, pretty (preserves types)
pub fn editor_json_to_doc(text: &str) -> Result<Document>
pub fn bson_type_name(b: &Bson) -> &'static str        // "string","int32","int64","double","decimal128","objectId","date","binary","bool","null","object","array","regex","timestamp","minKey","maxKey","other"
```

- [ ] **Step 1: Failing tests**
  - `editor_json_to_doc(&doc_to_editor_json(&exotic_doc()))` equals `exotic_doc()` for every field, including `Int32(1)` staying `Int32` and `Int64(1)` staying `Int64`, `Decimal128`, `Binary` (generic and UUID subtype), `DateTime::MAX`/`MIN`, `MinKey`/`MaxKey`. NaN: compare via `is_nan()`, since `NaN != NaN`.
  - `doc_to_view_json` of an `Int64` is human-readable (`"n": 5`), not `{"$numberLong": ...}`.
  - `editor_json_to_doc("[]")`, `""`, `{"a":}` -> `InvalidJson`/`InvalidDocument`, no panic.
  - `bson_type_name` for each variant of `exotic_doc()`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** with `Bson::Document(doc.clone()).into_canonical_extjson()` / `into_relaxed_extjson()` -> `serde_json::to_string_pretty`; reverse via `Bson::try_from(serde_json::Value)`.
- [ ] **Step 4: Run, expect PASS.** If any exotic type fails to round-trip, fix the conversion; do not weaken the test.
- [ ] **Step 5: Commit** `feat(core): convert documents to and from extended json`

---

### Task 9: CRUD service

**Files:**
- Create: `crates/core/src/crud.rs`; Modify: `lib.rs`

**Interfaces:**
- Consumes: `DocumentStore`, `ejson`, `Namespace`
- Produces:
```rust
pub struct DocumentService<S: DocumentStore> { /* store: Arc<S> */ }
impl<S: DocumentStore> DocumentService<S> {
    pub fn new(store: Arc<S>) -> Self;
    pub async fn insert_from_json(&self, ns: &Namespace, text: &str) -> Result<Bson>;
    pub async fn replace_from_json(&self, ns: &Namespace, original_id: &Bson, text: &str) -> Result<()>;
    pub async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()>;
    pub async fn clone_document(&self, ns: &Namespace, source: &Document) -> Result<Bson>;  // strips _id, inserts
}
pub fn id_of(doc: &Document) -> Result<&Bson>;   // InvalidDocument if no _id
```
`replace_from_json` rejects (`InvalidDocument`) an edit that changes `_id` (MongoDB `_id` is immutable) and rejects text with no `_id`.

- [ ] **Step 1: Failing tests** using `InMemoryStore` with real docs, one per `_id` kind: ObjectId, String, Int32, Int64, UUID `Binary`, compound `{a:1,b:"x"}`. For each kind: seed two docs; `replace_from_json` on the first changes only the first; `delete` removes only the first. Plus:
  - editing `_id` -> `InvalidDocument`, store unchanged.
  - `clone_document` produces a doc equal to the source except a new `_id`; the original is untouched.
  - `insert_from_json` of invalid JSON -> `InvalidJson`, nothing inserted.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** Compare `_id` by `Bson` equality after parsing, since typed extjson preserves the kind.
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(core): add document crud service with id handling`

---

### Task 10: MongoStore (real driver) and runtime bridge

**Files:**
- Create: `crates/core/src/mongo_store.rs`, `crates/core/src/runtime.rs`; Modify: `lib.rs`

**Interfaces:**
- Consumes: `connect`, `DocumentStore`, `DocumentService`, `SecretStore`, tunnel (Task 13 later wires in; here `tunnel: Option<SocketAddr>` is `None`)
- Produces:
```rust
pub struct MongoStore { client: mongodb::Client }   // impl DocumentStore; MongoStore::new(client)
pub enum Command {
    Connect(ConnectionConfig), Disconnect,
    ListDatabases, ListCollections { db: String },
    Find { ns: Namespace, query: Query },
    Insert { ns: Namespace, json: String },
    Replace { ns: Namespace, id: Bson, json: String },
    Delete { ns: Namespace, id: Bson },
    Clone { ns: Namespace, source: Document },
}
pub enum Event {
    Connected, Disconnected,
    Databases(Vec<String>), Collections { db: String, names: Vec<String> },
    Results { ns: Namespace, query: Query, page: Page },
    Mutated { ns: Namespace },                  // UI re-runs the current query
    Failed { context: &'static str, error: CoreError },  // context is an i18n key suffix e.g. "connect","find"
}
pub struct RuntimeHandle { pub tx: UnboundedSender<Command>, pub rx: UnboundedReceiver<Event> }
pub fn spawn_runtime(secrets: Arc<dyn SecretStore>, repaint: Arc<dyn Fn() + Send + Sync>) -> RuntimeHandle;
pub fn spawn_runtime_with_store(store: Arc<dyn DocumentStore>, repaint: ...) -> RuntimeHandle;  // tests / pre-connected
```
`find` in `MongoStore` fetches `limit + 1`, sets `has_more`, applies sort/projection/skip, and maps all driver errors to `CoreError::Database`.

- [ ] **Step 1: Failing tests** with `InMemoryStore` through `spawn_runtime_with_store` (`#[tokio::test]`): `ListDatabases` -> `Event::Databases`; `Find` -> `Results` carrying the same `query`; `Insert` -> `Mutated`; invalid insert JSON -> `Event::Failed { error: InvalidJson }` and the runtime keeps serving later commands; a `FailingStore` (a test double returning `Database("down")`) yields `Failed` for every command with no hang, and the repaint callback fires once per event.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** One Tokio task loops on `rx.recv()`; each command is handled in a `tokio::spawn` so a slow `find` does not block others; every handler ends with exactly one event send then `repaint()`. Wrap every driver call in `tokio::time::timeout(Duration::from_secs(30), ..)` mapping elapsed to `Database("timeout")`.
- [ ] **Step 4: Run, expect PASS.** Then add an `#[ignore]` integration test hitting a local `mongod` (see Task 17) and run it manually.
- [ ] **Step 5: Commit** `feat(core): add mongo store and async runtime bridge`

---

### Task 11: UI crate, i18n layer, locale file

**Files:**
- Create: `crates/ui/Cargo.toml`, `crates/ui/src/lib.rs`, `crates/ui/src/i18n.rs`, `crates/ui/locales/en.json`; Modify: workspace `members`

**Interfaces:**
- Produces:
```rust
pub struct I18n { /* flat map "a.b.c" -> String */ }
impl I18n {
    pub fn from_json(src: &str) -> Result<Self, String>;   // flattens nested objects; rejects non-string leaves
    pub fn english() -> Self;                              // include_str!("../locales/en.json")
    pub fn t(&self, key: &str) -> String;                  // missing key -> "⟦key⟧" so gaps are visible
    pub fn t_with(&self, key: &str, params: &[(&str, &str)]) -> String;   // replaces {name}
    pub fn error(&self, e: &CoreError) -> String;          // t(e.i18n_key())
}
```

- [ ] **Step 1: Failing tests:** nested flatten (`{"a":{"b":"x"}}` -> `t("a.b") == "x"`); `t_with("hello {name}", [("name","Ann")])`; missing key returns `⟦k⟧`; **a test that `I18n::english()` has a translation for every `CoreError::i18n_key()`** (build one error per variant); invalid JSON -> `Err`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** `I18n` using `serde_json::Value` recursion. Start `en.json` with `errors.*` (the 8 keys from Task 1), `app.title`, `common.{ok,cancel,save,delete,close,retry}`.
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(ui): add i18n layer and english locale`

---

### Task 12: View-model functions and connection manager view

**Files:**
- Create: `crates/ui/src/viewmodel.rs`, `crates/ui/src/views/mod.rs`, `crates/ui/src/views/connections.rs`; Modify: `lib.rs`, `en.json`

**Interfaces:**
- Consumes: `ConnectionConfig`, `AuthMechanism`, `GSSAPI_AVAILABLE`, `SecretStore`
- Produces (pure, unit-tested without egui):
```rust
pub fn table_columns(docs: &[Document]) -> Vec<String>;        // union of keys, first-seen order, "_id" first
pub fn cell_text(b: &Bson) -> String;                           // scalars as text; docs/arrays as "{n fields}" / "[n items]"
pub struct ConnectionForm { pub name: String, pub uri: String, pub auth: AuthMechanism, pub username: String, pub password: String, pub tls_enabled: bool, pub tls_ca: String, pub tls_client_cert: String, pub ssh_enabled: bool, pub ssh_host: String, pub ssh_port: String, pub ssh_user: String, pub ssh_key_path: String, pub ssh_secret: String }
impl ConnectionForm { pub fn to_config(&self) -> Result<ConnectionConfig, CoreError>; pub fn from_config(c: &ConnectionConfig) -> Self; }
pub fn auth_options() -> Vec<AuthMechanism>;                    // omits Gssapi when !GSSAPI_AVAILABLE
pub fn save_connection(cfg: &ConnectionConfig, form: &ConnectionForm, secrets: &dyn SecretStore) -> Result<()>;  // writes secrets to Keychain, never to cfg
```
- Produces view fn: `pub fn show_connections(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) -> Option<ConnectionAction>` with `enum ConnectionAction { Connect(Uuid), Save(ConnectionForm), Delete(Uuid) }`.

- [ ] **Step 1: Failing tests** for `table_columns` (heterogeneous docs, `_id` first), `cell_text` (every `exotic_doc` field produces non-empty text, nested sizes), `ConnectionForm::to_config` (bad port `"abc"` -> `Err`, blank name -> `Err`, ssh disabled -> `ssh == None`), `save_connection` (password only in the `MemorySecretStore`, absent from `serde_json::to_string(&cfg)`), `auth_options` excludes Gssapi when unavailable.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** pure functions first, then the egui form (labels via `i18n.t("connections.*")`, validation error shown via `i18n.error`). Add keys: `connections.{title,new,name,uri,auth,username,password,tls,tls_ca,tls_client_cert,ssh,ssh_host,ssh_port,ssh_user,ssh_key,ssh_secret,connect,test}`, `auth.{none,scram_sha1,scram_sha256,x509,gssapi,plain}`.
- [ ] **Step 4: Run, expect PASS; `cargo check -p mongostudio-ui`.**
- [ ] **Step 5: Commit** `feat(ui): add connection manager and view-model helpers`

---

### Task 13: SSH tunnel

**Files:**
- Create: `crates/core/src/tunnel.rs`; Modify: `lib.rs`, `runtime.rs` (use tunnel in `Connect`)

**Interfaces:**
- Consumes: `SshConfig`, `SecretStore`, `secret_key`
- Produces:
```rust
pub enum HostKeyDecision { Trust, TrustAndRemember, Reject }
pub trait HostKeyPrompt: Send + Sync { fn decide(&self, host: &str, fingerprint: &str) -> HostKeyDecision; }
pub struct Tunnel { pub local_addr: SocketAddr, /* shutdown handle; Drop closes it */ }
pub async fn open_tunnel(ssh: &SshConfig, target_host: &str, target_port: u16, secret: Option<&str>, known_hosts: &Path, prompt: &dyn HostKeyPrompt) -> Result<Tunnel>;
pub fn check_host_key(known_hosts: &Path, host: &str, port: u16, key: &PublicKey, prompt: &dyn HostKeyPrompt) -> Result<()>;  // pure policy, unit-tested
```
Binds `127.0.0.1:0`; each accepted connection opens a `direct-tcpip` channel to the target and copies bytes both ways with `tokio::io::copy_bidirectional`.

- [ ] **Step 1: Failing tests** for `check_host_key` with a tempfile known_hosts and a fake prompt: known + matching -> Ok without prompting; unknown + `Trust` -> Ok, file unchanged; unknown + `TrustAndRemember` -> Ok and file now contains the host; unknown + `Reject` -> `Err(Ssh)`; **known but key changed -> `Err(Ssh)` without prompting** (never offer to trust a changed key). Also: `open_tunnel` to a closed port -> `Err(Ssh)` within 10 s.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** with `russh` (client handler's `check_server_key` delegates to `check_host_key`), `russh-keys` for key loading (`load_secret_key(path, passphrase)`) and `known_hosts` helpers. Wrap connect/auth in a 10 s timeout. Errors never include the secret.
- [ ] **Step 4: Run, expect PASS** for unit tests. Add an `#[ignore]` integration test against an `openssh-server` container (Task 17).
- [ ] **Step 5: Wire into `Connect`:** when `cfg.ssh` is `Some`, read the secret from the store (`SshPassword` or `SshPassphrase`), derive the target from the parsed URI host, open the tunnel, pass `Some(tunnel.local_addr)` to `connect`, and keep the `Tunnel` alive in the runtime until `Disconnect`. If the tunnel dies mid-session, the next failed command surfaces as `Event::Failed`.
- [ ] **Step 6: Commit** `feat(core): add ssh tunnel with host key verification`

---

### Task 14: Sidebar and find bar views

**Files:**
- Create: `crates/ui/src/views/sidebar.rs`, `crates/ui/src/views/findbar.rs`; Modify: `viewmodel.rs`, `en.json`

**Interfaces:**
- Produces:
```rust
// viewmodel.rs (pure)
pub struct SidebarState { pub databases: Vec<DbNode>, pub selected: Option<Namespace> }
pub struct DbNode { pub name: String, pub expanded: bool, pub collections: Option<Vec<String>> }   // None = not loaded yet
impl SidebarState {
    pub fn with_databases(self, names: Vec<String>) -> Self;
    pub fn with_collections(self, db: &str, names: Vec<String>) -> Self;
    pub fn toggled(self, db: &str) -> Self;
    pub fn selecting(self, ns: Namespace) -> Self;
}
pub struct FindBarState { pub text: QueryText, pub last_error: Option<CoreError> }
pub fn submit_find(state: &FindBarState) -> Result<Query, FindBarState>;   // Err returns new state with last_error set
// views
pub enum SidebarAction { LoadCollections(String), Open(Namespace), Refresh }
pub fn show_sidebar(ui, &SidebarState, &I18n) -> Option<SidebarAction>;
pub enum FindAction { Run(Query), Prev, Next, Clear }
pub fn show_findbar(ui, &mut FindBarState, has_prev: bool, has_next: bool, &I18n) -> Option<FindAction>;
```

- [ ] **Step 1: Failing tests** (pure): `with_collections` on an unknown db is a no-op; `toggled` twice returns the original; databases and collections with dots, spaces, unicode (`"my db"`, `"a.b.c"`, `"коллекция"`) are kept verbatim and selectable; `submit_find` with bad JSON returns a state with `last_error` set and **leaves `text` unchanged**; good JSON clears `last_error`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** pure state transitions (each returns a new value), then the egui widgets. Keys: `sidebar.{title,refresh,empty}`, `findbar.{filter,sort,projection,skip,limit,run,clear,prev,next}`.
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(ui): add sidebar and find bar`

---

### Task 15: Tree, table and JSON document views, editor modal

**Files:**
- Create: `crates/ui/src/views/{tree,table,json,editor}.rs`; Modify: `viewmodel.rs`, `en.json`; `cargo add egui_extras` in the UI crate

**Interfaces:**
- Consumes: `Page`, `ejson`, `bson_type_name`, `id_of`
- Produces:
```rust
pub enum ViewMode { Tree, Table, Json }
pub struct TreeRow { pub depth: usize, pub key: String, pub type_name: &'static str, pub value: String, pub path: Vec<String> }
pub fn flatten_doc(doc: &Document, expanded: &HashSet<Vec<String>>) -> Vec<TreeRow>;   // pure; collapsed docs/arrays show no children
pub enum DocAction { Edit(Document), Clone(Document), Delete(Bson), Insert }
pub fn show_documents(ui, mode: ViewMode, page: &Page, state: &mut TreeState, &I18n) -> Option<DocAction>;
pub struct EditorState { pub original_id: Option<Bson>, pub text: String, pub error: Option<CoreError> }
impl EditorState { pub fn for_edit(doc: &Document) -> Self; pub fn for_new() -> Self; }
pub enum EditorAction { Save { original_id: Option<Bson>, text: String }, Cancel }
pub fn show_editor(ctx, &mut EditorState, &I18n) -> Option<EditorAction>;
```

- [ ] **Step 1: Failing tests** (pure): `flatten_doc` on `exotic_doc()` yields a row per top-level field with the right `type_name`; a nested doc/array is hidden until its path is in `expanded`, then its children appear at `depth + 1`; 10_000-element array renders only up to a cap of 200 rows plus a summary row (add `tree.more_items` key) so the UI stays responsive; `EditorState::for_edit` text parses back to the same document (`editor_json_to_doc`); `for_new` text is `{}`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** `flatten_doc`, then views: tree via `egui::CollapsingHeader`-free manual rows (using `TreeRow`), table via `egui_extras::TableBuilder` with columns from `table_columns`, JSON via a read-only monospace `TextEdit` of `doc_to_view_json`. Row context menu (`doc.{edit,clone,delete,insert}`) emits `DocAction`. Delete asks confirmation (`doc.confirm_delete`). Keys: `view.{tree,table,json}`, `doc.*`, `editor.{title_edit,title_new,save,cancel}`.
- [ ] **Step 4: Run, expect PASS.**
- [ ] **Step 5: Commit** `feat(ui): add tree, table and json views with document editor`

---

### Task 16: App wiring and binary

**Files:**
- Create: `crates/ui/src/app.rs`, `crates/app/Cargo.toml`, `crates/app/src/main.rs`; Modify: workspace `members`, `ui/src/lib.rs`

**Interfaces:**
- Consumes: everything above
- Produces: `pub struct MongoStudioApp` implementing `eframe::App`; `MongoStudioApp::new(handle: RuntimeHandle, i18n: I18n, connections: Vec<ConnectionConfig>, secrets: Arc<dyn SecretStore>) -> Self`. Pure reducer: `pub fn apply_event(state: AppState, ev: Event) -> AppState` (all event handling lives here so it is testable without egui).

- [ ] **Step 1: Failing tests** on `apply_event`: `Databases` fills the sidebar; `Results` replaces the page and the stored `query`; **`Failed { context: "find" }` keeps the previous page and sets a localized banner key** (Review Focus 3); `Mutated` produces a "re-run current query" flag; `Disconnected` clears sidebar and page; events for a stale namespace (user switched collection) are ignored.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** `AppState` (immutable; reducer returns new state), `eframe::App::update`: drain `handle.rx` with `try_recv`, reduce, then draw panels (connections window or `SidePanel` sidebar + central find bar + document views + editor modal) and send `Command`s for returned actions. Persist the connection list (no secrets) as JSON in `~/Library/Application Support/mongostudio/connections.json`; validate on load, report unreadable files via a banner instead of ignoring them. `main.rs`: build a multi-thread Tokio runtime, `spawn_runtime(KeyringSecretStore, repaint)` where `repaint` calls `ctx.request_repaint()`, run `eframe::run_native`.
- [ ] **Step 4: Run, expect PASS; `cargo build --release`.**
- [ ] **Step 5: Manual smoke test** with a local `mongod` (`docker run -p 27017:27017 mongo:7`): connect, expand, query, view in all three modes, insert, edit, clone, delete; confirm each step. Use the `run` skill if available.
- [ ] **Step 6: Commit** `feat(app): wire runtime, state reducer and eframe shell`

---

### Task 17: Integration tests and coverage gate

**Files:**
- Create: `crates/core/tests/mongo_integration.rs`, `crates/core/tests/ssh_integration.rs`; Modify: `crates/core/Cargo.toml` (`[dev-dependencies] testcontainers`)

- [ ] **Step 1: Write integration tests** (marked `#[ignore]` unless `MONGOSTUDIO_IT=1`, so default `cargo test` stays fast): against `mongo:7` plain: connect, list, insert, paged `find` over 120 docs (3 pages, `has_more` correct), replace, delete, each `_id` kind from Task 9; against `mongo:7 --auth` with a SCRAM user and a password containing `@:/%`; wrong password -> `Failed`, within the timeout bound; SSH tunnel to `mongo` via an `openssh-server` container (password auth), including host key unknown/changed behavior; stop the container mid-session and assert the next command returns `Failed` rather than hanging.
- [ ] **Step 2: Run:** `MONGOSTUDIO_IT=1 cargo test -p mongostudio-core -- --include-ignored`
- [ ] **Step 3: Coverage:** `cargo install cargo-llvm-cov` then `cargo llvm-cov -p mongostudio-core --features test-support --fail-under-lines 80`. Fix gaps with tests, not exclusions.
- [ ] **Step 4: Run the full gate:** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- [ ] **Step 5: Commit** `test: add container-backed integration tests and coverage gate`

---

## Self-Review

- **Spec coverage:** connections/URI/TLS/SRV (T2, T4); SCRAM/X.509/LDAP-PLAIN (T4); Kerberos best-effort + spike (T4, T5, T12 `auth_options`); SSH tunnel + known_hosts (T13); sidebar lazy-load (T6, T10, T14); tree/table/JSON + paging 50 (T6, T15); find bar (T7, T14); CRUD with typed editing (T8, T9, T15); Keychain (T3, T12); i18n (T11 onward); error handling and localized messages (T1, T11, T16); testing and coverage (all tasks, T17). Open spec questions (name/license, ssh-agent) are intentionally not tasks; ssh-agent can be added to T13 if the user decides yes.
- **Placeholders:** none intended; the only deliberate deferral is the dependency versions, which are resolved by `cargo add` at the task that first needs them.
- **Type consistency:** `Namespace`, `Query`, `Page`, `DocumentStore`, `Command`/`Event` and `CoreError` keys are defined once (T1, T6, T10) and used with identical names downstream. `Event::Failed.context` is a `&'static str` used only to pick an i18n key suffix.
- **Review Focus:** items 1-5 each map to the test in the named task.
