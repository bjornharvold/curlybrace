# Core Browser — Design (Sub-project 1)

Date: 2026-10-12
Status: Draft, awaiting user review

## Context and intent

A free, open-source desktop MongoDB IDE written in Rust, with the feature set of commercial tools such as
Studio 3T. Built as a new program from public documentation, observable behavior and MongoDB's public
docs/wire protocol. **No decompilation or porting of any proprietary binary.** No license manager; every
feature is free.

The full feature set is split into sub-projects, each with its own spec -> plan -> implementation cycle.
This document covers sub-project 1 only. Later sub-projects (out of scope here): visual query builder,
IntelliShell, aggregation editor, import/export, backup/restore, schema explorer, data compare/sync,
SQL-to-Mongo, index management, user/role management.

Platform: macOS only. UI toolkit: egui (pure Rust).

## Scope

In scope:
- Saved connections: URI parsing, TLS, replica sets, SRV.
- Auth: SCRAM, X.509, Kerberos (GSSAPI, best-effort), LDAP (PLAIN).
- SSH tunnel (password or key).
- Sidebar listing databases and collections (lazy-loaded).
- Document views: tree, table, JSON, with pagination (50 per page).
- Find bar: filter, sort, projection, skip, limit.
- Document CRUD: insert, edit (inline and JSON), delete, clone, with BSON type-aware editing.
- All user-facing strings resolved through i18n keys (hand-edited `en.json`, nested objects).

Out of scope: everything listed under later sub-projects; Windows/Linux builds.

## Architecture

Cargo workspace:

| Crate | Responsibility | Depends on |
|-------|----------------|------------|
| `mongostudio-core` | `mongodb` driver wrapper, connection model, auth config, SSH tunnel (`russh`), secret storage, `CoreError` | no UI crates |
| `mongostudio-ui` | egui views and widgets, i18n layer | `core` |
| `mongostudio` (bin) | app state, wiring, runtime startup | `core`, `ui` |

`core` has no egui dependency so later sub-projects stay reusable and testable headless.

The driver is async. A background Tokio runtime hosts a command handler; the UI sends commands and
receives events over `mpsc` channels. The UI thread never blocks. Each event requests a repaint.

## Data flow

1. Open connection: build tunnel (if configured), build client from the connection model, ping.
2. Sidebar: `list_databases`, then `list_collection_names` on expand.
3. Query: `find` with the cursor, paged 50 at a time. Results held as immutable `bson::Document` values.
4. Views render the same page data (tree / table / JSON).
5. Edit: Document -> relaxed extended JSON -> user edit -> parse and type-validate -> `replace_one`
   (filtered by `_id`) or `insert_one`. Delete uses `delete_one` by `_id`. Clone = insert with a new `_id`.
   State updates return new values; no in-place mutation.

## Security and errors

- Passwords, SSH keys and passphrases are stored in the macOS Keychain. Connection config on disk holds
  only references. Secrets never appear in logs or error messages.
- All user input (URI, JSON filter, edited documents, tunnel settings) is validated at the boundary with
  clear errors.
- Every failure maps to a typed `CoreError`; the UI shows a localized message keyed off the variant and
  logs the detail. Nothing is silently swallowed.
- SSH host keys are verified against `known_hosts`; unknown hosts prompt the user.

## Known risk: Kerberos

The Rust driver's GSSAPI support depends on a system GSSAPI library and may be incomplete. Plan: gate it
behind a cargo feature `gssapi`, verify feasibility in a spike at the start of planning, and fall back to
documenting it as unsupported if the driver cannot do it. SCRAM, X.509 and LDAP are not affected.

## Testing

- TDD (red/green/refactor), coverage target >= 80% for `core`.
- Shared fixtures in a `mock_objects` module in `core` exposing real `bson` documents and connection
  models; real objects over mocks. Mocks only at boundaries (driver, SSH, keychain traits).
- Integration tests against a MongoDB container (plain, auth, TLS variants) and a local SSH server
  container for the tunnel.
- UI logic (paging, view-model conversion, edit validation) kept in plain functions and unit-tested
  without egui.

## Open questions

- Project name and license (placeholder crate prefix `mongostudio`).
- Whether tunnel credentials can reuse the system `ssh-agent`.
