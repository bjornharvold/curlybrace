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
| 2 (built) | Several simultaneous sessions: a tree root and view per connection, status (the read-only flag moved to #47) | 4 |
| 3 | Copy data between connections, Data Compare source/target from open connections | 5 |
| 4 | Shell and tools settings (global and per-connection) | 6 |
| later | Remaining edit tabs: Proxy, Advanced, History | 3 |

Slice 1 and slice 2 are built; slice 2 is detailed in `2026-10-14-multiple-sessions-design.md`. Slices 3 and 4 become linked follow-up issues. The single active browsing session is unchanged in slice 1.

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
| `folder` | `Option<String>` | Trimmed, non-empty, at most 80 chars, no control or invisible format characters, else `None` |
| `created_at` | `Option<i64>` | Unix seconds, set on first save |
| `updated_at` | `Option<i64>` | Unix seconds, set on every save |
| `last_connected` | `Option<i64>` | Unix seconds, set when a connect succeeds |

`ConnectionForm` carries these through an edit, so `to_config()` does not drop them.

Pure, immutable helpers in core: `duplicate`, `folders`, `rename_folder`, `delete_folder` (a delete moves
the folder's connections to the root; it never deletes connections).

### Migration

No file migration. Old entries lack the new fields and deserialize to `None`.

Releases up to v0.2.0 accepted any non-blank name, while `validate()` is now strict (name at most 200
chars, no control or invisible format characters such as U+200D). New input (the edit dialog, import) is
strict. Entries LOADED from the user's own file are repaired first (`repair_labels`): forbidden
characters removed, labels trimmed and cut to their limits, a blank name replaced by the URI host (then
`connection-<id prefix>`), an emptied folder dropped. The id is kept, so keychain entries stay attached.
Only an entry that still fails `validate()` after the repair (for example a bad URI) is skipped. Any
repair or skip backs the original file up before anything can overwrite it (a failed backup turns saving
off) and shows a banner; a repair banner says names were adjusted, never that entries were lost.

## Secrets and Duplicate

Secrets are keyed `conn:{id}:{kind}`. Duplicate gets a new id, then copies the three secret kinds with
`get` then `set`. If any step fails, the new secrets are removed and the new config is not added, so a
duplicate is all or nothing. Delete keeps its current order: secrets first, the config only if all succeed.

## Import and Export

- **Export** writes the same JSON array as `connections.json`. It has no password field by construction.
- **Import** runs every entry through the tolerant loader (`validate()`), assigns a fresh id, clears
  `last_connected`, and reports imported and skipped counts. Import is strict: an entry with an invalid
  name or folder is skipped on import (and repaired only when loaded from the user's own saved file). An import can never overwrite an existing
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

A new `Command::TestConnection { request_id, cfg, secrets }` and `Event::TestConnection { request_id,
outcome }`. The runtime reuses validation, the keychain read, the optional SSH tunnel and
`client::connect`, then drops the client and the tunnel. `outcome` is a `Result<TestReport, CoreError>`;
`TestReport` carries the server version and the `tls` and `authenticated` flags, which mean "configured",
not "verified". A failure carries a typed error that maps to an i18n key. Typed secrets travel in
`TestSecrets`; the password is a `PasswordChoice` (`Stored`, `Without` when the user cleared the saved
password, or `Typed`). Latest request wins, like the connect epoch. SSH host-key verification keeps
failing closed.

## Manager UI

- The connections screen (shown whenever the app is not connected) becomes the manager list; only
  the edit form is a modal. Toolbar: New, Edit, Duplicate, Delete, Import, Export, Move to folder.
  Filter box with a match count. Sortable table: name, hosts, security summary, last connected, last
  modified. Folders as collapsible groups. Each row has a Connect button, and double-click connects.
  There is no Close button: the list is the home screen.
- Edit dialog: name, folder, URI with From URI, authentication, TLS, SSH, Test Connection, To URI.
  It closes on Cancel or Escape only; a click outside never discards a typed form.
  Test Connection reports what is configured (TLS on, credentials supplied), not what was
  verified; the wording says "configured" and never "verified".
- `ConnectionAction` gains Duplicate, MoveToFolder, RenameFolder, DeleteFolder, StartImport,
  StartExport, Test and BuildUri. These are impure and go through `handle_connection_action`, like
  Save and Delete. From URI is a local view action: it parses the pasted text into the form and needs
  no handler.
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
