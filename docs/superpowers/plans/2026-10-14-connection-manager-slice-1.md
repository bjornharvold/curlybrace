# Connection Manager, Slice 1: Store + Manager UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Saved connections gain folders and timestamps, and a manager screen lets a person create, edit, duplicate, delete, filter, sort, group, import, export and test them, with From/To URI. One browsing session stays active, as today.

**Architecture:** The model grows four `#[serde(default)]` fields so old `connections.json` files load. Pure helpers (folders, duplicate, URI parse/build) live in `curlybrace-core`. Impure manager operations (duplicate with secret copy, folder moves, import/export, `last_connected` stamping) are new functions in `curlybrace-ui/src/app/`, taking `&dyn SecretStore` and the config path as seams like `save`/`delete` do today. Test Connection is a new runtime command that opens a store and tunnel, reports, and drops both without touching the browsing epoch. The connections screen becomes a sortable, filterable, folder-grouped table with a modal edit dialog.

**Tech Stack:** Rust 2024, egui/eframe 0.32, tokio, serde/serde_json, `uuid`, `mongodb` driver, `rfd` (already a dependency), `tempfile` (tests).

**Spec:** `docs/superpowers/specs/2026-10-14-connection-manager-design.md` (issue #45). Read it first.

**Deviation from the spec to record in Task 10:** the connections screen already is the "manager" (it is shown whenever the app is not connected), so the list lives on that screen and only the **edit** form is a modal. The spec said the list is a modal; the spec is updated in Task 10.

## Global Constraints

- Secrets (passwords, SSH password, key passphrase) live only in the `SecretStore`. The connection file, exports, fixtures, snapshots, logs and error text never contain them. `ConnectionConfig` keeps no password field.
- Old `connections.json` files must still load: every new `ConnectionConfig` field is `#[serde(default)]`.
- Every user-visible string is an i18n key in `crates/ui/locales/en.json` (nested, **alphabetical at every level**, enforced by `en_json_is_nested_and_alphabetical_at_every_level`). Never hardcode English. Report new keys at the end.
- Immutability: functions return new values; no in-place mutation of shared state (`let mut` on local builders is fine).
- Files stay under 400 lines typical, 800 max. `crates/ui/src/app/connections.rs` (497 lines) must not grow: new operations go in new files.
- Prefer real objects over mocks; shared fixtures go in `crates/core/src/test_support/mock_objects.rs`.
- Rust gates before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Commits use conventional format (`feat(connections): ...`, lowercase imperative subject) and end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- No Studio 3T names or assets ("IntelliShell" etc.) anywhere.
- Never log URIs, passwords, passphrases or file paths.

## Review Focus

Failure modes the spec implies but no obvious test covers; each has a pinning test in the task named.

1. **Duplicate when the keychain fails halfway** (e.g. password copied, ssh-password read errors): no half-copied secrets and no new connection remain. Task 3 (`copy_all` rollback) and Task 4.
2. **Import of a file that contains an existing id**, duplicate ids inside the file, or a hostile `folder`/`name` (control characters, 10 MB string, 100k entries): fresh ids, validation skips, size/entry caps. Task 5.
3. **Passwords with `@ : / % ? #` and non-ASCII** in From URI / To URI round-trip without corruption and without appearing in any error text. Task 6.
4. **Test Connection must not disturb the browsing session**: the connected store and epoch survive a test, a failed test, and a test while connecting. Task 7.
5. **Stale Test Connection result** from an earlier request must not overwrite a newer one, and editing the form after a test must not leave a stale "success" shown for different settings. Task 8.
6. **Renaming a folder onto an existing folder name** merges instead of losing connections; deleting a folder never deletes connections. Task 2.
7. **A save that fails to persist** (read-only config dir) must not stamp `last_connected` as success or silently drop the change. Tasks 4 and 5.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/clock.rs` (new) | `now_secs()` |
| `crates/core/src/connection.rs` | new fields, validation, `duplicate`, `unique_name`, stamping |
| `crates/core/src/connection_folders.rs` (new) | pure folder helpers |
| `crates/core/src/secrets.rs` | `SecretKind::ALL`, `SecretString`, `copy_all` |
| `crates/core/src/uri.rs` (new) | `parse_uri`, `build_uri`, `PasswordMode` |
| `crates/core/src/runtime/{mod,handlers}.rs` | `Command::TestConnection`, `Event::TestConnection`, `TestReport` |
| `crates/core/src/store.rs`, `mongo_store.rs` | `DocumentStore::server_version` |
| `crates/ui/src/app/persistence.rs` | extract `parse_connections` |
| `crates/ui/src/app/connection_ops.rs` (new) | duplicate, folder ops, `mark_connected` |
| `crates/ui/src/app/connection_io.rs` (new) | import/export |
| `crates/ui/src/app/connection_picker.rs` (new) | open/save dialog flow for import/export |
| `crates/ui/src/viewmodel/connection_list.rs` (new) | filter, sort, group, formatting |
| `crates/ui/src/viewmodel/connection_form.rs` | carry metadata, `apply_parsed` |
| `crates/ui/src/views/connections.rs` | state + orchestration only |
| `crates/ui/src/views/connections_list.rs` (new) | toolbar + table |
| `crates/ui/src/views/connections_edit.rs` (new) | edit modal (moved form code) |
| `crates/ui/locales/en.json`, `src/i18n_usage_tests.rs` | keys, `connections.` prefix |

---

### Task 1: Model fields, clock, and metadata carried through the form

**Files:**
- Create: `crates/core/src/clock.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod clock;`), `crates/core/src/connection.rs`, `crates/ui/src/viewmodel/connection_form.rs`, `crates/ui/src/app/connections.rs` (`save` signature), `crates/ui/src/app/mod.rs:245` (call site)
- Test: inline `#[cfg(test)]` modules in those files

**Interfaces:**
- Produces:
  - `curlybrace_core::clock::now_secs() -> i64`
  - `ConnectionConfig { folder: Option<String>, created_at: Option<i64>, updated_at: Option<i64>, last_connected: Option<i64>, .. }`
  - `ConnectionConfig::with_save_stamp(self, now: i64) -> Self` (sets `created_at` if absent, always `updated_at`)
  - `ConnectionConfig::with_connected_at(self, now: i64) -> Self`
  - `ConnectionConfig::duplicate(&self, name: String, now: i64) -> Self`
  - `unique_name(base: &str, existing: &[ConnectionConfig]) -> String`
  - `ConnectionForm { folder: String, created_at: Option<i64>, last_connected: Option<i64>, .. }`
  - `connections::save(state, form, secrets, path, now: i64) -> AppState`

- [ ] **Step 1: Write failing core tests** (append to `mod tests` in `crates/core/src/connection.rs`)

```rust
    #[test]
    fn old_files_without_the_new_fields_still_load() {
        let old = r#"{"id":"6f9619ff-8b86-d011-b42d-00c04fc964ff","name":"a","uri":"mongodb://h",
            "auth":"None","username":null,"auth_source":null,
            "tls":{"enabled":false,"ca_file":null,"client_cert_key_file":null,"allow_invalid_certs":false},
            "ssh":null}"#;
        let c: ConnectionConfig = serde_json::from_str(old).unwrap();
        assert_eq!(c.folder, None);
        assert_eq!(c.created_at, None);
        assert_eq!(c.updated_at, None);
        assert_eq!(c.last_connected, None);
    }

    #[test]
    fn save_stamp_keeps_created_and_refreshes_updated() {
        let first = ConnectionConfig::new("a", "mongodb://h").with_save_stamp(100);
        assert_eq!((first.created_at, first.updated_at), (Some(100), Some(100)));
        let second = first.with_save_stamp(200);
        assert_eq!((second.created_at, second.updated_at), (Some(100), Some(200)));
    }

    #[test]
    fn duplicate_gets_a_new_id_and_fresh_metadata_and_keeps_settings() {
        let mut src = ConnectionConfig::new("prod", "mongodb://h").with_save_stamp(10);
        src.folder = Some("work".into());
        src.last_connected = Some(50);
        src.username = Some("bob".into());
        let copy = src.duplicate("prod copy".into(), 300);
        assert_ne!(copy.id, src.id);
        assert_eq!(copy.name, "prod copy");
        assert_eq!(copy.folder.as_deref(), Some("work"));
        assert_eq!(copy.username.as_deref(), Some("bob"));
        assert_eq!((copy.created_at, copy.updated_at), (Some(300), Some(300)));
        assert_eq!(copy.last_connected, None);
    }

    #[test]
    fn unique_name_appends_a_counter_only_when_taken() {
        let a = ConnectionConfig::new("prod copy", "mongodb://h");
        let b = ConnectionConfig::new("prod copy 2", "mongodb://h");
        assert_eq!(unique_name("fresh", &[a.clone()]), "fresh");
        assert_eq!(unique_name("prod copy", &[a.clone()]), "prod copy 2");
        assert_eq!(unique_name("prod copy", &[a, b]), "prod copy 3");
    }

    #[test]
    fn folder_must_be_short_and_free_of_control_characters() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.folder = Some("ok folder".into());
        assert!(c.validate().is_ok());
        c.folder = Some("x".repeat(MAX_FOLDER_LEN + 1));
        assert!(c.validate().is_err());
        c.folder = Some("bad\nname".into());
        assert!(c.validate().is_err());
        c.folder = Some("   ".into());
        assert!(c.validate().is_err(), "a blank folder must be None, not an empty string");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-core connection::tests`
Expected: compile errors (`no field folder`, `with_save_stamp` not found).

- [ ] **Step 3: Implement**

`crates/core/src/clock.rs`:

```rust
//! Wall-clock seconds, kept in one place so callers can pass `now` in tests.

/// Seconds since the Unix epoch; 0 if the clock is before it.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
```

In `crates/core/src/connection.rs` add after the imports `pub const MAX_FOLDER_LEN: usize = 80;`, add the four fields to `ConnectionConfig` (each preceded by `#[serde(default)]`), set them to `None` in `new`, then:

```rust
impl ConnectionConfig {
    pub fn with_save_stamp(self, now: i64) -> Self {
        Self {
            created_at: self.created_at.or(Some(now)),
            updated_at: Some(now),
            ..self
        }
    }

    pub fn with_connected_at(self, now: i64) -> Self {
        Self {
            last_connected: Some(now),
            ..self
        }
    }

    /// A copy with a new id and fresh timestamps. Secrets are not part of the config: the
    /// caller copies them (see `secrets::copy_all`).
    pub fn duplicate(&self, name: String, now: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            created_at: Some(now),
            updated_at: Some(now),
            last_connected: None,
            ..self.clone()
        }
    }

    fn validate_folder(&self) -> Result<()> {
        let Some(folder) = &self.folder else {
            return Ok(());
        };
        let bad = folder.trim().is_empty()
            || folder.trim() != folder
            || folder.chars().count() > MAX_FOLDER_LEN
            || folder.chars().any(char::is_control);
        if bad {
            return Err(CoreError::InvalidUri("invalid folder name".into()));
        }
        Ok(())
    }
}

/// `base`, or `base 2`, `base 3`, ... if a connection already has that name.
pub fn unique_name(base: &str, existing: &[ConnectionConfig]) -> String {
    let taken = |n: &str| existing.iter().any(|c| c.name == n);
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| base.to_owned())
}
```

Call `self.validate_folder()?;` in `validate()` right after `self.validate_uri()?;`. Also add `pub mod clock;` to `lib.rs`.

- [ ] **Step 4: Run core tests**

Run: `cargo test -p curlybrace-core connection::tests`
Expected: PASS.

- [ ] **Step 5: Failing form + save tests** (append to `mod tests` in `connection_form.rs`, and to the test module in `app/connections.rs`)

```rust
    #[test]
    fn form_round_trips_folder_and_timestamps() {
        let mut c = ConnectionConfig::new("a", "mongodb://h").with_save_stamp(10);
        c.folder = Some("work".into());
        c.last_connected = Some(99);
        let back = ConnectionForm::from_config(&c).to_config().unwrap();
        assert_eq!(back.folder.as_deref(), Some("work"));
        assert_eq!(back.created_at, Some(10));
        assert_eq!(back.last_connected, Some(99));
    }

    #[test]
    fn blank_folder_text_becomes_none() {
        let mut form = ConnectionForm::from_config(&ConnectionConfig::new("a", "mongodb://h"));
        form.folder = "   ".into();
        assert_eq!(form.to_config().unwrap().folder, None);
    }
```

In `app/connections.rs` tests (use the existing helpers there; mirror a neighbouring `save` test for setup):

```rust
    #[test]
    fn save_stamps_created_once_and_updated_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let path: ConfigPath = Ok(dir.path().join("connections.json"));
        let secrets = MemorySecretStore::new();
        let mut form = ConnectionForm::default();
        form.name = "a".into();
        form.uri = "mongodb://h".into();
        let state = save(AppState::new(vec![]), &form, &secrets, &path, 100);
        let saved = state.conn.connections[0].clone();
        assert_eq!((saved.created_at, saved.updated_at), (Some(100), Some(100)));
        let state = save(state, &state_form(&saved), &secrets, &path, 200);
        let again = &state.conn.connections[0];
        assert_eq!((again.created_at, again.updated_at), (Some(100), Some(200)));
    }
```

with a helper `fn state_form(c: &ConnectionConfig) -> ConnectionForm { ConnectionForm::from_config(c) }` in the test module.

Run: `cargo test -p curlybrace-ui connection` — Expected: compile errors.

- [ ] **Step 6: Implement form and save changes**

In `ConnectionForm` add `pub folder: String`, `pub created_at: Option<i64>`, `pub last_connected: Option<i64>` (Default: empty, `None`, `None`). In `to_config()` build the struct with:

```rust
            folder: non_blank(&self.folder),
            created_at: self.created_at,
            updated_at: None,
            last_connected: self.last_connected,
```

`updated_at: None` here is intentional: `save` stamps it. In `from_config` set `folder: c.folder.clone().unwrap_or_default()`, `created_at: c.created_at`, `last_connected: c.last_connected`. In `save(...)` add the `now: i64` parameter and replace `let cfg = match form.to_config() { Ok(cfg) => cfg, ...}` with the same match then `let cfg = cfg.with_save_stamp(now);` (an `updated_at` of the old config is dropped by design: `with_save_stamp` always refreshes it). In `app/mod.rs` change the call to `connections::save(state, &form, secrets, &self.config_path, curlybrace_core::clock::now_secs())`. Fix every other `save(` call site the compiler reports (tests) by passing a literal.

Run: `cargo test -p curlybrace-core -p curlybrace-ui` — Expected: PASS.

- [ ] **Step 7: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): add folder and timestamp fields to saved connections" -m "Fields default so older connections.json files still load; the form carries them through edits." -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Pure folder helpers

**Files:**
- Create: `crates/core/src/connection_folders.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod connection_folders;`)

**Interfaces:**
- Consumes: `ConnectionConfig`, `MAX_FOLDER_LEN` (Task 1)
- Produces:
  - `normalize_folder(raw: &str) -> Result<Option<String>>`
  - `folder_names(list: &[ConnectionConfig]) -> Vec<String>` (sorted, unique)
  - `moved_to_folder(list: &[ConnectionConfig], id: Uuid, folder: Option<String>, now: i64) -> Vec<ConnectionConfig>`
  - `renamed_folder(list: &[ConnectionConfig], from: &str, to: &str, now: i64) -> Result<Vec<ConnectionConfig>>`
  - `deleted_folder(list: &[ConnectionConfig], name: &str, now: i64) -> Vec<ConnectionConfig>`

- [ ] **Step 1: Write the failing tests** (in the new file's `mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn in_folder(name: &str, folder: Option<&str>) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(name, "mongodb://h");
        c.folder = folder.map(str::to_owned);
        c
    }

    #[test]
    fn normalize_trims_and_maps_blank_to_none() {
        assert_eq!(normalize_folder("  work ").unwrap(), Some("work".into()));
        assert_eq!(normalize_folder("   ").unwrap(), None);
        assert!(normalize_folder("a\tb").is_err());
        assert!(normalize_folder(&"x".repeat(MAX_FOLDER_LEN + 1)).is_err());
    }

    #[test]
    fn folder_names_are_sorted_unique_and_skip_the_root() {
        let list = [
            in_folder("a", Some("zeta")),
            in_folder("b", Some("alpha")),
            in_folder("c", Some("alpha")),
            in_folder("d", None),
        ];
        assert_eq!(folder_names(&list), vec!["alpha".to_owned(), "zeta".to_owned()]);
    }

    #[test]
    fn move_changes_only_that_connection_and_stamps_it() {
        let a = in_folder("a", None);
        let b = in_folder("b", None);
        let out = moved_to_folder(&[a.clone(), b.clone()], a.id, Some("work".into()), 77);
        assert_eq!(out[0].folder.as_deref(), Some("work"));
        assert_eq!(out[0].updated_at, Some(77));
        assert_eq!(out[1], b, "the other connection is untouched");
        assert_eq!(a.folder, None, "the input is not mutated");
    }

    #[test]
    fn rename_moves_every_member_and_merges_into_an_existing_folder() {
        let list = [
            in_folder("a", Some("old")),
            in_folder("b", Some("new")),
            in_folder("c", Some("other")),
        ];
        let out = renamed_folder(&list, "old", "new", 5).unwrap();
        assert_eq!(out.len(), 3, "no connection is lost by a merge");
        assert_eq!(out[0].folder.as_deref(), Some("new"));
        assert_eq!(out[1].folder.as_deref(), Some("new"));
        assert_eq!(out[2].folder.as_deref(), Some("other"));
        assert_eq!(out[2].updated_at, None, "unaffected rows keep their timestamps");
    }

    #[test]
    fn rename_to_a_blank_or_invalid_name_is_an_error() {
        let list = [in_folder("a", Some("old"))];
        assert!(renamed_folder(&list, "old", "  ", 5).is_err());
        assert!(renamed_folder(&list, "old", "a\nb", 5).is_err());
    }

    #[test]
    fn deleting_a_folder_moves_its_members_to_the_root_and_deletes_nothing() {
        let list = [in_folder("a", Some("gone")), in_folder("b", Some("kept"))];
        let out = deleted_folder(&list, "gone", 9);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].folder, None);
        assert_eq!(out[0].updated_at, Some(9));
        assert_eq!(out[1].folder.as_deref(), Some("kept"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-core connection_folders`
Expected: FAIL (module/functions missing).

- [ ] **Step 3: Implement**

```rust
//! Pure helpers for the one-level folder grouping of saved connections. Every function returns
//! a new list; the input is never mutated.
use crate::connection::{ConnectionConfig, MAX_FOLDER_LEN};
use crate::error::{CoreError, Result};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Trimmed folder name, or `None` for blank input (the root). Rejects over-long names and
/// control characters.
pub fn normalize_folder(raw: &str) -> Result<Option<String>> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_FOLDER_LEN || name.chars().any(char::is_control) {
        return Err(CoreError::InvalidUri("invalid folder name".into()));
    }
    Ok(Some(name.to_owned()))
}

/// The distinct non-root folder names, sorted.
pub fn folder_names(list: &[ConnectionConfig]) -> Vec<String> {
    list.iter()
        .filter_map(|c| c.folder.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn restamped(c: &ConnectionConfig, folder: Option<String>, now: i64) -> ConnectionConfig {
    ConnectionConfig {
        folder,
        updated_at: Some(now),
        ..c.clone()
    }
}

pub fn moved_to_folder(
    list: &[ConnectionConfig],
    id: Uuid,
    folder: Option<String>,
    now: i64,
) -> Vec<ConnectionConfig> {
    list.iter()
        .map(|c| {
            if c.id == id {
                restamped(c, folder.clone(), now)
            } else {
                c.clone()
            }
        })
        .collect()
}

/// Renames `from` to `to`; if `to` already exists the two folders merge.
pub fn renamed_folder(
    list: &[ConnectionConfig],
    from: &str,
    to: &str,
    now: i64,
) -> Result<Vec<ConnectionConfig>> {
    let target = normalize_folder(to)?
        .ok_or_else(|| CoreError::InvalidUri("invalid folder name".into()))?;
    Ok(list
        .iter()
        .map(|c| match c.folder.as_deref() {
            Some(f) if f == from => restamped(c, Some(target.clone()), now),
            _ => c.clone(),
        })
        .collect())
}

/// Moves the folder's members to the root. Never deletes a connection.
pub fn deleted_folder(list: &[ConnectionConfig], name: &str, now: i64) -> Vec<ConnectionConfig> {
    list.iter()
        .map(|c| match c.folder.as_deref() {
            Some(f) if f == name => restamped(c, None, now),
            _ => c.clone(),
        })
        .collect()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-core connection_folders`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): add pure folder helpers" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Secret copy with rollback, and a redacting `SecretString`

**Files:**
- Modify: `crates/core/src/secrets.rs`, `crates/ui/src/app/connections.rs` (replace local `ALL_KINDS` with `SecretKind::ALL`)

**Interfaces:**
- Produces:
  - `SecretKind::ALL: [SecretKind; 3]`
  - `copy_all(store: &dyn SecretStore, from: Uuid, to: Uuid) -> Result<()>`: copies every present secret; on any error removes whatever it wrote under `to` and returns the error.
  - `SecretString` (tuple struct over `String`): `new(impl Into<String>)`, `expose(&self) -> &str`, `Clone`, `PartialEq`, `Default`, `Debug` prints `SecretString(<redacted>)`.

- [ ] **Step 1: Write failing tests** (append to `secrets.rs` `mod tests`)

```rust
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
        s.set(&secret_key(from, SecretKind::Password), "pw").unwrap();
        copy_all(&s, from, to).unwrap();
        assert_eq!(s.get(&secret_key(to, SecretKind::Password)).unwrap().as_deref(), Some("pw"));
        assert_eq!(s.get(&secret_key(to, SecretKind::SshPassword)).unwrap(), None);
        assert_eq!(
            s.get(&secret_key(from, SecretKind::Password)).unwrap().as_deref(),
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
        store.inner.set(&secret_key(from, SecretKind::Password), "pw").unwrap();
        store.inner.set(&secret_key(from, SecretKind::SshPassword), "ssh").unwrap();
        let err = copy_all(&store, from, to).unwrap_err();
        assert_eq!(err, CoreError::Secret("boom".into()));
        for kind in SecretKind::ALL {
            assert_eq!(store.get(&secret_key(to, kind)).unwrap(), None, "{kind:?} left behind");
        }
    }

    #[test]
    fn secret_string_never_prints_its_value() {
        let s = SecretString::new("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.expose(), "hunter2");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-core secrets`
Expected: FAIL (items missing).

- [ ] **Step 3: Implement** (in `secrets.rs`)

```rust
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
```

In `app/connections.rs` delete the `ALL_KINDS` const and use `SecretKind::ALL` in `delete`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-core secrets && cargo test -p curlybrace-ui connections`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): add all-or-nothing secret copy and a redacting SecretString" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Manager operations: duplicate, folder moves, last-connected stamp

**Files:**
- Create: `crates/ui/src/app/connection_ops.rs`
- Modify: `crates/ui/src/app/mod.rs` (declare `mod connection_ops;`, route actions, stamp in `pump`), `crates/ui/src/app/connections.rs` (make `persist`, `refused` `pub(super)`), `crates/ui/src/views/connections.rs` (`ConnectionAction` variants)

**Interfaces:**
- Consumes: `copy_all`, `moved_to_folder`, `renamed_folder`, `deleted_folder`, `unique_name`, `duplicate`, `with_connected_at`, `ConfigPath`, `AppState`, `Banner`.
- Produces (all `pub(super)` in `connection_ops`):
  - `duplicate(state, id: Uuid, copy_name: String, now: i64, secrets: &dyn SecretStore, path: &ConfigPath) -> AppState`
  - `move_to_folder(state, id: Uuid, folder: String, now: i64, path: &ConfigPath) -> AppState` (blank string = root)
  - `rename_folder(state, from: String, to: String, now: i64, path: &ConfigPath) -> AppState`
  - `delete_folder(state, name: String, now: i64, path: &ConfigPath) -> AppState`
  - `mark_connected(state, id: Uuid, now: i64, path: &ConfigPath) -> AppState`
  - `ConnectionAction::{Duplicate(Uuid), MoveToFolder{id, folder: String}, RenameFolder{from, to}, DeleteFolder(String)}`

All of them return the state unchanged except for the banner/error on failure, and persist the new list only after the in-memory list was built; if persisting fails, `duplicate` removes the copied secrets and leaves the list unchanged (all or nothing), the other operations keep the in-memory change and show the `Banner::Persist`, exactly like `save` does.

- [ ] **Step 1: Write failing tests** (bottom of the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};

    fn setup() -> (tempfile::TempDir, ConfigPath, MemorySecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let path: ConfigPath = Ok(dir.path().join("connections.json"));
        (dir, path, MemorySecretStore::new())
    }

    fn state_with(list: Vec<ConnectionConfig>) -> AppState {
        AppState::new(list)
    }

    #[test]
    fn duplicate_copies_config_and_secrets_and_persists() {
        let (_dir, path, secrets) = setup();
        let src = ConnectionConfig::new("prod", "mongodb://h").with_save_stamp(1);
        secrets.set(&secret_key(src.id, SecretKind::Password), "pw").unwrap();
        let state = duplicate(state_with(vec![src.clone()]), src.id, "prod copy".into(), 50, &secrets, &path);
        assert_eq!(state.conn.connections.len(), 2);
        let copy = &state.conn.connections[1];
        assert_eq!(copy.name, "prod copy");
        assert_ne!(copy.id, src.id);
        assert_eq!(
            secrets.get(&secret_key(copy.id, SecretKind::Password)).unwrap().as_deref(),
            Some("pw")
        );
        let on_disk = std::fs::read_to_string(path.as_ref().unwrap()).unwrap();
        assert!(on_disk.contains("prod copy"));
        assert!(!on_disk.contains("pw\""), "no secret in the file");
    }

    struct FailingSet(MemorySecretStore);
    impl SecretStore for FailingSet {
        fn get(&self, k: &str) -> curlybrace_core::error::Result<Option<String>> { self.0.get(k) }
        fn set(&self, _: &str, _: &str) -> curlybrace_core::error::Result<()> {
            Err(CoreError::Secret("boom".into()))
        }
        fn delete(&self, k: &str) -> curlybrace_core::error::Result<()> { self.0.delete(k) }
    }

    #[test]
    fn duplicate_that_fails_to_copy_secrets_adds_nothing() {
        let (_dir, path, memory) = setup();
        let src = ConnectionConfig::new("prod", "mongodb://h");
        memory.set(&secret_key(src.id, SecretKind::Password), "pw").unwrap();
        let secrets = FailingSet(memory);
        let state = duplicate(state_with(vec![src.clone()]), src.id, "copy".into(), 50, &secrets, &path);
        assert_eq!(state.conn.connections.len(), 1);
        assert!(state.banner.is_some(), "the failure is surfaced, not swallowed");
        assert!(!path.as_ref().unwrap().exists(), "nothing was written");
    }

    #[test]
    fn duplicate_whose_persist_fails_removes_the_copied_secrets() {
        let (dir, _path, secrets) = setup();
        // A path whose parent is a file cannot be written.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let path: ConfigPath = Ok(blocker.join("connections.json"));
        let src = ConnectionConfig::new("prod", "mongodb://h");
        secrets.set(&secret_key(src.id, SecretKind::Password), "pw").unwrap();
        let state = duplicate(state_with(vec![src.clone()]), src.id, "copy".into(), 50, &secrets, &path);
        assert_eq!(state.conn.connections.len(), 1);
        assert!(state.banner.is_some());
        // Only the source's secret remains.
        assert!(secrets.get(&secret_key(src.id, SecretKind::Password)).unwrap().is_some());
    }

    #[test]
    fn unknown_id_is_a_no_op() {
        let (_dir, path, secrets) = setup();
        let state = duplicate(state_with(vec![]), Uuid::new_v4(), "x".into(), 1, &secrets, &path);
        assert!(state.conn.connections.is_empty());
    }

    #[test]
    fn move_rename_and_delete_folder_persist_the_new_list() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = move_to_folder(state_with(vec![a.clone()]), a.id, "work".into(), 5, &path);
        assert_eq!(state.conn.connections[0].folder.as_deref(), Some("work"));
        let state = rename_folder(state, "work".into(), "play".into(), 6, &path);
        assert_eq!(state.conn.connections[0].folder.as_deref(), Some("play"));
        let state = delete_folder(state, "play".into(), 7, &path);
        assert_eq!(state.conn.connections.len(), 1, "deleting a folder keeps its connections");
        assert_eq!(state.conn.connections[0].folder, None);
        assert!(std::fs::read_to_string(path.as_ref().unwrap()).unwrap().contains("\"a\""));
    }

    #[test]
    fn invalid_folder_name_sets_an_error_and_changes_nothing() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = move_to_folder(state_with(vec![a.clone()]), a.id, "bad\nname".into(), 5, &path);
        assert!(state.conn.error.is_some());
        assert_eq!(state.conn.connections[0].folder, None);
    }

    #[test]
    fn mark_connected_stamps_and_persists() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = mark_connected(state_with(vec![a.clone()]), a.id, 42, &path);
        assert_eq!(state.conn.connections[0].last_connected, Some(42));
        assert!(std::fs::read_to_string(path.as_ref().unwrap()).unwrap().contains("42"));
    }

    #[test]
    fn mark_connected_with_a_blocked_guard_shows_a_banner_and_keeps_the_list() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let mut state = state_with(vec![a.clone()]);
        state.persist = PersistGuard::blocked();
        let state = mark_connected(state, a.id, 42, &path);
        assert!(state.banner.is_some());
        assert_eq!(state.conn.connections[0].last_connected, None);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-ui connection_ops`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement** (top of `connection_ops.rs`)

```rust
//! Manager operations beyond save/delete: duplicate, folder changes and the connect stamp.
//! Like `connections::save`, they take the keychain and the config path as arguments.
use super::connections::{ConfigPath, persist, refused};
use super::persistence::PersistGuard;
use super::state::{AppState, Banner};
use crate::views::connections::ConnectionsState;
use curlybrace_core::connection::{ConnectionConfig, unique_name};
use curlybrace_core::connection_folders::{
    deleted_folder, moved_to_folder, normalize_folder, renamed_folder,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::secrets::{SecretKind, SecretStore, copy_all, secret_key};
use uuid::Uuid;

fn with_list(state: AppState, list: Vec<ConnectionConfig>, path: &ConfigPath) -> AppState {
    let banner = persist(path, &list).err().map(Banner::Persist);
    AppState {
        conn: ConnectionsState {
            connections: list,
            error: None,
            ..state.conn
        },
        banner: banner.or(state.banner),
        ..state
    }
}

fn with_error(state: AppState, error: CoreError) -> AppState {
    AppState {
        conn: ConnectionsState {
            error: Some(error),
            ..state.conn
        },
        ..state
    }
}

/// Adds a copy of `id` named `copy_name` (made unique), with its keychain secrets. All or
/// nothing: a failed secret copy or a failed save leaves neither a config nor secrets behind.
pub(super) fn duplicate(
    state: AppState,
    id: Uuid,
    copy_name: String,
    now: i64,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let Some(source) = state.conn.connections.iter().find(|c| c.id == id) else {
        return state;
    };
    let name = unique_name(&copy_name, &state.conn.connections);
    let copy = source.duplicate(name, now);
    if let Err(error) = copy_all(secrets, id, copy.id) {
        return AppState {
            banner: Some(Banner::Failure {
                context: "duplicate_secrets",
                error,
            }),
            ..state
        };
    }
    let mut list = state.conn.connections.clone();
    list.push(copy.clone());
    if let Err(e) = persist(path, &list) {
        for kind in SecretKind::ALL {
            let _ = secrets.delete(&secret_key(copy.id, kind));
        }
        return AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        };
    }
    AppState {
        conn: ConnectionsState {
            connections: list,
            selected: Some(copy.id),
            error: None,
            ..state.conn
        },
        ..state
    }
}

pub(super) fn move_to_folder(
    state: AppState,
    id: Uuid,
    folder: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    match normalize_folder(&folder) {
        Ok(folder) => {
            let list = moved_to_folder(&state.conn.connections, id, folder, now);
            with_list(state, list, path)
        }
        Err(e) => with_error(state, e),
    }
}

pub(super) fn rename_folder(
    state: AppState,
    from: String,
    to: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    match renamed_folder(&state.conn.connections, &from, &to, now) {
        Ok(list) => with_list(state, list, path),
        Err(e) => with_error(state, e),
    }
}

pub(super) fn delete_folder(state: AppState, name: String, now: i64, path: &ConfigPath) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let list = deleted_folder(&state.conn.connections, &name, now);
    with_list(state, list, path)
}

/// Stamps `last_connected` after a successful connect. A blocked or failing save shows the
/// persistence banner; it never pretends the stamp was saved.
pub(super) fn mark_connected(state: AppState, id: Uuid, now: i64, path: &ConfigPath) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let list: Vec<ConnectionConfig> = state
        .conn
        .connections
        .iter()
        .map(|c| if c.id == id { c.clone().with_connected_at(now) } else { c.clone() })
        .collect();
    with_list(state, list, path)
}
```

Import `PersistGuard` only inside the tests module if the compiler says it is unused at the top. In `connections.rs` change `fn persist` and `fn refused` to `pub(super) fn`. Declare `mod connection_ops;` in `app/mod.rs`. Add to `ConnectionAction` (in `views/connections.rs`): `Duplicate(Uuid)`, `MoveToFolder { id: Uuid, folder: String }`, `RenameFolder { from: String, to: String }`, `DeleteFolder(String)`.

In `handle_connection_action` (`app/mod.rs`) add arms, with `let now = curlybrace_core::clock::now_secs();` taken once at the top of the function:

```rust
            ConnectionAction::Duplicate(id) => {
                let name = self.i18n.t_with(
                    "connections.copy_name",
                    &[("name", &state.connection(id).map(|c| c.name.clone()).unwrap_or_default())],
                );
                self.state = connection_ops::duplicate(state, id, name, now, secrets, &self.config_path);
            }
            ConnectionAction::MoveToFolder { id, folder } => {
                self.state = connection_ops::move_to_folder(state, id, folder, now, &self.config_path);
            }
            ConnectionAction::RenameFolder { from, to } => {
                self.state = connection_ops::rename_folder(state, from, to, now, &self.config_path);
            }
            ConnectionAction::DeleteFolder(name) => {
                self.state = connection_ops::delete_folder(state, name, now, &self.config_path);
            }
```

In `pump()` replace the event loop body so the stamp happens exactly when a connect completes:

```rust
        while let Ok(event) = self.handle.rx.try_recv() {
            if let Some(line) = failure_log_line(&event) {
                log::warn!("{line}");
            }
            let was_connected = self.state.status == ConnStatus::Connected;
            let is_connect = matches!(event, Event::Connected { .. });
            self.dispatch(Input::Event(event));
            if is_connect && !was_connected && self.state.status == ConnStatus::Connected {
                if let Some(id) = self.state.connected {
                    let state = std::mem::take(&mut self.state);
                    self.state = connection_ops::mark_connected(
                        state,
                        id,
                        curlybrace_core::clock::now_secs(),
                        &self.config_path,
                    );
                }
            }
        }
```

(Add the `Event` import if missing; use the existing path in `app/mod.rs`.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-ui connection_ops`
Expected: PASS.

- [ ] **Step 5: Add a wiring test** in `crates/ui/tests/wiring.rs`, next to `browse_edit_and_delete_through_the_real_runtime`, reusing `harness()`: connect, wait until connected (`until`), then reload the config file and assert the saved connection has `last_connected.is_some()`. Use the same helpers the neighbouring test uses to read `app.state` and the harness config path.

Run: `cargo test -p curlybrace-ui --test wiring`
Expected: PASS.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): duplicate, folder operations and last-connected stamp" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Import and export

**Files:**
- Create: `crates/ui/src/app/connection_io.rs`, `crates/ui/src/app/connection_picker.rs`
- Modify: `crates/ui/src/app/persistence.rs` (extract `parse_connections`), `crates/ui/src/app/file_picker.rs` (`open_file`), `crates/ui/src/app/mod.rs`, `crates/ui/src/views/connections.rs` (`ConnectionAction::{StartImport, StartExport}`, `Notice`)

**Interfaces:**
- Produces:
  - `persistence::parse_connections(bytes: &[u8]) -> Result<Loaded, PersistError>` (the body of `load_connections` after the read)
  - `connection_io::MAX_IMPORT_BYTES: usize = 5 * 1024 * 1024`, `MAX_IMPORT_ENTRIES: usize = 2000`
  - `connection_io::export_bytes(list: &[ConnectionConfig]) -> Result<Vec<u8>, PersistError>`
  - `connection_io::prepare_import(bytes: &[u8], existing: &[ConnectionConfig], now: i64) -> Result<Imported, PersistError>` where `Imported { connections: Vec<ConnectionConfig>, skipped: usize }`: every entry gets a fresh id, `created_at = updated_at = Some(now)`, `last_connected = None`, a name unique against `existing` and the batch.
  - `connection_io::import_into(state, path_to_file: &Path, now, config: &ConfigPath) -> AppState` and `export_to(state, path_to_file: &Path) -> AppState`
  - `FilePicker::open_file(&self, extension: &str) -> Option<PathBuf>` (default `None`)
  - `Notice { Imported { imported: usize, skipped: usize }, Exported { count: usize } }` in `views::connections`, stored as `ConnectionsState.notice: Option<Notice>`.

- [ ] **Step 1: Write failing tests** (in `connection_io.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, "mongodb://h")
    }

    #[test]
    fn export_has_no_password_field_and_round_trips_through_import() {
        let mut a = cfg("a");
        a.username = Some("bob".into());
        let bytes = export_bytes(&[a.clone()]).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.to_lowercase().contains("password"));
        let imported = prepare_import(&bytes, &[], 9).unwrap();
        assert_eq!(imported.connections.len(), 1);
        assert_eq!(imported.connections[0].username.as_deref(), Some("bob"));
    }

    #[test]
    fn import_assigns_fresh_ids_even_when_the_file_repeats_an_existing_id() {
        let existing = cfg("a");
        let bytes = export_bytes(&[existing.clone(), existing.clone()]).unwrap();
        let imported = prepare_import(&bytes, &[existing.clone()], 9).unwrap();
        let ids: std::collections::HashSet<_> = imported.connections.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids.contains(&existing.id));
    }

    #[test]
    fn import_makes_names_unique_and_resets_connect_history() {
        let mut a = cfg("a");
        a.last_connected = Some(5);
        let bytes = export_bytes(&[a.clone(), a.clone()]).unwrap();
        let imported = prepare_import(&bytes, &[a], 9).unwrap();
        let names: Vec<_> = imported.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a 2", "a 3"]);
        assert!(imported.connections.iter().all(|c| c.last_connected.is_none()));
        assert!(imported.connections.iter().all(|c| c.created_at == Some(9)));
    }

    #[test]
    fn import_skips_invalid_entries_and_counts_them() {
        let bytes = br#"[{"nope":1}, 7]"#;
        let imported = prepare_import(bytes, &[], 9).unwrap();
        assert_eq!((imported.connections.len(), imported.skipped), (0, 2));
    }

    #[test]
    fn import_rejects_non_json_and_oversized_input() {
        assert_eq!(prepare_import(b"not json", &[], 1).unwrap_err(), PersistError::Corrupt);
        let big = vec![b' '; MAX_IMPORT_BYTES + 1];
        assert!(prepare_import(&big, &[], 1).is_err());
    }

    #[test]
    fn import_rejects_too_many_entries() {
        let many = format!("[{}]", vec!["0"; MAX_IMPORT_ENTRIES + 1].join(","));
        assert!(prepare_import(many.as_bytes(), &[], 1).is_err());
    }

    #[test]
    fn hostile_folder_and_name_entries_are_skipped_by_validation() {
        let mut bad = cfg("a");
        bad.folder = Some("x\ny".into());
        let bytes = serde_json::to_vec(&[bad]).unwrap();
        assert_eq!(prepare_import(&bytes, &[], 1).unwrap().skipped, 1);
    }
}
```

Plus two state-level tests: `import_into` with a real file in a tempdir adds the entries, persists, and sets `notice = Imported`; `import_into` on a blocked guard leaves the list unchanged and shows a banner; `export_to` writes a 0600 file containing the names.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-ui connection_io`
Expected: FAIL.

- [ ] **Step 3: Implement**

First extract in `persistence.rs`: split `load_connections` into the read (unchanged errors) followed by `parse_connections(&bytes)` containing `serde_json::from_slice`, the per-entry deserialize, the `validate()` filter and the `skipped` count. Existing tests must still pass unchanged.

`connection_io.rs`:

```rust
//! Import and export of the saved-connection list. Exports never carry secrets (the type has
//! no secret fields); imports never trust ids, names or sizes from the file.
use super::connections::{ConfigPath, persist, refused};
use super::persistence::{PersistError, atomic_write, parse_connections};
use super::state::{AppState, Banner};
use crate::views::connections::{ConnectionsState, Notice};
use curlybrace_core::connection::{ConnectionConfig, unique_name};
use std::path::Path;
use uuid::Uuid;

pub const MAX_IMPORT_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_IMPORT_ENTRIES: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub connections: Vec<ConnectionConfig>,
    pub skipped: usize,
}

pub fn export_bytes(list: &[ConnectionConfig]) -> Result<Vec<u8>, PersistError> {
    serde_json::to_vec_pretty(list)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))
}

pub fn prepare_import(
    bytes: &[u8],
    existing: &[ConnectionConfig],
    now: i64,
) -> Result<Imported, PersistError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(PersistError::Corrupt);
    }
    let entries = serde_json::from_slice::<Vec<serde_json::Value>>(bytes)
        .map_err(|_| PersistError::Corrupt)?
        .len();
    if entries > MAX_IMPORT_ENTRIES {
        return Err(PersistError::Corrupt);
    }
    let loaded = parse_connections(bytes)?;
    let mut taken = existing.to_vec();
    let mut fresh = Vec::with_capacity(loaded.connections.len());
    for c in loaded.connections {
        let name = unique_name(&c.name, &taken);
        let c = ConnectionConfig {
            id: Uuid::new_v4(),
            name,
            created_at: Some(now),
            updated_at: Some(now),
            last_connected: None,
            ..c
        };
        taken.push(c.clone());
        fresh.push(c);
    }
    Ok(Imported {
        connections: fresh,
        skipped: loaded.skipped,
    })
}

pub(super) fn import_into(
    state: AppState,
    file: &Path,
    now: i64,
    config: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let read = std::fs::metadata(file)
        .map_err(|e| PersistError::Read(e.kind()))
        .and_then(|m| {
            if m.len() > MAX_IMPORT_BYTES as u64 {
                Err(PersistError::Corrupt)
            } else {
                std::fs::read(file).map_err(|e| PersistError::Read(e.kind()))
            }
        });
    let imported = read.and_then(|bytes| prepare_import(&bytes, &state.conn.connections, now));
    match imported {
        Ok(imported) => {
            let count = imported.connections.len();
            let mut list = state.conn.connections.clone();
            list.extend(imported.connections);
            let banner = persist(config, &list).err().map(Banner::Persist);
            AppState {
                conn: ConnectionsState {
                    connections: list,
                    notice: Some(Notice::Imported {
                        imported: count,
                        skipped: imported.skipped,
                    }),
                    ..state.conn
                },
                banner: banner.or(state.banner),
                ..state
            }
        }
        Err(e) => AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        },
    }
}

pub(super) fn export_to(state: AppState, file: &Path) -> AppState {
    let result = export_bytes(&state.conn.connections).and_then(|b| atomic_write(file, &b));
    match result {
        Ok(()) => {
            let count = state.conn.connections.len();
            AppState {
                conn: ConnectionsState {
                    notice: Some(Notice::Exported { count }),
                    ..state.conn
                },
                ..state
            }
        }
        Err(e) => AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        },
    }
}
```

Make `atomic_write` and `parse_connections` `pub(crate)` if they are not. Add `Notice` and `pub notice: Option<Notice>` to `views/connections.rs` (derive `Debug, Clone, PartialEq`), and clear it in `select`/`start_new`.

`file_picker.rs`: add to the trait

```rust
    /// `None` when cancelled. Default: not supported (tests and embedding).
    fn open_file(&self, _extension: &str) -> Option<PathBuf> {
        None
    }
```

and in `NativeFilePicker`:

```rust
    fn open_file(&self, extension: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().add_filter(extension, &[extension]).pick_file()
    }
```

`connection_picker.rs` (mirrors `start_pick`/`poll` for the compare export, with no draft):

```rust
//! The native open/save dialog for importing and exporting connections. The dialog runs on a
//! worker thread; the answer is polled once per frame. Paths are never logged.
use super::file_picker::{FilePicker, PickPoll};
use curlybrace_core::runtime::Repaint;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnPickKind {
    Import,
    Export,
}

#[derive(Debug)]
pub struct ConnPick {
    pub kind: ConnPickKind,
    answers: Receiver<Option<PathBuf>>,
}

impl ConnPick {
    pub fn poll(&self) -> PickPoll {
        match self.answers.try_recv() {
            Ok(path) => PickPoll::Done(path),
            Err(TryRecvError::Empty) => PickPoll::Waiting,
            Err(TryRecvError::Disconnected) => PickPoll::Done(None),
        }
    }
}

const FILE_NAME: &str = "curlybrace-connections.json";
const EXTENSION: &str = "json";

pub fn start(picker: Arc<dyn FilePicker>, kind: ConnPickKind, repaint: Repaint) -> ConnPick {
    let (tx, answers) = channel();
    let work = move || {
        let chosen = match kind {
            ConnPickKind::Import => picker.open_file(EXTENSION),
            ConnPickKind::Export => picker.save_file(FILE_NAME, EXTENSION),
        };
        let _ = tx.send(chosen);
        repaint();
    };
    let _ = std::thread::Builder::new()
        .name("curlybrace-connection-picker".into())
        .spawn(work);
    ConnPick { kind, answers }
}
```

In `app/mod.rs`: add a field `conn_pick: Option<connection_picker::ConnPick>` to the app struct (initialise `None` where `pick` is initialised). `handle_connection_action`: `StartImport`/`StartExport` set `self.conn_pick` via `connection_picker::start(self.picker.clone(), kind, self.repaint.clone())` unless one is already open (use the same field names the compare export uses for its picker and repaint; read how `start_pick` is called today). In `poll_picker` (or a sibling `poll_connection_picker` called from `pump`) take the finished pick and run `connection_io::import_into(state, &path, now, &self.config_path)` or `export_to(state, &path)`; a cancelled dialog does nothing.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-ui connection_io persistence`
Expected: PASS (existing persistence tests still green).

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): import and export saved connections without secrets" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: From URI and To URI (core, pure)

**Files:**
- Create: `crates/core/src/uri.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod uri;`)

**Interfaces:**
- Consumes: `AuthMechanism`, `ConnectionConfig`, `CoreError::InvalidUri`
- Produces:
  - `ParsedUri { uri: String, username: Option<String>, password: Option<SecretString>, auth_source: Option<String>, auth: Option<AuthMechanism>, tls: Option<bool> }` (`uri` has no userinfo, no `authSource`/`authMechanism`/`tls`/`ssl` query keys)
  - `parse_uri(input: &str) -> Result<ParsedUri, CoreError>`
  - `PasswordMode<'a> { Omit, Masked, Include(&'a str) }`
  - `build_uri(cfg: &ConnectionConfig, password: PasswordMode<'_>) -> String`
  - `MASK: &str = "****"`

Error messages are fixed static strings (`"scheme must be mongodb:// or mongodb+srv://"`, `"missing host"`, `"malformed connection string"`, `"unsupported authentication mechanism"`, `"secrets must not be embedded in the uri"`); they never contain any part of the input.

- [ ] **Step 1: Write failing tests** (`mod tests` in `uri.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionConfig;

    #[test]
    fn parses_credentials_options_and_strips_them_from_the_uri() {
        let p = parse_uri(
            "mongodb://bob:s3cr%40t@h1:27017,h2:27018/app?replicaSet=rs0&authSource=admin&authMechanism=SCRAM-SHA-256&tls=true",
        )
        .unwrap();
        assert_eq!(p.uri, "mongodb://h1:27017,h2:27018/app?replicaSet=rs0");
        assert_eq!(p.username.as_deref(), Some("bob"));
        assert_eq!(p.password.as_ref().map(SecretString::expose), Some("s3cr@t"));
        assert_eq!(p.auth_source.as_deref(), Some("admin"));
        assert_eq!(p.auth, Some(AuthMechanism::ScramSha256));
        assert_eq!(p.tls, Some(true));
    }

    #[test]
    fn username_without_a_mechanism_defaults_to_scram_sha_256() {
        let p = parse_uri("mongodb://bob:pw@h/").unwrap();
        assert_eq!(p.auth, Some(AuthMechanism::ScramSha256));
    }

    #[test]
    fn no_userinfo_means_no_credentials_and_no_auth_change() {
        let p = parse_uri("mongodb+srv://cluster0.example.com/db").unwrap();
        assert_eq!((p.username, p.password, p.auth), (None, None, None));
        assert_eq!(p.uri, "mongodb+srv://cluster0.example.com/db");
    }

    #[test]
    fn accepts_a_username_without_a_password() {
        let p = parse_uri("mongodb://bob@h/db").unwrap();
        assert_eq!(p.username.as_deref(), Some("bob"));
        assert_eq!(p.password, None);
    }

    #[test]
    fn rejects_bad_input_without_echoing_it() {
        let secret = "hunter2-do-not-leak";
        for bad in [
            format!("http://u:{secret}@h"),
            format!("mongodb://u:{secret}@"),
            format!("mongodb://u:%zz{secret}@h"),
            format!("mongodb://u:{secret}@h/?authMechanism=NOPE"),
            format!("mongodb://u:p@h/?tlsCertificateKeyFilePassword={secret}"),
            String::new(),
        ] {
            let err = parse_uri(&bad).unwrap_err().to_string();
            assert!(!err.contains(secret), "{bad:?} leaked into {err:?}");
        }
    }

    #[test]
    fn password_round_trips_for_awkward_characters() {
        for pw in ["p@ss:/%word", "a b?c#d", "ünï€", "100%", "::@@//"] {
            let mut cfg = ConnectionConfig::new("a", "mongodb://h:27017/db");
            cfg.username = Some("us:er@x".into());
            let built = build_uri(&cfg, PasswordMode::Include(pw));
            let parsed = parse_uri(&built).unwrap();
            assert_eq!(parsed.username.as_deref(), Some("us:er@x"));
            assert_eq!(parsed.password.as_ref().map(SecretString::expose), Some(pw), "{built}");
            assert_eq!(parsed.uri, "mongodb://h:27017/db");
        }
    }

    #[test]
    fn build_masks_by_default_and_omits_when_asked() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://h/db?replicaSet=rs0");
        cfg.username = Some("bob".into());
        cfg.auth_source = Some("admin".into());
        assert_eq!(
            build_uri(&cfg, PasswordMode::Masked),
            "mongodb://bob:****@h/db?replicaSet=rs0&authSource=admin"
        );
        assert_eq!(
            build_uri(&cfg, PasswordMode::Omit),
            "mongodb://bob@h/db?replicaSet=rs0&authSource=admin"
        );
    }

    #[test]
    fn build_without_a_username_has_no_userinfo_even_if_a_password_is_given() {
        let cfg = ConnectionConfig::new("a", "mongodb://h");
        assert_eq!(build_uri(&cfg, PasswordMode::Include("pw")), "mongodb://h");
    }

    #[test]
    fn build_adds_the_mechanism_and_tls_options() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://h/db");
        cfg.username = Some("bob".into());
        cfg.auth = AuthMechanism::ScramSha1;
        cfg.tls.enabled = true;
        let built = build_uri(&cfg, PasswordMode::Omit);
        assert!(built.contains("authMechanism=SCRAM-SHA-1"));
        assert!(built.contains("tls=true"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-core uri::`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Connection-string parsing and building. Pure and synchronous. Errors are fixed strings and
//! never contain any part of the input (it can hold a password).
use crate::connection::{AuthMechanism, ConnectionConfig};
use crate::error::{CoreError, Result};
use crate::secrets::SecretString;

pub const MASK: &str = "****";

const SECRET_QUERY_KEYS: [&str; 2] = ["tlscertificatekeyfilepassword", "proxypassword"];

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedUri {
    /// The connection string without userinfo and without the options held as form fields.
    pub uri: String,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub auth_source: Option<String>,
    pub auth: Option<AuthMechanism>,
    pub tls: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
pub enum PasswordMode<'a> {
    Omit,
    Masked,
    Include(&'a str),
}

fn invalid(msg: &'static str) -> CoreError {
    CoreError::InvalidUri(msg.into())
}

pub fn parse_uri(input: &str) -> Result<ParsedUri> {
    let input = input.trim();
    let (scheme, rest) = if let Some(r) = input.strip_prefix("mongodb+srv://") {
        ("mongodb+srv://", r)
    } else if let Some(r) = input.strip_prefix("mongodb://") {
        ("mongodb://", r)
    } else {
        return Err(invalid("scheme must be mongodb:// or mongodb+srv://"));
    };
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let (userinfo, hosts) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    if hosts.is_empty() {
        return Err(invalid("missing host"));
    }
    let (username, password) = match userinfo {
        None => (None, None),
        Some(u) => {
            let (user, pass) = match u.split_once(':') {
                Some((user, pass)) => (user, Some(pass)),
                None => (u, None),
            };
            let user = percent_decode(user)?;
            let pass = pass.map(percent_decode).transpose()?.map(SecretString::new);
            ((!user.is_empty()).then_some(user), pass)
        }
    };
    let (path, query) = match tail.split_once('?') {
        Some((p, q)) => (p, q),
        None => (tail, ""),
    };
    let mut kept = Vec::new();
    let (mut auth_source, mut auth, mut tls) = (None, None, None);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let lower = key.to_ascii_lowercase();
        if SECRET_QUERY_KEYS.contains(&lower.as_str()) {
            return Err(invalid("secrets must not be embedded in the uri"));
        }
        match lower.as_str() {
            "authsource" => auth_source = Some(percent_decode(value)?),
            "authmechanism" => auth = Some(parse_mechanism(value)?),
            "tls" | "ssl" => tls = Some(value.eq_ignore_ascii_case("true")),
            _ => kept.push(pair),
        }
    }
    if auth.is_none() && username.is_some() {
        auth = Some(AuthMechanism::ScramSha256);
    }
    let mut uri = format!("{scheme}{hosts}{path}");
    if !kept.is_empty() {
        uri.push(if path.is_empty() { '/' } else { '?' });
        if path.is_empty() {
            uri.push('?');
        }
        uri.push_str(&kept.join("&"));
    }
    Ok(ParsedUri {
        uri,
        username,
        password,
        auth_source: auth_source.filter(|s| !s.is_empty()),
        auth,
        tls,
    })
}

fn parse_mechanism(value: &str) -> Result<AuthMechanism> {
    Ok(match value.to_ascii_uppercase().as_str() {
        "SCRAM-SHA-1" => AuthMechanism::ScramSha1,
        "SCRAM-SHA-256" => AuthMechanism::ScramSha256,
        "MONGODB-X509" => AuthMechanism::X509,
        "GSSAPI" => AuthMechanism::Gssapi,
        "PLAIN" => AuthMechanism::Plain,
        _ => return Err(invalid("unsupported authentication mechanism")),
    })
}

fn mechanism_name(auth: AuthMechanism) -> Option<&'static str> {
    match auth {
        AuthMechanism::None => None,
        AuthMechanism::ScramSha1 => Some("SCRAM-SHA-1"),
        AuthMechanism::ScramSha256 => Some("SCRAM-SHA-256"),
        AuthMechanism::X509 => Some("MONGODB-X509"),
        AuthMechanism::Gssapi => Some("GSSAPI"),
        AuthMechanism::Plain => Some("PLAIN"),
    }
}

/// Builds a connection string from a saved config. `Masked` shows `****` in place of the
/// password; only `Include` ever carries a real one. Without a username there is no userinfo.
pub fn build_uri(cfg: &ConnectionConfig, password: PasswordMode<'_>) -> String {
    let (scheme, rest) = match cfg.uri.strip_prefix("mongodb+srv://") {
        Some(r) => ("mongodb+srv://", r),
        None => ("mongodb://", cfg.uri.strip_prefix("mongodb://").unwrap_or(&cfg.uri)),
    };
    let userinfo = cfg.username.as_deref().map(|user| {
        let user = percent_encode(user);
        match password {
            PasswordMode::Omit => format!("{user}@"),
            PasswordMode::Masked => format!("{user}:{MASK}@"),
            PasswordMode::Include(pw) => format!("{user}:{}@", percent_encode(pw)),
        }
    });
    let mut extras = Vec::new();
    if let Some(source) = &cfg.auth_source {
        extras.push(format!("authSource={}", percent_encode(source)));
    }
    if let Some(name) = mechanism_name(cfg.auth) {
        extras.push(format!("authMechanism={name}"));
    }
    if cfg.tls.enabled {
        extras.push("tls=true".to_owned());
    }
    let mut out = format!("{scheme}{}{rest}", userinfo.unwrap_or_default());
    if !extras.is_empty() {
        let has_query = out.contains('?');
        let has_path = out[scheme.len()..].contains('/');
        out.push(if has_query { '&' } else if has_path { '?' } else { '/' });
        if !has_query && !has_path {
            out.push('?');
        }
        out.push_str(&extras.join("&"));
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| invalid("malformed connection string"))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| invalid("malformed connection string"))
}
```

Note for the implementer: the `authority.rsplit_once('@')` split relies on `@`, `/`, `?` inside credentials being percent-encoded, which the spec requires. A raw `/` in a password makes the authority end early; the `rejects_bad_input_without_echoing_it` test covers that no input is echoed either way. If a test above fails on the exact kept-query formatting for a URI with no path, fix `parse_uri`'s join, not the test.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-core uri::`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): parse and build connection strings with a masked password" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Test Connection in the runtime

**Files:**
- Modify: `crates/core/src/runtime/mod.rs`, `crates/core/src/runtime/handlers.rs`, `crates/core/src/runtime/tunneling.rs` (ssh secret override), `crates/core/src/store.rs` (`server_version`), `crates/core/src/mongo_store.rs`, `crates/core/src/runtime/compare/mod.rs:492` (pass `None`)
- Test: `crates/core/src/runtime/test_connection_tests.rs` (new, `#[cfg(test)] mod` declared in `runtime/mod.rs`)

**Interfaces:**
- Consumes: `SecretString` (Task 3), `Connector`, `open_tunnel_if_configured`, `read_password`.
- Produces:
  - `Command::TestConnection { request_id: u64, cfg: ConnectionConfig, secrets: TestSecrets }`
  - `TestSecrets { password: Option<SecretString>, ssh_secret: Option<SecretString> }` (`Default`, `Clone`, `Debug` redacting via `SecretString`). `None` means "use the stored secret".
  - `Event::TestConnection { request_id: u64, outcome: Result<TestReport, CoreError> }`
  - `TestReport { server_version: Option<String>, tls: bool, authenticated: bool }`
  - `DocumentStore::server_version(&self) -> Result<Option<String>>` (default `Ok(None)`)
  - `open_tunnel_if_configured(shared, job, cfg, ssh_secret: Option<&str>)`

Rules: `TestConnection` never calls `begin_epoch`, never touches the installed store or tunnel, runs with its own always-current `Currentness`, is bounded by `connect_timeout`, and its tunnel and store are dropped before the event is sent. It still fails closed on unknown or changed SSH host keys (same `host_key_prompt`).

- [ ] **Step 1: Write failing tests** (`test_connection_tests.rs`). Reuse the fake `Connector` and `RuntimeConfig` patterns from `crates/core/src/runtime/epoch_tests.rs` and `prompt_gate_tests.rs` (read those two files first for the helper names; the fakes below assume a `FakeConnector` returning a ready store and a variant that fails with `CoreError::AuthFailed`, plus a `store_with_version(Option<&str>)` helper; create them in this file if the existing helpers are private).

```rust
    #[tokio::test]
    async fn a_successful_test_reports_the_server_version_and_installs_nothing() {
        let mut rt = start_with(FakeConnector::ok_with_version("7.0.5"));
        rt.tx.send(Command::TestConnection {
            request_id: 1,
            cfg: ConnectionConfig::new("a", "mongodb://h"),
            secrets: TestSecrets::default(),
        }).unwrap();
        let ev = next_event(&mut rt).await;
        assert_eq!(
            ev,
            Event::TestConnection {
                request_id: 1,
                outcome: Ok(TestReport { server_version: Some("7.0.5".into()), tls: false, authenticated: false }),
            }
        );
        rt.tx.send(Command::ListDatabases).unwrap();
        assert!(matches!(
            next_event(&mut rt).await,
            Event::Failed { context: "list_databases", .. }
        ), "a test must not leave a connected store behind");
    }

    #[tokio::test]
    async fn a_test_does_not_disturb_an_installed_session() {
        let mut rt = start_connected();           // existing helper pattern: Connect, await Connected
        rt.tx.send(Command::TestConnection { request_id: 7, cfg: ConnectionConfig::new("b", "mongodb://h"), secrets: TestSecrets::default() }).unwrap();
        let _ = next_event(&mut rt).await;
        rt.tx.send(Command::ListDatabases).unwrap();
        assert!(matches!(next_event(&mut rt).await, Event::Databases(_)));
    }

    #[tokio::test]
    async fn a_failed_test_reports_the_typed_error_and_keeps_the_session() {
        let mut rt = start_connected_then_failing_connector();
        rt.tx.send(Command::TestConnection { request_id: 2, cfg: ConnectionConfig::new("b", "mongodb://h"), secrets: TestSecrets::default() }).unwrap();
        assert_eq!(
            next_event(&mut rt).await,
            Event::TestConnection { request_id: 2, outcome: Err(CoreError::AuthFailed) }
        );
        rt.tx.send(Command::ListDatabases).unwrap();
        assert!(matches!(next_event(&mut rt).await, Event::Databases(_)));
    }

    #[tokio::test]
    async fn an_invalid_config_fails_without_touching_the_connector() {
        let connector = FakeConnector::counting();
        let mut rt = start_with(connector.clone());
        rt.tx.send(Command::TestConnection { request_id: 3, cfg: ConnectionConfig::new(" ", "mongodb://h"), secrets: TestSecrets::default() }).unwrap();
        assert!(matches!(next_event(&mut rt).await, Event::TestConnection { request_id: 3, outcome: Err(CoreError::InvalidUri(_)) }));
        assert_eq!(connector.calls(), 0);
    }

    #[tokio::test]
    async fn typed_secrets_win_over_stored_ones_and_are_not_in_debug_output() {
        let connector = FakeConnector::recording_password();
        let mut rt = start_with_stored_password(connector.clone(), "stored");
        let secrets = TestSecrets { password: Some(SecretString::new("typed")), ssh_secret: None };
        assert!(!format!("{secrets:?}").contains("typed"));
        rt.tx.send(Command::TestConnection { request_id: 4, cfg: ConnectionConfig::new("a", "mongodb://h"), secrets }).unwrap();
        let _ = next_event(&mut rt).await;
        assert_eq!(connector.last_password().as_deref(), Some("typed"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-core test_connection`
Expected: FAIL (variants missing).

- [ ] **Step 3: Implement**

In `store.rs` add to `DocumentStore`:

```rust
    /// The server's version string, if it can be read. Informational: a failure is `None`.
    async fn server_version(&self) -> Result<Option<String>> {
        Ok(None)
    }
```

In `mongo_store.rs`'s `impl DocumentStore for MongoStore`:

```rust
    async fn server_version(&self) -> Result<Option<String>> {
        let info = self
            .client
            .database("admin")
            .run_command(doc! { "buildInfo": 1 })
            .await;
        Ok(info
            .ok()
            .and_then(|d| d.get_str("version").ok().map(str::to_owned)))
    }
```

In `runtime/mod.rs`:

```rust
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestSecrets {
    pub password: Option<crate::secrets::SecretString>,
    pub ssh_secret: Option<crate::secrets::SecretString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TestReport {
    pub server_version: Option<String>,
    pub tls: bool,
    pub authenticated: bool,
}
```

Add `TestConnection { request_id: u64, cfg: ConnectionConfig, secrets: TestSecrets }` to `Command`, `context()` arm `"test_connection"`; add `TestConnection { request_id: u64, outcome: Result<TestReport, CoreError> }` to `Event`. Add `Command::TestConnection { .. }` to the `unreachable!` arm of `data_command` in `handlers.rs`. At the top of `dispatch()` (before the generation logic):

```rust
    if let Command::TestConnection { request_id, cfg, secrets } = command {
        spawn_test(shared, events, repaint, request_id, cfg, secrets);
        return;
    }
```

and in `handlers.rs`:

```rust
struct AlwaysCurrent;
impl Currentness for AlwaysCurrent {
    fn is_current(&self) -> bool {
        true
    }
}

pub(super) fn spawn_test(
    shared: Arc<Shared>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    repaint: super::Repaint,
    request_id: u64,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
) {
    tokio::spawn(async move {
        // An inner task so a panic still produces exactly one answer.
        let work = tokio::spawn(async move { test_connection(&shared, cfg, secrets).await });
        let outcome = work
            .await
            .unwrap_or_else(|_| Err(CoreError::Database("internal error".into())));
        if events
            .send(Event::TestConnection { request_id, outcome })
            .is_ok()
        {
            repaint();
        }
    });
}

async fn test_connection(
    shared: &Arc<Shared>,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
) -> Result<super::TestReport> {
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    bounded(limit, async {
        let password = match &secrets.password {
            Some(p) => Some(p.expose().to_owned()),
            None => read_password(shared, &cfg).await?,
        };
        let job: Arc<dyn Currentness> = Arc::new(AlwaysCurrent);
        let ssh_secret = secrets.ssh_secret.as_ref().map(|s| s.expose());
        let tunnel = open_tunnel_if_configured(shared, job, &cfg, ssh_secret).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared.connector.connect(&cfg, password.as_deref(), addr).await?;
        let server_version = store.server_version().await?;
        // Dropped here, before the answer: a test leaves no client and no tunnel behind.
        drop(store);
        drop(tunnel);
        Ok(super::TestReport {
            server_version,
            tls: cfg.tls.enabled,
            authenticated: cfg.auth != crate::connection::AuthMechanism::None,
        })
    })
    .await
}
```

Import `Currentness` from `super::tunneling` (it is `pub(crate)`). In `tunneling.rs` add the `ssh_secret: Option<&str>` parameter to `open_tunnel_if_configured`: when `Some`, use it instead of calling `read_ssh_secret`:

```rust
    let secret = match ssh_secret {
        Some(s) => Some(s.to_owned()),
        None => read_ssh_secret(shared, cfg, ssh).await?,
    };
```

Pass `None` at the two existing call sites (`handlers.rs` `connect_store`, `compare/mod.rs` `open_side`). Fix the exhaustive-match errors the compiler reports (for example in the UI's `failure_log_line` and any `Event` matches): `Event::TestConnection` is not a failure event, so those arms return `None`/ignore it.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-core`
Expected: PASS, including the existing epoch and prompt-gate tests.

- [ ] **Step 5: Tier-2 integration test** in `crates/core/tests/docker_mongo_auth.rs` style: a test (gated like its neighbours) that starts the auth container, runs `TestConnection` with the right and wrong password and asserts `Ok(report)` with a non-empty `server_version` and `Err(CoreError::AuthFailed)`. Mirror `scram_config`.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): add a Test Connection runtime command that opens no session" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: UI state for Test Connection, From URI and To URI

**Files:**
- Modify: `crates/ui/src/views/connections.rs` (state fields, actions), `crates/ui/src/app/reducer.rs` (`Input::TestConnection`, event arm), `crates/ui/src/app/actions.rs` (or a new small `app/connection_test.rs`, preferred to keep `actions.rs` short), `crates/ui/src/viewmodel/connection_form.rs` (`apply_parsed`), `crates/ui/src/app/mod.rs` (route `BuildUri`, `Test`)

**Interfaces:**
- Consumes: `TestSecrets`, `TestReport`, `Command::TestConnection`, `parse_uri`, `build_uri`, `PasswordMode`, `SecretString`.
- Produces:
  - `TestState { Idle, Running { request_id: u64 }, Done { request_id: u64, outcome: Result<TestReport, CoreError> } }` (`Default = Idle`) and `ConnectionsState { test: TestState, test_seq: u64, uri_input: String, built_uri: Option<SecretString>, .. }`
  - `Input::TestConnection(Box<ConnectionForm>)`
  - `ConnectionAction::{Test(Box<ConnectionForm>), BuildUri { form: Box<ConnectionForm>, include_password: bool }}`
  - `ConnectionForm::apply_parsed(self, parsed: ParsedUri) -> Self`: fills `uri`, `username`, `auth`, `auth_source`, `tls_enabled`, and sets `password` only when the parsed URI had one; clears `clear_password` when it does.
  - `connection_test::start(state: AppState, form: &ConnectionForm) -> (AppState, Vec<Command>)`; `connection_test::on_event(state, request_id, outcome) -> AppState`

Rules (pin with tests): a result is applied only when `request_id == test_seq` and the state is `Running` for it; `Input::TestConnection` with an invalid form sets `conn.error` and sends no command; any edit that changes a form field resets `test` to `Idle` (do this in the view when the form's `PartialEq` differs from the form snapshot taken at the test start: store that snapshot-free check as `TestState::Done` being cleared whenever `state.form != state.test_form`; implement by keeping `test_form: Option<ConnectionForm>` in `ConnectionsState` and calling `ConnectionsState::invalidate_stale_test()` once per frame before drawing); the pasted URI buffer is cleared after a successful parse; the built URI is cleared when the selection or form changes or the dialog closes. Errors from `parse_uri` go to `conn.error`; the pasted text is never echoed.

- [ ] **Step 1: Write failing tests**

```rust
    // reducer-level, using the existing `event(state, Event::..)`/`Input` helpers from reducer_tests.rs
    #[test]
    fn test_connection_sends_a_command_with_typed_secrets_and_marks_running() {
        let mut form = valid_form();
        form.password = "typed".into();
        let (state, cmds) = reduce(AppState::default(), Input::TestConnection(Box::new(form)));
        assert!(matches!(state.conn.test, TestState::Running { request_id: 1 }));
        match &cmds[..] {
            [Command::TestConnection { request_id: 1, secrets, .. }] => {
                assert_eq!(secrets.password.as_ref().map(|p| p.expose()), Some("typed"));
            }
            other => panic!("unexpected commands: {other:?}"),
        }
    }

    #[test]
    fn an_empty_password_field_means_use_the_stored_one() {
        let (_s, cmds) = reduce(AppState::default(), Input::TestConnection(Box::new(valid_form())));
        assert!(matches!(&cmds[..], [Command::TestConnection { secrets, .. }] if secrets.password.is_none()));
    }

    #[test]
    fn an_invalid_form_sets_an_error_and_sends_nothing() {
        let (state, cmds) = reduce(AppState::default(), Input::TestConnection(Box::default()));
        assert!(cmds.is_empty());
        assert!(state.conn.error.is_some());
        assert_eq!(state.conn.test, TestState::Idle);
    }

    #[test]
    fn only_the_latest_request_result_is_applied() {
        let (s, _) = reduce(AppState::default(), Input::TestConnection(Box::new(valid_form())));
        let (s, _) = reduce(s, Input::TestConnection(Box::new(valid_form())));      // request 2
        let ok = Ok(TestReport { server_version: None, tls: false, authenticated: false });
        let (s, _) = reduce(s, Input::Event(Event::TestConnection { request_id: 1, outcome: ok.clone() }));
        assert!(matches!(s.conn.test, TestState::Running { request_id: 2 }), "stale result ignored");
        let (s, _) = reduce(s, Input::Event(Event::TestConnection { request_id: 2, outcome: ok }));
        assert!(matches!(s.conn.test, TestState::Done { request_id: 2, .. }));
    }

    #[test]
    fn a_test_result_is_shown_even_when_not_connected_to_any_server() {
        // The reducer's "not connected" catch-all must not swallow it.
        let (s, _) = reduce(AppState::default(), Input::TestConnection(Box::new(valid_form())));
        assert_eq!(s.status, ConnStatus::Disconnected);
        let (s, _) = reduce(s, Input::Event(Event::TestConnection { request_id: 1, outcome: Err(CoreError::AuthFailed) }));
        assert!(matches!(s.conn.test, TestState::Done { outcome: Err(CoreError::AuthFailed), .. }));
    }

    #[test]
    fn editing_the_form_after_a_test_clears_the_result() {
        let mut conn = ConnectionsState::default();
        conn.form = valid_form();
        conn.test_form = Some(conn.form.clone());
        conn.test = TestState::Done { request_id: 1, outcome: Ok(TestReport { server_version: None, tls: false, authenticated: false }) };
        conn.form.uri = "mongodb://other".into();
        conn.invalidate_stale_test();
        assert_eq!(conn.test, TestState::Idle);
    }
```

Form tests:

```rust
    #[test]
    fn apply_parsed_fills_fields_and_keeps_the_password_out_of_the_uri() {
        let parsed = parse_uri("mongodb://bob:pw@h/db?authSource=admin&tls=true").unwrap();
        let form = ConnectionForm::default().apply_parsed(parsed);
        assert_eq!(form.uri, "mongodb://h/db");
        assert_eq!(form.username, "bob");
        assert_eq!(form.password, "pw");
        assert_eq!(form.auth_source, "admin");
        assert!(form.tls_enabled);
        assert!(!form.clear_password);
        assert!(form.to_config().is_ok(), "the result must validate: no password in the uri");
    }

    #[test]
    fn apply_parsed_without_a_password_keeps_what_the_user_typed() {
        let mut form = ConnectionForm::default();
        form.password = "typed".into();
        let form = form.apply_parsed(parse_uri("mongodb://bob@h").unwrap());
        assert_eq!(form.password, "typed");
    }
```

Handler tests (`app/mod.rs` or a `connection_uri` test module): `BuildUri { include_password: false }` yields `built_uri` containing `****` and not the typed password; `include_password: true` with an empty password field reads the keychain (`MemorySecretStore` seeded) and yields the real one; the `Debug` output of `ConnectionsState` never contains the password.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-ui test_connection apply_parsed`
Expected: FAIL.

- [ ] **Step 3: Implement**

Create `app/connection_test.rs`:

```rust
//! Test Connection in the reducer: a request counter, a stale-result guard and the typed secrets.
use super::state::AppState;
use crate::viewmodel::ConnectionForm;
use crate::views::connections::{ConnectionsState, TestState};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, TestReport, TestSecrets};
use curlybrace_core::secrets::SecretString;

pub(super) fn start(state: AppState, form: &ConnectionForm) -> (AppState, Vec<Command>) {
    let cfg = match form.to_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            let conn = ConnectionsState {
                error: Some(error),
                ..state.conn
            };
            return (AppState { conn, ..state }, Vec::new());
        }
    };
    let request_id = state.conn.test_seq + 1;
    let secrets = TestSecrets {
        password: (!form.password.is_empty()).then(|| SecretString::new(form.password.clone())),
        ssh_secret: (!form.ssh_secret.is_empty()).then(|| SecretString::new(form.ssh_secret.clone())),
    };
    let conn = ConnectionsState {
        test_seq: request_id,
        test: TestState::Running { request_id },
        test_form: Some(form.clone()),
        error: None,
        ..state.conn
    };
    (
        AppState { conn, ..state },
        vec![Command::TestConnection { request_id, cfg, secrets }],
    )
}

pub(super) fn on_event(
    state: AppState,
    request_id: u64,
    outcome: Result<TestReport, CoreError>,
) -> AppState {
    if !matches!(state.conn.test, TestState::Running { request_id: r } if r == request_id) {
        return state;
    }
    let conn = ConnectionsState {
        test: TestState::Done { request_id, outcome },
        ..state.conn
    };
    AppState { conn, ..state }
}
```

In `reducer.rs` add the input and route it: `Input::TestConnection(form) => connection_test::start(state, &form)`; add the event arm **before** the `_ if state.status != ConnStatus::Connected` catch-all in `on_event`:

```rust
        Event::TestConnection { request_id, outcome } => {
            (connection_test::on_event(state, request_id, outcome), Vec::new())
        }
```

In `views/connections.rs` add `TestState`, the new `ConnectionsState` fields (`test`, `test_seq`, `test_form: Option<ConnectionForm>`, `uri_input: String`, `built_uri: Option<SecretString>`), `invalidate_stale_test`:

```rust
    /// Drops a test result (or a stale built URI) once the form no longer matches what was tested.
    pub fn invalidate_stale_test(&mut self) {
        if self.test_form.as_ref() != Some(&self.form) {
            if matches!(self.test, TestState::Done { .. }) {
                self.test = TestState::Idle;
            }
            self.built_uri = None;
        }
    }
```

and `ConnectionAction::{Test(Box<ConnectionForm>), BuildUri { form: Box<ConnectionForm>, include_password: bool }}`. `ConnectionsState` derives `Debug`: `SecretString`'s redacting `Debug` keeps `built_uri` out of logs. `select`/`start_new` reset `test`, `test_form`, `uri_input`, `built_uri`.

`ConnectionForm::apply_parsed`:

```rust
    /// Applies a parsed connection string. The password goes into the password field (it is
    /// stored in the keychain on save); the string itself is not kept.
    pub fn apply_parsed(self, parsed: ParsedUri) -> Self {
        let has_password = parsed.password.is_some();
        Self {
            uri: parsed.uri,
            username: parsed.username.unwrap_or(self.username),
            auth: parsed.auth.unwrap_or(self.auth),
            auth_source: parsed.auth_source.unwrap_or(self.auth_source),
            tls_enabled: parsed.tls.unwrap_or(self.tls_enabled),
            password: parsed.password.map_or(self.password, |p| p.expose().to_owned()),
            clear_password: self.clear_password && !has_password,
            ..self
        }
    }
```

In `handle_connection_action`:

```rust
            ConnectionAction::Test(form) => {
                self.state = state;
                self.dispatch(Input::TestConnection(form));
            }
            ConnectionAction::BuildUri { form, include_password } => {
                self.state = connection_uri::build(state, &form, include_password, secrets);
            }
```

with `app/connection_uri.rs`:

```rust
//! To URI: builds the string for the form; a real password only on explicit request.
use super::state::AppState;
use crate::viewmodel::ConnectionForm;
use crate::views::connections::ConnectionsState;
use curlybrace_core::secrets::{SecretKind, SecretStore, SecretString, secret_key};
use curlybrace_core::uri::{PasswordMode, build_uri};

pub(super) fn build(
    state: AppState,
    form: &ConnectionForm,
    include_password: bool,
    secrets: &dyn SecretStore,
) -> AppState {
    let cfg = match form.to_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            let conn = ConnectionsState { error: Some(error), ..state.conn };
            return AppState { conn, ..state };
        }
    };
    let typed = (!form.password.is_empty()).then(|| form.password.clone());
    let password = if include_password {
        match typed {
            Some(p) => Ok(Some(p)),
            None => secrets.get(&secret_key(cfg.id, SecretKind::Password)),
        }
    } else {
        Ok(None)
    };
    let built = password.map(|pw| match (include_password, pw.as_deref()) {
        (true, Some(pw)) => build_uri(&cfg, PasswordMode::Include(pw)),
        _ => build_uri(&cfg, PasswordMode::Masked),
    });
    let conn = match built {
        Ok(text) => ConnectionsState {
            built_uri: Some(SecretString::new(text)),
            test_form: Some(form.clone()),
            error: None,
            ..state.conn
        },
        Err(error) => ConnectionsState { error: Some(error), ..state.conn },
    };
    AppState { conn, ..state }
}
```

(`test_form` doubles as the "form this built URI belongs to" snapshot; `invalidate_stale_test` already clears `built_uri` when the form changes. If that coupling reads badly in review, give `built_uri` its own snapshot field instead.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-ui`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): wire Test Connection and From/To URI into the UI state" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: List view-model (filter, sort, group, formatting)

**Files:**
- Create: `crates/ui/src/viewmodel/connection_list.rs`
- Modify: `crates/ui/src/viewmodel/mod.rs` (`pub mod connection_list;` and re-exports in the style of the neighbours)

**Interfaces:**
- Consumes: `ConnectionConfig`, `AuthMechanism`
- Produces:
  - `SortColumn { Name, Server, Security, LastConnected, LastModified }`, `Sort { column: SortColumn, descending: bool }` (`Default`: name ascending); `Sort::toggled(self, column) -> Sort` (same column flips direction, a new column starts ascending)
  - `visible<'a>(list: &'a [ConnectionConfig], filter: &str, sort: Sort) -> Vec<&'a ConnectionConfig>`
  - `Group<'a> { folder: Option<&'a str>, rows: Vec<&'a ConnectionConfig> }`; `grouped<'a>(rows: &[&'a ConnectionConfig]) -> Vec<Group<'a>>` (root first, folders alphabetical, row order preserved)
  - `servers(cfg: &ConnectionConfig) -> String`
  - `Security { tls: bool, ssh: bool, auth: AuthMechanism }`, `security(cfg) -> Security`
  - `format_utc(secs: i64) -> String` (`YYYY-MM-DD HH:MM`, UTC)

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, uri: &str, folder: Option<&str>) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(name, uri);
        c.folder = folder.map(str::to_owned);
        c
    }

    #[test]
    fn filter_matches_name_folder_and_uri_case_insensitively() {
        let list = [
            cfg("Prod", "mongodb://db.example.com", None),
            cfg("Stage", "mongodb://other", Some("Work")),
        ];
        let names = |f: &str| visible(&list, f, Sort::default()).iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("prod"), vec!["Prod"]);
        assert_eq!(names("WORK"), vec!["Stage"]);
        assert_eq!(names("EXAMPLE"), vec!["Prod"]);
        assert_eq!(names("  "), vec!["Prod", "Stage"], "blank filter shows everything");
        assert!(names("nomatch").is_empty());
    }

    #[test]
    fn sorting_by_name_ignores_case_and_can_descend() {
        let list = [cfg("b", "mongodb://h", None), cfg("A", "mongodb://h", None)];
        let asc = visible(&list, "", Sort::default());
        assert_eq!((asc[0].name.as_str(), asc[1].name.as_str()), ("A", "b"));
        let desc = visible(&list, "", Sort { column: SortColumn::Name, descending: true });
        assert_eq!(desc[0].name, "b");
    }

    #[test]
    fn never_connected_sorts_last_ascending_and_first_descending_by_last_connected() {
        let mut seen = cfg("seen", "mongodb://h", None);
        seen.last_connected = Some(10);
        let never = cfg("never", "mongodb://h", None);
        let list = [never, seen];
        let asc = visible(&list, "", Sort { column: SortColumn::LastConnected, descending: false });
        assert_eq!(asc[0].name, "seen");
        assert_eq!(asc[1].name, "never");
    }

    #[test]
    fn toggling_the_same_column_flips_direction_and_a_new_column_resets_it() {
        let s = Sort::default().toggled(SortColumn::Name);
        assert!(s.descending);
        let s = s.toggled(SortColumn::Server);
        assert_eq!((s.column, s.descending), (SortColumn::Server, false));
    }

    #[test]
    fn groups_put_the_root_first_then_folders_alphabetically() {
        let list = [
            cfg("a", "mongodb://h", Some("zeta")),
            cfg("b", "mongodb://h", None),
            cfg("c", "mongodb://h", Some("alpha")),
        ];
        let rows: Vec<_> = list.iter().collect();
        let groups = grouped(&rows);
        let order: Vec<_> = groups.iter().map(|g| g.folder).collect();
        assert_eq!(order, vec![None, Some("alpha"), Some("zeta")]);
    }

    #[test]
    fn servers_lists_every_host_without_credentials_or_options() {
        assert_eq!(servers(&cfg("a", "mongodb://u@h1:27017,h2:27018/db?x=1", None)), "h1:27017, h2:27018");
        assert_eq!(servers(&cfg("a", "mongodb+srv://c0.example.com/db", None)), "c0.example.com");
    }

    #[test]
    fn security_reports_tls_ssh_and_auth() {
        let mut c = cfg("a", "mongodb://h", None);
        c.tls.enabled = true;
        c.auth = AuthMechanism::ScramSha256;
        let s = security(&c);
        assert!(s.tls && !s.ssh);
        assert_eq!(s.auth, AuthMechanism::ScramSha256);
    }

    #[test]
    fn format_utc_is_a_fixed_width_iso_like_stamp() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p curlybrace-ui connection_list`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Pure view-model for the connection table: filtering, sorting, grouping and display text.
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Server,
    Security,
    LastConnected,
    LastModified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sort {
    pub column: SortColumn,
    pub descending: bool,
}

impl Sort {
    pub fn toggled(self, column: SortColumn) -> Self {
        if self.column == column {
            Self { descending: !self.descending, ..self }
        } else {
            Self { column, descending: false }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Security {
    pub tls: bool,
    pub ssh: bool,
    pub auth: AuthMechanism,
}

pub fn security(cfg: &ConnectionConfig) -> Security {
    Security {
        tls: cfg.tls.enabled,
        ssh: cfg.ssh.is_some(),
        auth: cfg.auth,
    }
}

fn matches(cfg: &ConnectionConfig, needle: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(needle);
    has(&cfg.name) || has(&cfg.uri) || cfg.folder.as_deref().is_some_and(has)
}

/// Option ordering where `None` sorts after every `Some` when ascending.
fn recent_first(a: Option<i64>, b: Option<i64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y).reverse(),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare(a: &ConnectionConfig, b: &ConnectionConfig, column: SortColumn) -> Ordering {
    let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
    match column {
        SortColumn::Name => by_name(),
        SortColumn::Server => servers(a).to_lowercase().cmp(&servers(b).to_lowercase()).then_with(by_name),
        SortColumn::Security => {
            let key = |c: &ConnectionConfig| (c.tls.enabled, c.ssh.is_some(), c.auth != AuthMechanism::None);
            key(a).cmp(&key(b)).then_with(by_name)
        }
        SortColumn::LastConnected => recent_first(a.last_connected, b.last_connected).then_with(by_name),
        SortColumn::LastModified => recent_first(a.updated_at, b.updated_at).then_with(by_name),
    }
}

pub fn visible<'a>(list: &'a [ConnectionConfig], filter: &str, sort: Sort) -> Vec<&'a ConnectionConfig> {
    let needle = filter.trim().to_lowercase();
    let mut rows: Vec<&ConnectionConfig> = list
        .iter()
        .filter(|c| needle.is_empty() || matches(c, &needle))
        .collect();
    rows.sort_by(|a, b| {
        let ord = compare(a, b, sort.column);
        if sort.descending { ord.reverse() } else { ord }
    });
    rows
}

pub struct Group<'a> {
    pub folder: Option<&'a str>,
    pub rows: Vec<&'a ConnectionConfig>,
}

pub fn grouped<'a>(rows: &[&'a ConnectionConfig]) -> Vec<Group<'a>> {
    let mut folders: Vec<&str> = rows.iter().filter_map(|c| c.folder.as_deref()).collect();
    folders.sort_unstable();
    folders.dedup();
    let mut groups = vec![Group {
        folder: None,
        rows: rows.iter().copied().filter(|c| c.folder.is_none()).collect(),
    }];
    groups.extend(folders.into_iter().map(|f| Group {
        folder: Some(f),
        rows: rows.iter().copied().filter(|c| c.folder.as_deref() == Some(f)).collect(),
    }));
    groups.retain(|g| !g.rows.is_empty());
    groups
}

/// The hosts of the URI, without credentials, path or options.
pub fn servers(cfg: &ConnectionConfig) -> String {
    let rest = cfg
        .uri
        .strip_prefix("mongodb+srv://")
        .or_else(|| cfg.uri.strip_prefix("mongodb://"))
        .unwrap_or(&cfg.uri);
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let hosts = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    hosts.split(',').collect::<Vec<_>>().join(", ")
}

/// `YYYY-MM-DD HH:MM` in UTC (civil-from-days, valid for the whole i64 range we can show).
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m) = (rem / 3600, rem % 3600 / 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{d:02} {h:02}:{m:02}")
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p curlybrace-ui connection_list`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates
git commit -m "feat(connections): add the list view-model for filter, sort and folder grouping" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Manager screen, edit modal, i18n

**Files:**
- Create: `crates/ui/src/views/connections_list.rs`, `crates/ui/src/views/connections_edit.rs`
- Modify: `crates/ui/src/views/connections.rs` (state + `show_connections` orchestration only; move `show_form`, `secret_row`, `tls_section`, `ssh_section`, `auth_combo`, `text_row` into `connections_edit.rs` unchanged first, as a pure move commit-able on its own), `crates/ui/src/views/mod.rs`, `crates/ui/locales/en.json`, `crates/ui/src/i18n_usage_tests.rs` (`PREFIXES` gains `"connections."`), `docs/superpowers/specs/2026-10-14-connection-manager-design.md` (record the deviation)

**Interfaces:**
- Consumes: Tasks 4, 5, 8, 9.
- Produces: `show_connections(ui, state, i18n) -> Option<ConnectionAction>` keeps its signature. New state: `ConnectionsState { filter: String, sort: Sort, editing: bool, folder_prompt: Option<FolderPrompt> }` and `FolderPrompt { Move { id: Uuid, input: String }, Rename { from: String, input: String } }`.

**New en.json keys** (add inside `"connections"`, **alphabetical position**; `connections.test` and every existing key stay):
`built_uri`, `cancel_edit` (use existing `common.cancel` instead if present), `clear_filter`, `col_last_connected`, `col_last_modified`, `col_name`, `col_security`, `col_server`, `copy_name` (`"{name} copy"`), `delete_folder`, `duplicate`, `edit`, `edit_title`, `export`, `filter`, `filter_count` (`"{shown} of {total}"`), `folder`, `folder_move_title`, `folder_rename_title`, `from_uri`, `from_uri_hint`, `import`, `import_done` (`"Imported {imported}, skipped {skipped}."`), `export_done` (`"Exported {count} connections without secrets."`), `include_password`, `move_to_folder`, `never`, `no_match`, `rename_folder`, `root_group`, `security_none`, `security_ssh`, `security_tls`, `test_ok` (`"Connected. Server version {version}."`), `test_ok_no_version`, `test_running`, `to_uri`, `tls_warning_any_cert` (reuse `allow_invalid_certs_ssh_warning` pattern; add `allow_invalid_certs_warning` for the non-SSH case), `uri_password_warning` (`"This string contains your password. Do not share it."`). Run the alphabetical test after editing; fix ordering, not the test.

- [ ] **Step 1: Pure move.** Move the form drawing code into `connections_edit.rs` with no behaviour change. Run `cargo test -p curlybrace-ui` (snapshots stay `#[ignore]`d). Commit: `refactor(connections): move the connection form into its own view file`.

- [ ] **Step 2: Failing tests.** Add to `tests/accessibility.rs` style (read the neighbouring accessibility tests for the harness) a test that renders the screen with three connections (one in a folder) and asserts, via the accesskit tree, that: the filter box has a label, each table header is a button with its text, each row exposes its name, the toolbar buttons are labelled, and the edit modal's text fields all have labels. Add a view-state unit test in `views/connections.rs`:

```rust
    #[test]
    fn opening_new_or_edit_opens_the_modal_and_save_closes_it() {
        let mut s = ConnectionsState::new(vec![ConnectionConfig::new("a", "mongodb://h")]);
        s.start_new();
        assert!(s.editing);
        s.editing = false;
        let cfg = s.connections[0].clone();
        s.select(&cfg);
        s.begin_edit();
        assert!(s.editing && s.form.name == "a");
        s.saved();
        assert!(!s.editing);
    }
```

Run: `cargo test -p curlybrace-ui` — Expected: FAIL.

- [ ] **Step 3: Implement the list** (`connections_list.rs`). The function draws, in order: heading; a toolbar row (`New`, `Edit`, `Duplicate`, `Delete`, `Import`, `Export`, `Move to folder`; selection-dependent buttons are disabled with `ui.add_enabled`); the filter `TextEdit::singleline` with `hint_text(i18n.t("connections.filter"))` and a `filter_count` label using `visible(...)` length; then an `egui::Grid` with clickable header cells (`ui.button(label)` that calls `state.sort = state.sort.toggled(column)`; show an arrow glyph `▲`/`▼` only via the `Sort`, no text) and, per `grouped(&rows)` group, a `CollapsingHeader` (id from the folder name; the root group uses `i18n.t("connections.root_group")` and no collapse) containing one row per connection: name as `selectable_label` (click selects, `double_clicked()` returns `ConnectionAction::Connect(id)`), servers, security text built from `security(cfg)` with `security_tls`/`security_ssh`/`auth_label_key(auth)` keys joined by `", "` or `security_none`, last connected via `format_utc` or `connections.never`, last modified likewise. Folder headers get `Rename` and `Delete folder` small buttons (`FolderPrompt::Rename`, `ConnectionAction::DeleteFolder`). Render `state.notice` with `import_done`/`export_done` through `i18n.t_with`. Empty list shows `connections.no_match` when the filter hides everything, else nothing.

- [ ] **Step 4: Implement the edit modal** (`connections_edit.rs`). `egui::Modal::new(Id::new("connection_edit")).show(ctx, ...)` containing: name, folder (text + a `ComboBox` of `folder_names` that fills it), the From URI row (`state.uri_input` text field + `from_uri` button: on click `match parse_uri(&state.uri_input) { Ok(p) => { state.form = state.form.clone().apply_parsed(p); state.uri_input.clear(); state.error = None } Err(e) => state.error = Some(e) }`), then the moved form body, then a **Test connection** button (`connections.test`, returns `ConnectionAction::Test(Box::new(state.form.clone()))`) and the result label (`TestState::Running` → `test_running`; `Done(Ok(r))` → `test_ok` with `{version}` or `test_ok_no_version`; `Done(Err(e))` → `super::error_label::show_error`), then **To URI** (`to_uri` button emits `BuildUri { include_password: false }`, `include_password` button emits `BuildUri { include_password: true }`; when `built_uri` is `Some`, show it in a read-only `TextEdit::singleline` plus a Copy button using `ui.ctx().copy_text(...)`, and when the shown string came from `include_password: true` also show `uri_password_warning`; the view tracks which one with a `built_with_password: bool` set when the action is emitted). Footer: Cancel (`state.editing = false`) and Save (existing behaviour; on `ConnectionAction::Save` the handler's successful save sets `editing = false` through `ConnectionsState::saved()`, called from `connections::save`'s success path). Warnings: when `form.allow_invalid_certs` show `connections.allow_invalid_certs_warning` (or the SSH variant when `ssh_enabled`) as a coloured label, using `ui.visuals().warn_fg_color`.

- [ ] **Step 5: Wire the screen.** `show_connections` calls `invalidate_stale_test()`, draws the list, the edit modal when `editing`, the folder prompt modal (a single text field + OK/Cancel emitting `MoveToFolder`/`RenameFolder`, with the existing folder names as buttons for Move), and the existing delete-confirm modal. `begin_edit`, `start_new`, `saved` set/clear `editing`. Double-click or the `Connect` footer button emit `Connect`.

- [ ] **Step 6: i18n.** Add all keys; add `"connections."` to `PREFIXES` in `i18n_usage_tests.rs`; run `cargo test -p curlybrace-ui i18n` and fix any unused or missing key. The existing `connections.test` key is now used.

- [ ] **Step 7: Spec update.** In the spec's "Manager UI" section replace "A modal dialog replaces the inline list" with "The connections screen (shown whenever the app is not connected) becomes the manager list; only the edit form is a modal." Keep the rest.

- [ ] **Step 8: Run tests**

Run: `cargo test -p curlybrace-ui`
Expected: PASS. Then run the app (`cargo run -p curlybrace`) and click through: new, save, duplicate, move to a folder, rename the folder, filter, sort each column, import/export, test, From URI, To URI, connect, restart, confirm persistence.

- [ ] **Step 9: Gates and commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add -A crates docs
git commit -m "feat(connections): manager screen with filter, sort, folders and an edit dialog" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Wiring, snapshots, accessibility, secrets audit, follow-ups, review

**Files:**
- Modify: `crates/ui/tests/wiring.rs`, `crates/ui/tests/snapshots.rs` (+ new PNGs), `crates/ui/tests/accessibility*.rs`, `crates/core/src/test_support/mock_objects.rs`
- Create: GitHub issues (via `gh`, only with the maintainer's go-ahead; draft the text first)

- [ ] **Step 1: Shared fixtures.** Add to `mock_objects.rs`: `mock_connection_in_folder(name, folder) -> ConnectionConfig` and `mock_connections()` returning three connections (root, two in one folder, one with TLS+SSH). Use them in the new tests instead of ad-hoc literals.

- [ ] **Step 2: Wiring tests** (`tests/wiring.rs`, real runtime, fake connector, `MemorySecretStore`, tempdir config path):
  - duplicate through `UiEvent::Connection(Duplicate)` gives two rows, copied secret present, file contains no secret;
  - test connection through the runtime while connected leaves the browse session usable;
  - export then import in a fresh app yields the same names with new ids;
  - restart (build a second app on the same config path) keeps folders, timestamps and `last_connected`.

- [ ] **Step 3: Snapshots.** Add `connection_manager_list` (three connections, one folder, sorted), `connection_manager_filtered`, `connection_edit_dialog` (with a successful test result), using the `shot!` macro. Generate with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored`, then **look at each PNG** and confirm no password or secret text is visible. The two existing snapshots will change; review and update them.

- [ ] **Step 4: Secrets audit.** Add a test that serializes `mock_connections()`, the export bytes and `format!("{:?}", state.conn)` of a state holding a typed password, a built URI and a test result, and asserts none contains the marker password string used in the fixtures. Run `grep -rn "password" crates/ui/tests/snapshots` is not applicable to PNGs: rely on viewing them.

- [ ] **Step 5: Run everything.**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
scripts/check-no-db-in-ci.sh
```

Expected: all green. If docker is available also run `scripts/container-tests.sh` for the Task 7 integration test.

- [ ] **Step 6: Review loop.** Run `/code-review` on the branch (`git diff main...HEAD`). Fix every CRITICAL/HIGH finding, then re-run the review on the fix, and repeat until a full pass finds nothing new. After any cross-cutting change (for example the `open_tunnel_if_configured` signature, `save`'s `now`, the new `Event` variants) re-walk every call site and state what it means there. Specifically re-check the seven items under Review Focus.

- [ ] **Step 7: Follow-up issues.** Draft, then (with the maintainer's approval) create issues linked to #45 for: simultaneous sessions (section 4), copy between connections (5), shell and tools settings (6), nested and empty folders (versioned envelope), remaining edit tabs (Proxy, Advanced, History), "show on startup" and recent connections on a start screen, export of secrets as an explicit opt-in. Use the repo's label conventions (`enhancement`, `source:internal`, `ui`/`core`, a `Fix with ...` label).

- [ ] **Step 8: PR.** Push with `git push -u origin feat/connection-manager-slice-1` and open a PR against the integration branch (not `main` directly if the repo uses one; otherwise confirm the target with the maintainer first). Body: what and why, the new i18n keys, a **Security** section (credential storage, TLS trust, SSH host-key trust touched; no secrets in exports/logs/snapshots; import cannot clobber ids), a test plan with checkboxes, and `Refs #45`. End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

---

## Self-Review

**Spec coverage:** data model and migration (Task 1); folder helpers (2); secrets, duplicate rollback (3, 4); import/export (5); From/To URI (6, 8, 10); Test Connection (7, 8, 10); manager UI, filter, sort, folders, edit dialog (9, 10); `last_connected` stamp (4); i18n and prefix extension (10); security notes and audit (11); testing list from the spec (1 to 11); follow-ups (11). Spec items intentionally not built: simultaneous sessions, copy between connections, shell/tools, nested/empty folders, exporting secrets, "show on startup" (no settings store was found; tracked as a follow-up).

**Placeholder scan:** the UI rendering steps in Task 10 (3 to 5) describe widgets in prose rather than full egui code, because they are mechanical compositions of the exact view-model calls defined in Tasks 8 and 9 and the existing patterns in `views/connections.rs` (`text_row`, `secret_row`, the `egui::Modal` delete confirm). Every function they call has a defined signature above. All logic with branching (filter/sort/group, parse/build, copy/rollback, import validation, reducer guards) has full code and tests.

**Type consistency:** `save(..., now)` (Task 1) is the call used in Task 4's handler; `SecretKind::ALL` (Task 3) replaces `ALL_KINDS` in Tasks 3 and 4; `TestState`/`test_seq`/`test_form` (Task 8) are the names used in the Task 8 tests and Task 10; `Notice` is defined in Task 5 and rendered in Task 10; `PasswordMode`, `parse_uri`, `build_uri`, `ParsedUri` (Task 6) are used with those names in Task 8.
