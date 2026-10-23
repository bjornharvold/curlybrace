# Read-only connections — design

Issue: #109 (split out of #47; follows PR #108 colour tags and production flag).

## Intent

A user can save a connection as **read-only**. On such a connection only read
operations are possible. It is the "ultra safe" way to connect to a production
database: unlike the production flag (#108), which only warns and lets the user
confirm, read-only **refuses** every write, and the refusal lives in the core so
a UI bug cannot bypass it.

## Goals

- A `read_only` flag on the saved connection, edited in the connection dialog
  next to the production checkbox. Persists; old connection files still load.
- The core refuses every write on a read-only connection with a typed error
  (`CoreError::ReadOnly`), before anything reaches the database.
- The UI hides or disables write actions on a read-only connection and says why.
- A text/shape marker for read-only wherever the connection is named (colour is
  never the only signal). Every user-facing string is an i18n key in `en.json`.
- Ticking read-only on an **open** session takes effect immediately.

## Non-goals

- Server-side enforcement (a read-only database user). This is a client guard.
- Writes that do not exist yet (import, drop, rename, index changes, bulk edit).
  The guard is built so those are covered when added (see Core enforcement), but
  no such commands are added here.
- Blocking local file exports from Data Compare (they do not touch the database).

## Anti-goals

- A read-only connection that still performs a write, by any path, including
  Data Compare apply.
- A refusal that surfaces as an "unknown write outcome" or "unreachable write"
  error. The refusal happens before the store call and is reported as read-only.

## Decisions (made with the user)

- Enforced in the core, not only the UI.
- Ticking read-only on an open session applies **immediately**, via a new core
  command that updates the live session's flag. Unticking applies on reconnect,
  matching #108's production behaviour.
- In Data Compare a read-only connection can be compared as the **source**
  (it is read), but nothing is ever copied into it: it cannot be chosen as the
  **target**, and every apply is judged by the side each operation writes to, so
  "Copy to source", "Delete from source" and field copies to the source are
  refused when the source is read-only. Source/target is not the rule; the
  destination of each write is.

## Current state (from code reading, 2026-10-23)

- `ConnectionConfig` (`crates/core/src/connection.rs`) already carries
  `production: bool` with a lenient deserializer that loads a damaged value as
  `true`, and `color`. `read_only` follows the same pattern.
- The only database writes are `Insert`, `Replace`, `Delete`, `Clone` (all via
  `runtime/handlers.rs::data_command` -> `DocumentService` -> `DocumentStore`)
  and `Compare Apply` (`runtime/compare/apply.rs` -> `store.bulk_write`).
  `Command::is_write()` already classifies the first four.
- No raw command runner, shell or user-typed aggregation exists; the one
  aggregation is internal with no `$out`/`$merge`. No hidden read-that-writes.
- The UI treats a session as production if its snapshot at connect OR the live
  saved flag says so (`ui/src/app/write_guard.rs::session_production`). The core
  only sees the `ConnectionConfig` passed to `Connect`.
- Typed errors are `CoreError` (`error.rs`), each with an `i18n_key()` and a
  distinct-key unit test; failures reach the UI as `Event::Failed` /
  `CompareEvent::Failed`.

## Design

### Model
- Add `read_only: bool` to `ConnectionConfig`. A missing field loads as `false`
  (old files). A present-but-damaged value loads as `true` (fail safe) and counts
  as "repaired" in the startup banner, mirroring `lenient_production`.
- Dialog: checkbox + hint next to production, via the connection form viewmodel.
  Read-only and production may both be set.

### Core enforcement
- Wrap the `DocumentStore` in a `ReadOnlyStore` that rejects `insert`, `replace`,
  `delete` and `bulk_write` with `CoreError::ReadOnly`. This is the single choke
  point; any future write method added to the trait must be rejected there (the
  trait should make that hard to forget, e.g. a test that fails when a new
  write method is not covered — builder's call on mechanism).
- The read-only state is **live and shared**, not fixed at construction, so the
  new update command can flip it for an open session without reconnecting. The
  session holds the flag; the wrapper reads it on every write.
- Also check early in `handlers::run` (via `Command::is_write()`) and in
  `compare::apply::start_apply`, so the refusal is reported with the right
  context before any "pending write" bookkeeping starts.
- New runtime command `SetReadOnly { session, read_only }` updates the flag for a
  live session (and for a compare session's stores). Setting it to `true` is
  honoured at once; the UI only sends it when the user ticks the box.
- Compare: the compare session's stores are wrapped the same way. A read-only
  connection can be opened as source; an `Apply` with any operation (plain,
  field or expanded bulk) whose destination side is read-only is refused up
  front with `ReadOnly`, before the job slot is taken. A lock that lands
  mid-apply makes the store refuse the remaining batches, reported as
  `ReadOnly` (nothing was written), not as an unknown outcome.

### UI
- Session read-only = snapshot at connect OR live saved flag (same rule as
  `session_production`), so the UI locks at once on tick.
- Edit, clone, delete, insert and compare apply are hidden or disabled with a
  stated reason; a banner modelled on `Banner::ProjectionReadOnly` explains it.
  If a write somehow arrives anyway, the `ReadOnly` failure is shown with its own
  localized message (`errors.read_only`).
- Compare target picker excludes read-only connections and says why. Review,
  Apply and the queue actions judge every destination the changes write to: a
  queued change into a read-only source closes Review and Apply with the reason,
  and "Copy to source" / "Copy selected fields to source" / "Delete from source"
  are disabled with the reason; pending lines into a read-only connection carry
  the badge.
- Sending the live lock also raises the session's read-only snapshot (active,
  parked and compare sides), so unticking afterwards keeps the UI locked like
  the core until a reconnect.
- Marker: a text badge ("Read-only") next to the production badge wherever the
  connection is named (list, session tab/header, compare), never colour alone.
- Saving the dialog with the box newly ticked while the connection is open sends
  `SetReadOnly`.

### i18n
All new strings are keys in `ui/locales/en.json` (nested): dialog label and hint,
badge, disabled-action reason, banner, compare-target reason, `errors.read_only`,
repaired-flag banner text. Keys under existing prefixes so `i18n_usage_tests`
stays satisfied. New keys are reported in the PR.

## Testing

TDD, coverage >= 80%.
- Core unit: `ReadOnlyStore` rejects each write method and passes reads; flag flip
  on a live store; `CoreError::ReadOnly` has a distinct i18n key.
- Core runtime: every write command (`Insert`, `Replace`, `Delete`, `Clone`,
  `Compare Apply`) on a read-only session returns `ReadOnly` and never reaches the
  store; `SetReadOnly` flips an open session; reads still work.
- Model: old file loads `false`; damaged value loads `true` and is reported;
  round-trip and duplicate keep the flag.
- UI: viewmodel and dialog tests; accessibility tests (accesskit roles/labels) for
  the checkbox, badge, disabled actions and banner; compare target exclusion;
  snapshot-vs-live rule for read-only.
- Use real objects via the existing test stores/fixtures, mocks only at the
  store boundary.

## Calls made while writing this (not discussed with the user)

1. **Lock-only command, keyed by connection.** `SetReadOnly` is really
   `LockReadOnly { connection }`: it can only raise the flag, never lower it. The
   core never lowers a live session's flag; unticking applies on reconnect. It
   applies to every live data session and every compare session store built from
   that saved connection (a connection can back several). The UI sends it when the
   box is newly ticked on a connection that has open sessions.
2. **Compare target checked twice in the core.** At `Open` (a read-only target is
   refused) and again at `Apply` (covers a connection locked after being chosen).
   The picker exclusion in the UI is only a convenience on top.
3. **`is_write()` is not trusted alone.** `ReadOnlyStore` is the backstop and a
   test must walk every runtime `Command` variant and assert write commands are
   refused with `ReadOnly`. The early `is_write()` check exists only so the
   refusal is reported before pending-write bookkeeping; if it disagrees with the
   store, `ReadOnly` from the store must still map to the read-only error, never
   to `WriteOutcomeUnknown` or `UnreachableWrite`.
4. **Repaired-flag banner.** Count damaged `read_only` values alongside damaged
   `production` values in the same startup banner, with a key that names the flag
   so the two are distinguishable. Update the existing banner tests accordingly.
5. **Connection management stays allowed.** Editing, duplicating and deleting the
   saved connection are configuration, not database writes, and are not blocked.
   "Edit, clone, delete, insert" in the UI section means documents only.

## Left to the builder

- Exact shape of the shared live flag and how the wrapper/session share it.
- Mechanism that makes a newly added `DocumentStore` write method fail tests
  until covered.
- Wording of all strings, badge placement details, banner styling.
