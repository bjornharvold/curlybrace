# Connection Manager: Design

Date: 2026-10-14
Status: Slice 1 scope and the folder decision approved by the maintainer on 2026-10-14
Issue: #45. Related: #38 (parity map), #8 (Data Compare & Sync), #13 (ssh-agent), #17, #18, #21.

## Context and intent

One place to create, edit, organise and securely store many MongoDB connections, and later to keep several
of them connected at once. Being connected to more than one server is the prerequisite for copying data
between instances.

The feature is designed as a whole in the issue but **built in slices**. This note details slice 1 and
records what the later slices must not break.

| Slice | Delivers | Issue sections |
|---|---|---|
| 1 | Saved-connection store (folders, timestamps), manager dialog, Test Connection, From/To URI, import/export | 1, 2, 7 |
| 2 | Several simultaneous sessions: a tree root and tabs per connection, status, read-only flag | 4 |
| 3 | Copy data between connections, Data Compare source/target from open connections | 5 |
| 4 | Shell and tools settings (global and per-connection) | 6 |
| later | Remaining edit tabs: Proxy, Advanced, History | 3 |

Slices 2 to 4 become linked follow-up issues. The single active browsing session is unchanged in slice 1.

## Decisions

1. **Folders** are a field on each connection (`folder: Option<String>`), one flat level. A folder exists
   only while it has connections. Empty and nested folders need a versioned file envelope and are a
   follow-up; the array file format is kept so old files and import/export stay trivial.
2. **Export never contains secrets** in slice 1; there is no opt-in. Import always assigns fresh ids.
3. **Test Connection** opens no session and installs no store.
4. **Secrets** stay in the keychain, keyed by connection id. The connection file stays secret-free.

## Data model

`ConnectionConfig` (`crates/core/src/connection.rs`) gains, all `#[serde(default)]` so older files load:

| Field | Type | Meaning |
|---|---|---|
| `folder` | `Option<String>` | Trimmed, non-empty, at most 80 chars, no control characters, else `None` |
| `created_at` | `Option<i64>` | Unix seconds, set on first save |
| `updated_at` | `Option<i64>` | Unix seconds, set on every save |
| `last_connected` | `Option<i64>` | Unix seconds, set when a connect succeeds |

`ConnectionForm` carries these through an edit, so `to_config()` does not drop them.

Pure, immutable helpers in core: `duplicate`, `folders`, `rename_folder`, `delete_folder` (a delete moves
the folder's connections to the root; it never deletes connections).

### Migration

No file migration. Old entries lack the new fields and deserialize to `None`. The loader's per-entry
tolerance, backup and `PersistGuard` behaviour are unchanged.

## Secrets and Duplicate

Secrets are keyed `conn:{id}:{kind}`. Duplicate gets a new id, then copies the three secret kinds with
`get` then `set`. If any step fails, the new secrets are removed and the new config is not added, so a
duplicate is all or nothing. Delete keeps its current order: secrets first, the config only if all succeed.

## Import and Export

- **Export** writes the same JSON array as `connections.json`. It has no password field by construction.
- **Import** runs every entry through the tolerant loader (`validate()`), assigns a fresh id, clears
  `last_connected`, and reports imported and skipped counts. An import can never overwrite an existing
  connection or inherit another connection's keychain entries.
- Paths come from the existing file picker.

## From URI and To URI

- **From URI**: a pure parser with typed errors. It accepts `mongodb://` and `mongodb+srv://`, takes the
  username, `authSource` and `authMechanism` into form fields, and keeps hosts and options in `uri`. The
  password goes to the secret store. The pasted string is not kept, and error text never quotes it.
- **To URI**: built from the form with percent-encoding. The password is masked by default; "include
  password" is an explicit action that reads the keychain and is never persisted.
- Open point: a synchronous parser versus the async `ClientOptions::parse`. Decided by the first failing
  test; the parser must not echo input in errors either way.

## Test Connection

A new `Command::TestConnection` and `Event::TestConnectionResult { request_id, result }`. The runtime
reuses validation, the keychain read, the optional SSH tunnel and `client::connect`, then drops the client
and the tunnel. The result is typed: reachable, auth ok, TLS ok, server version, or a failure carrying an
i18n key. Latest request wins, like the connect epoch. SSH host-key verification keeps failing closed.

## Manager UI

- The connections screen (shown whenever the app is not connected) becomes the manager list; only
  the edit form is a modal. Toolbar: New, Edit, Duplicate,
  Delete, Import, Export, To URI. Filter box with a match count. Sortable table: name, hosts, security
  summary, last connected, last modified. Folders as collapsible groups. Footer: Close, Connect.
  Double-click connects.
- Edit dialog: name, folder, URI with From URI, authentication, TLS, SSH, Test Connection, To URI.
  Test Connection reports what is configured (TLS on, credentials supplied), not what was
  verified; the wording says "configured" and never "verified".
- `ConnectionAction` gains Duplicate, MoveToFolder, RenameFolder, DeleteFolder, Import, Export,
  TestConnection, FromUri and ToUri. These are impure and go through `handle_connection_action`, like
  Save and Delete.
- `last_connected` is stamped when `Event::Connected` arrives, and persisted.
- Every string is an i18n key in `en.json`. The unused-key check is extended to the `connections.` prefix.
- "Accept any certificate" and "allow invalid hostnames" keep a visible warning.

## Security

This touches credential storage, TLS trust and SSH host-key trust, so the PR calls it out (SECURITY.md).
No URI, password or passphrase is logged, redacted in errors, or written to an export. Fixtures and
snapshots contain no secrets.

## Testing

Core unit tests for the model, folder helpers, duplicate rollback and URI round trips. Reducer and wiring
tests with `MemorySecretStore` and a tempdir config path. Snapshot tests for the manager and edit dialogs.
Accessibility and i18n tests. An integration test of Test Connection against a real MongoDB container.
Shared fixtures extend `mock_objects.rs`.

## Out of scope for slice 1

Simultaneous sessions, a tree root per connection, read-only flags, copy between connections, shell and
tool executables, Proxy, Advanced and History tabs, nested or empty folders, exporting secrets.
