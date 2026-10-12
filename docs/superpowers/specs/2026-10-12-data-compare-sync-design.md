# Data Compare & Sync: Design

Date: 2026-10-12
Status: Approved by the maintainer on 2026-10-12 (including multi-pair, sequential pairs and a shared result budget)
Issue: #8 (epic). Reused later by #9 (masking and reschema) and #11 (task scheduler).

## Context and intent

Compare two collections, on the same server or on different servers, show exactly how they differ, and
(in later slices) apply the differences or export them. The motivating use is moving data between
environments (staging vs production, production vs a local copy) without throwaway scripts.

The feature is designed as a whole here, because apply and multi-pair runs shape the data model, but it is
**built in slices**. Only slice 1 is planned in detail now.

A compare covers **one or more collection pairs** (a whole database against another, or hand-picked
collections, including differently named ones). A single-pair compare is simply a list of one.

| Slice | Delivers | Writes to a database? |
|---|---|---|
| 1 | Diff engine, **pair-aware** compare session and runtime protocol, read-only Compare workspace with a single pair | **No** |
| 1b | "Find duplicate keys" helper | No |
| 1c | Multi-pair UI: database-level pairing, pair list, run overview | No |
| 2 | Apply (copy source to target, target to source, delete extras; field-level; pending changes) | Yes, behind confirmation |
| 3 | Export: `mongosh` script, extended JSON, CSV report | No |

Slice 1 builds the protocol and the runner pair-aware from the start, so slice 1c adds UI only and never
changes the events.

## Decisions already made with the user

1. **Scope:** design all slices now, plan and build slice 1 first.
2. **Connections:** a compare is a self-contained **job** that opens its own connections to the source
   and the target from saved connections. It does not use or disturb the browsing session.
3. **Match key:** `_id` by default, or one or more chosen fields. The key must be unique per side; a
   duplicate stops the compare with an error naming an example.
4. **Rules (all four offered):** ignore listed fields (or, added after the documentation cross-check, compare
   only listed fields), ignore array order, numbers equal across types, ignore field order in documents. Strict BSON equality is the default, except that field order is
   ignored by default.
5. **Result storage:** match keys plus fingerprints held in memory under a hard cap. Field-level
   detail is fetched on demand. **No document data is written to disk.** Apply re-checks fingerprints.
6. **UI:** a Compare workspace next to Browse, a top-level switch in the main window.
7. **Engine:** sorted merge-join over two keyset-paged streams. Not a hash of one whole side.
8. **`_id` on copy:** when the key is not `_id`, a copied document keeps its source `_id` by default and
   a collision on the target is a per-document failure; the user may instead choose "let the target
   assign new `_id`s". (Proposed in the design review and approved with it.)
9. **Multiple pairs:** designed in now (the maintainer's call after the documentation cross-check). Pairs run one
   after another inside one session; a failure in one pair does not stop the others.

## Non-goals

- Two-way merge or conflict resolution. Every apply is one direction with an explicit selection.
- Comparing indexes, collection options, validators, views, GridFS or schemas. Documents only.
- Scheduling or unattended runs (that is #11).
- Comparing more than one source and one target (the two sides are always one source connection and one
  target connection), and running pairs concurrently (they run one at a time).
- Duplicate keys: documents sharing a key are an error, never paired up by guesswork.

## Architecture

### Core units (`curlybrace-core::compare`)

No UI dependency. Each unit has one purpose and is tested on its own.

| Module | Purpose | Depends on |
|---|---|---|
| `key` | `KeySpec`, `KeyTuple`, and `compare_bson`, the client-side comparator that reproduces MongoDB's sort order | `bson` |
| `normalize` | Turns a document into a canonical form under `CompareRules` | `key` |
| `fingerprint` | 128-bit fingerprint of a normalized document | `normalize`, `sha2` |
| `diff` | Field-level difference between two documents, with BSON paths and types | `normalize` |
| `pairing` | Suggests collection pairs between two databases (matched, only in source, only in target) | `DocumentStore` kinds |
| `scan` | `KeyScanner`: a keyset-paged, ordered stream of one side, with order and duplicate checks | `DocumentStore`, `key` |
| `engine` | Merge-join of two scanners into a `ResultSet` that draws from the session's shared budget | `scan`, `fingerprint` |
| `job` | Runtime integration: sessions, pair runner, commands, events, budget, cancellation | `engine`, `pairing`, runtime |

### UI units (`curlybrace-ui`)

`viewmodel/compare/` holds every decision as pure functions on immutable state (setup form and its
validation, results filter and selection, detail selection). `views/compare/` is the thin egui layer.
This is the same split the Browse feature already uses.

### Types

```rust
pub struct KeySpec { pub fields: Vec<String> }        // dotted paths; non-empty; default ["_id"]
pub struct KeyTuple(pub Vec<Bson>);                   // one value per key field, in order

pub struct CompareRules {
    pub field_scope: FieldScope,                      // default All; dotted paths, a path through an array maps over its elements
    pub ignore_array_order: bool,                     // default false
    pub numbers_equal_across_types: bool,             // default false
    pub ignore_field_order: bool,                     // default true
    pub include_id: bool,                             // default true when the key contains `_id`, else false
}

pub enum FieldScope {
    All,                                              // compare every field
    AllExcept(Vec<String>),                           // ignore these paths
    Only(Vec<String>),                                // compare only these paths
}

/// One pair to compare inside an open compare session (the session owns the two connections).
/// A collection that does not exist counts as empty, so a pair can target a name that is not there yet.
pub struct PairSpec {
    pub source: Namespace,
    pub target: Namespace,
    pub source_filter: Document,                      // limits which documents are compared; default {}
    pub target_filter: Document,                      // the UI applies one filter to both sides unless told otherwise
    pub key: KeySpec,                                 // per pair: collections rarely share a business key
    pub rules: CompareRules,
}

/// What `StartCompare` receives: the pairs to run, one after another, in list order.
pub struct CompareRun {
    pub pairs: Vec<PairSpec>,
    pub limits: CompareLimits,
}

/// Unique within a session and monotonic: a later run on the same session continues the numbering, so
/// results of earlier pairs stay addressable until discarded.
pub struct PairId(pub u32);

pub struct CompareLimits {
    pub batch_size: u32,                              // default 1_000 documents per scan batch (per side)
    pub max_differences: u64,                         // default 1_000_000, a budget SHARED by all pairs of a session
    pub max_result_bytes: u64,                        // default 256 MiB, estimated, also shared by the session
    pub max_unindexed_documents: u64,                 // default 50_000, per side (see "Index requirement")
}

pub enum PairStatus {
    Queued, Running, Finished(CompareSummary),
    Failed(CoreError),                                // this pair only; the run continues
    Cancelled,
    NotRun(NotRunReason),
}

pub enum NotRunReason {
    Cancelled,                                        // the run was cancelled before this pair started
    BudgetExhausted,                                  // an earlier pair used up the shared result budget
    SessionFailed,                                    // a connection was lost mid-run
    Refused(CoreError),                               // preflight refused it (for example not indexed)
}

pub enum CollectionKind { Collection, View, Timeseries }

pub struct PairCheck {                                // one per pair, answer to CompareCheck
    pub index: (IndexStatus, IndexStatus),            // (source, target)
    pub estimated: (u64, u64),
    pub verdict: PairVerdict,
}
pub enum PairVerdict { Ok, Warn, Refused(CoreError) } // Warn: unindexed but small; Refused: unindexed and large

pub struct Fingerprint(pub [u8; 16]);

pub enum Category { OnlyInSource, OnlyInTarget, Changed }   // identical documents are only counted

pub struct DiffEntry {
    pub key: KeyTuple,
    pub source: Option<Fingerprint>,                  // None for OnlyInTarget
    pub target: Option<Fingerprint>,                  // None for OnlyInSource
}

pub struct CompareSummary {
    pub duration_ms: u64,
    pub scanned_source: u64, pub scanned_target: u64,
    pub identical: u64, pub only_in_source: u64, pub only_in_target: u64, pub changed: u64,
}

pub struct FieldChange {
    pub path: String,                                 // a.b[2].c
    pub kind: ChangeKind,                             // Added | Removed | Changed
    pub source: Option<Bson>, pub target: Option<Bson>,
}
```

`include_id` exists because, when the key is a business key such as `email`, `_id` values differ across
environments by construction. Comparing `_id` would then mark every document as changed. So `_id` is
excluded from the comparison by default unless it is part of the key, and the user can switch it back on.

### `DocumentStore` additions

```rust
async fn estimated_count(&self, ns: &Namespace) -> Result<u64>;

/// Documents matching `filter` whose key tuple is AT OR AFTER `from` (all of them when `None`),
/// in ascending key order under BINARY (simple) collation, at most `limit`. The start key is included so a
/// duplicate that straddles two batches is still seen (the scanner discards the one overlap row and treats a
/// second row with the same key as `CompareDuplicateKey`). A document missing a key field yields
/// `CoreError::CompareKeyMissing`; an unusable key value yields `CompareKeyInvalid`.
async fn scan_sorted(&self, ns: &Namespace, filter: &Document, key: &KeySpec,
                     from: Option<&KeyTuple>, limit: u32) -> Result<Vec<Document>>;

/// Collections of a database with their kind. `system.*` collections are included here; the pairing helper
/// excludes them by default. A view can be read but never written, so apply refuses a view as a target.
async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>>;

/// Whether an index can serve `scan_sorted` for this key (prefix match, simple or no collation).
async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus>; // Indexed | NotIndexed

// Slice 2 only:
async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<BulkOutcome>;
```

`MongoStore::scan_sorted` combines the user filter with the keyset condition using `$and`, and builds the keyset filter as an `$or` chain (for key `(a, b)` and
`from = (x, y)`: `a > x`, or `a == x and b >= y`). Because the server's `$gt`/`$gte` only match values of the SAME
type class while its sort order is total across classes, each comparison is type-aware: "`f >= v` within its
class, OR `f` is not of any class ranked at or below `v`'s" (written with `$not`/`$type`). Without this a key
that changes type (an `_id` that is an ObjectId in some documents and a string in others) would silently end the
scan. A tier-1 test pages through mixed-type keys. The scan sorts by the key fields ascending, forces
`collation: { locale: "simple" }`, and sets `allowDiskUse`. Two corners of the keyset filter are pinned by tests. A NaN start key
(Double or Decimal128) cannot use `$gt`/`$gte` (the server's `$gt: NaN` matches nothing and `$gte: NaN` only NaN, while
NaN sorts below every other number), so "at or after NaN" is written as "is a number" and "strictly after NaN" as
"is a number and not NaN". `null`, `undefined`, `minKey` and `maxKey` only equal themselves in a query, so for them
the in-class branch is an equality (inclusive start only) and the rest is the "higher class" branch. The client
comparator ranks Undefined strictly below Null (the server's canonical order); the two never occur together as keys,
because Undefined is refused as a key value, but the order stays total for nested values. Double and Decimal128 are
compared by exact numeric value, never through `f64`, and the tier-1 conformance test (server `sort` against
`compare_bson`) pins this. A namespace that does not exist behaves as an empty
collection for `scan_sorted`, `estimated_count` (0) and `key_index_status` (`Indexed`, vacuously). `InMemoryStore`
implements the same contract with the shared comparator, so engine tests need no server.

## The comparator and why it is the main risk

The merge-join is only correct if the client orders keys exactly as the server sorts them. The comparator
`compare_bson` follows MongoDB's documented BSON order: MinKey, Null, numbers (compared by exact numeric
value across Int32, Int64, Double and Decimal128), strings (by bytes under simple collation), objects,
arrays, binary, ObjectId, boolean, date, timestamp, regex, MaxKey. It starts from the ordering already
written for the in-memory test store (`test_support/bson_order.rs`) and fixes its known gaps: that code
compares numbers through `f64`, which is wrong above 2^53 for Int64 and for Decimal128, so it is replaced
by exact comparison.

Two defences make a mismatch loud instead of silent:

1. **Conformance test against a real server** (local-server tier): insert keys of every BSON type, including
   boundary numbers, into a throwaway collection, read them with `sort({k: 1})` and simple collation, and
   assert that the server order equals `compare_bson` order.
2. **Runtime order check:** `KeyScanner` verifies that each side arrives strictly increasing. An
   out-of-order key raises `CompareKeyOrder`; an equal adjacent key raises `CompareDuplicateKey`. The job
   fails instead of reporting false differences.

## Index requirement

Keyset paging with a sort is only cheap when an index serves the sort; otherwise every batch re-sorts the
remaining collection, which is quadratic. Before starting, a preflight (`CompareCheck`) asks each side of each pair for
`key_index_status` and `estimated_count`:

- Indexed (a usable prefix, simple or no collation): proceed.
- Not indexed and the estimated count is at most `max_unindexed_documents`: proceed with a visible warning.
- Not indexed and larger: refuse with `CompareKeyNotIndexed`. The error tells the user which index to create.

The limit applies to each side separately, so a pair is refused if either side is unindexed and above it. In a
multi-pair run a refused pair does not block the others: it is shown as refused in the preflight and, if the
user runs the rest anyway, it finishes as `NotRun(Refused)` without being scanned.

An index whose collation is not simple counts as not indexed, because it cannot serve a simple-collation sort.

## Normalization and fingerprints

Normalization produces a canonical document so that equal-under-the-rules documents hash identically.
Applied in this order:

1. **Field scope.** For `AllExcept(paths)`, remove those paths. For `Only(paths)`, keep only those paths. A path
   segment applied to an array applies to every element, so `items.price` addresses `price` in each element of
   `items`. Then remove `_id` unless `include_id`.
2. **Numbers across types** (when enabled). Every numeric value maps to one canonical form: a value that is
   integral and fits `i64` becomes `Int64`; a finite non-integral `Double` stays `Double`; `Decimal128` is
   normalised (trailing zeros stripped) and becomes `Int64` when integral and in range. Equality is exact:
   `Double 0.1` is **not** equal to `Decimal128 0.1`, because the double is not exactly one tenth. `NaN`
   equals `NaN`, and `-0.0` equals `0.0`.
3. **Field order** (when ignored). Sort document keys recursively by byte order. When not ignored, order is
   kept and is part of equality.
4. **Array order** (when ignored). Normalize each element first, then sort the elements by their canonical
   BSON bytes. This is total and deterministic, including for arrays of documents.

The fingerprint is the first 16 bytes of SHA-256 over the canonical BSON bytes of the normalized document.
`sha2` is already in the dependency tree through the MongoDB driver. The fingerprint is computed on the
**normalized** document, so fields outside the scope never affect it.

## Engine

`engine::run(source_scanner, target_scanner, rules, budget, progress, cancel) -> Result<(ResultSet, CompareSummary)>`
(one call per pair; `budget` is the session's shared budget handle)

Merge-join of two ascending streams. At each step compare the two head keys:

- source key < target key: the source document is **only in source**; record its key and fingerprint.
- source key > target key: the target document is **only in target**.
- equal: fingerprint both normalized documents; equal fingerprints count as **identical**, otherwise record a
  **changed** entry with both fingerprints.

Key matching always uses `compare_bson`, so numeric keys match across types (`Int32 1` and `Int64 1` are the
same key) whatever the rules say. If the key field itself differs in type, the pair is matched and reported as
**changed** under strict rules, with that field's path in the detail.

Memory is bounded by two batches plus the retained results. The result sets of all pairs in a session draw from
one shared budget (`max_differences`, `max_result_bytes`, estimated from key sizes plus 40 bytes per entry). The
pair that passes the budget fails with `CompareCapExceeded` and the pairs after it finish as
`NotRun(BudgetExhausted)`; discarding the results of finished pairs (`DiscardPair`) releases their share so the
user can run the rest. Only counts are kept for identical documents. With a filter, `estimated_count` is only an upper bound, so the
progress view shows documents scanned without a percentage.

## Runtime integration and job lifecycle

A compare is a **session** (also called the job) with its own id, independent of the browsing Connect/Disconnect
epoch. A session owns two connections and runs an ordered list of **pairs**, one at a time.

### Sessions, commands and events

`OpenCompare` connects both sides once; every later command names the job id, so the setup form, the
preflight, the runs, the results and the detail all use the same two connections. The same saved connection
may be both sides (a same-server compare opens two connections to it). A session accepts one active run at a
time; after it ends, a new `StartCompare` may run more pairs (for example a failed pair with a different key).
`PairId`s continue across runs, and earlier results stay addressable until discarded.

| Command | Single answer event | Meaning |
|---|---|---|
| `OpenCompare { source: ConnectionConfig, target: ConnectionConfig }` | `CompareOpened { job }` | Registers the job and connects both sides |
| `CompareList { job, side, db: Option<String> }` | `CompareNamespaces { job, side, db, names }` | Databases when `db` is `None`, else the collections of `db` with their `CollectionKind`; fills the setup pickers and the pairing helper |
| `CompareCheck { job, pairs }` | `CompareChecked { job, pairs: Vec<PairCheck> }` | Preflight per pair: `index: (IndexStatus, IndexStatus)`, `estimated: (u64, u64)` and a verdict (`Ok`, `Warn`, `Refused`) |
| `StartCompare { job, run }` | `CompareStarted { job, pairs: Vec<PairId> }` | Assigns pair ids and starts running the pairs in the background |
| `CancelCompare { job }` | `CompareCancelRequested { job }` | Cancels the running pair; pairs not yet started become `NotRun(Cancelled)` |
| `CompareRows { job, pair, category, offset, limit }` | `CompareRowsPage { job, pair, category, offset, total, rows }` | A page of keys for one category of one pair |
| `CompareDetail { job, pair, key }` | `CompareDetailLoaded { job, pair, key, source, target, changes }` | Fetches both documents by key and diffs them under that pair's rules |
| `DiscardPair { job, pair }` | `PairDiscarded { job, pair }` | Drops that pair's results and returns its share of the budget |
| `CloseCompare { job }` | `CompareClosed { job }` | Drops the job, all results and both connections |

Failures of these commands arrive as the usual `Failed { context, error }` with new contexts
(`compare_open`, `compare_list`, `compare_check`, `compare_start`, `compare_rows`, `compare_detail`,
`compare_discard`). A session accepts only the commands that make sense in its state (for example no second
`StartCompare` while a run is active); anything else answers `Failed` with `CoreError::Unsupported` and a
static message.

**Job events** carry the job id and are keyed to the job, not to a command:

- `CompareProgress { job, pair, summary, estimated_total }`, throttled to about four per second per running pair.
- **Exactly one pair terminal event per pair:** `ComparePairFinished { job, pair, summary }`,
  `ComparePairFailed { job, pair, error, detail }`, `ComparePairCancelled { job, pair }` or
  `ComparePairNotRun { job, pair, reason }`.
- **Exactly one run terminal event per `StartCompare`**, after every pair of that run has had its terminal event:
  `CompareRunFinished { job, totals }` (every pair reached a terminal state, even if some failed),
  `CompareRunCancelled { job }`, or `CompareRunFailed { job, error }` (a session-level failure such as a lost
  connection; the running pair gets `ComparePairFailed` and the rest `NotRun(SessionFailed)`).

This extends the existing "exactly one event per command" rule on purpose: each command still gets exactly one
answer, each pair gets exactly one terminal event, and each run gets exactly one run terminal event. The UI
reducer drops job events for unknown or closed job ids, the same way it drops answers from a replaced session.

**Pairs fail independently.** A duplicate key, a missing key field, a key-order violation or a cap overflow ends
only that pair (`ComparePairFailed`); the next pair starts. Only a session-level failure or a cancel stops the run.

**Slice 1 runtime, as built.** The names above are the design vocabulary; the shipped protocol
(`compare::protocol`) uses `Open`/`Opened`/`OpenFailed`, `Databases`, `Collections`, `Check`/`Checked`, `Start`,
`Progress`, `PairDone`, `RunDone`, `Rows`, `Detail`, `Discard`, `Cancel`, `Close` and `Failed { context }`. Its answer
rules:

- `Open` answers `Opened` or `OpenFailed`. `Databases`, `Collections`, `Check`, `Rows` and `Detail` answer with their
  event or `Failed { context }`. `Start` answers with one `PairDone` per pair and then one `RunDone`, or `Failed`
  when refused. `Cancel`, `Discard` and `Close` have no answer of their own.
- `Start`, `Cancel`, `Discard` and `Close` register their effect synchronously in the dispatcher, so command order
  decides the outcome: a `Cancel` or a second `Start` right behind a `Start` sees the run, an `Open` followed by a
  `Close` always closes. Only the slow work runs in a spawned task.
- A session is usable only once it has announced `Opened`: a `Start` (or any other command) sent between `Open` and
  `Opened` is refused with `Failed`/`Unsupported`, never queued. A command naming an unknown or closed session is
  answered with `Failed` ("the comparison is closed"); a closed session emits nothing more.
- At most one run is active per session (a second `Start` is refused), and the UI keeps at most one `Check` in
  flight: `Checked` carries no request identity, so a second one could not be told from a stale one. The reducer
  marks a check stale when the form is edited meanwhile and drops its answer.
- `Rows` events carry no run id (known limitation, accepted in slice 1): a late page of an earlier run is
  recognised only by session, pair, category and offset. The reducer takes a page only when it is the exact
  (category, offset) it asked for, and a new `Start` discards the previous results first; slice 1c must add a run
  id before several runs overlap on one session.
- `FieldChange.path` is display-only in slice 1 (`a.b[2].c`, `a[*]` for an element of an unordered array). It is a
  rendered string, not an address: slice 2 must add structured path segments to `FieldChange` before any apply
  logic consumes it, because a field name containing a dot or a bracket is ambiguous in the string form.

`ComparePairFailed.detail` can carry the offending key so the user can see it in the UI. It is never logged.

### Serializable definitions (for the scheduler, #11)

`PairSpec`, `CompareRun`, `CompareRules`, `KeySpec`, `FieldScope` and `CompareLimits` derive `Serialize` and
`Deserialize` and contain no secrets. A saved comparison list (`ComparePlan`: a name, the source and target
saved-connection ids, the pairs and the limits) can be stored and run later by the scheduler. Saved definitions
refer to connections by their saved-connection id, never by embedded configuration, and secrets still come only
from the Keychain.

### Pairing helper (pure, in core)

`pairing::suggest(source: &[(String, CollectionKind)], target: &[(String, CollectionKind)], opts) -> Vec<Suggestion>`
where a suggestion is `Matched { name }`, `OnlyInSource { name }` or `OnlyInTarget { name }`. Names match
exactly. `system.*` collections are excluded unless `opts.include_system` is set. `Matched` pairs are added to
the pair list in one click; the user can also pair differently named collections by hand, and a
`OnlyInSource`/`OnlyInTarget` collection can be paired with a new name because a missing collection counts as
empty (copying a whole collection is a compare against an empty target followed by an apply; indexes and
collection options are not copied, which the UI states).

### Connections, secrets and prompts

The job connects with the same `Connector`, secret store and SSH tunnel code as Connect, using the connection
configs in `OpenCompare`. Secrets still come only from the Keychain. The interactive host-key prompt is reused
through the existing per-connect gate, keyed to the job: a prompt for a cancelled or closed job is rejected
without asking, and a late "trust" answer is turned into a rejection so nothing is written to `known_hosts`.
After a run ends the job **keeps its two connections open** so the detail view (and, in slice 2, apply) can
re-read documents; `CloseCompare`, cancellation of the session and app exit release them and their tunnels. A
connection that drops while the session is idle or finished is re-established by the driver on the next
command; one that drops mid-run ends the run with `CompareRunFailed`.

### Independence from browsing

Connect, Disconnect and the browser's epoch counters are untouched. A job's `Failed` or terminal events can
never end the browsing session, and a browsing Disconnect never cancels a job. Closing the app cancels all jobs.

## UI

`AppState` gains `workspace: Workspace { Browse, Compare }` and `compare: CompareUi`. A top-level switch
changes the main area. A running session keeps running while the user switches back to Browse. The Compare
workspace moves through four views: **Setup**, **Preflight**, **Run overview** and **Pair results**.

### Slice 1 (single pair)

- **Setup form:** source and target (saved connection, database, collection), key fields, the four rules,
  `include_id`, an optional filter (JSON, applied to both sides) and a choice between all fields, all except
  some, or only some. Picking both saved connections sends `OpenCompare`; the database and collection pickers
  then fill from `CompareList` on the open session. Changing a connection closes and reopens the session.
  Validation is a pure function: both sides chosen, key non-empty, filter and ignore paths parse, source and
  target not identical.
- **Preflight:** shows the index status and sizes, with a warning or a refusal as above.
- **Running:** progress counters per category and per side, estimated percent (none with a filter), Cancel.
- **Results:** category tabs with counts (Only in source, Only in target, Changed; Identical shown as a
  number). A virtualised list of keys for the active category. Selecting a row loads the detail.
- **Detail:** both documents side by side as canonical extended JSON, with the changed paths listed and
  highlighted, so a type difference (Int32 vs Int64) is visible. Changed, source-only and target-only fields use
  different markers (an icon and a label as well as a colour, so colour is never the only signal). A toggle shows
  only the differing fields, and expand/collapse works per document and for all. Documents are fetched on demand
  and only for the selected row.
- **Entry points:** the Compare workspace switch, and a "Compare to..." entry in the Browse sidebar's collection
  menu that switches to Compare with that collection prefilled as the source.
- **Selection** (used by apply): per row, per filtered category, or all. Selection is a set of keys in the
  view-model, scoped to a pair; no apply happens in slice 1.

The slice 1 UI always holds a pair list of exactly one entry. The state, the view-models and the protocol are
already pair-aware, so slice 1c only adds UI.

### Slice 1c (multiple pairs)

- **Pair list** in Setup: a table with source collection, target collection, key, a marker when the pair has its
  own filter or field scope, and its status. Buttons: **Add pair**, **Add all same-named collections** (from the
  source and target databases chosen in the pickers, through the pairing helper), **Remove**, and **Edit
  settings** for one pair (key, filter, field scope, rules). `OnlyInSource` and `OnlyInTarget` collections are
  listed as "no counterpart" and can be paired with a new name.
- **Defaults for all pairs:** key, rules, filter and field scope set at the top apply to every pair that has not
  customised that setting. A pure function tracks which settings a pair has overridden, so changing a default
  never overwrites a deliberate per-pair choice. System collections are hidden unless the user switches them on.
- **Run overview:** one row per pair with a status (an icon and a label), the counts per category, the
  duration, and the error for a failed pair; a totals row; progress for the running pair; Cancel. Row actions:
  **Open** (the pair results), **Discard**, and **Re-run** (opens Setup with that pair so a failed pair can get a
  different key). A pair that failed on a key problem offers **Find duplicate keys** (slice 1b).
- **Pair results:** the slice 1 results and detail views for one pair, reached from the overview, with a way
  back. Switching pair keeps each pair's own selection.
- **Entry point:** a "Compare database to..." entry on a database in the Browse sidebar, prefilled the same way.
- The preflight lists each pair's verdict (ok, warning, refused). The user can run all pairs, or only the ones
  that passed.

All text uses i18n keys in `en.json` (nested, alphabetical). Failures are typed `CoreError` variants.

## Errors

New typed variants, each with an `errors.*` i18n key and a static, secret-free message:

| Variant | When |
|---|---|
| `CompareDuplicateKey` | two documents on one side share a key tuple |
| `CompareKeyMissing` | a document lacks a key field |
| `CompareKeyInvalid` | a key field holds an array, a regular expression or a deprecated type (Undefined, Symbol, DbPointer, JavaScript, JavaScriptWithScope): arrays sort by their smallest element, which breaks strict ordering, and the server rejects a regular expression as a `$gte` argument, so a keyset scan could not resume from one |
| `CompareKeyOrder` | a side's keyset stream is not strictly increasing (comparator or collation mismatch) |
| `CompareKeyNotIndexed` | no usable index and the collection is above the unindexed limit |
| `CompareCapExceeded` | the result cap was reached |
| `CompareSetupInvalid` | the key or the rule field lists are malformed: an empty list, an empty name, an empty path segment or a repeated name (`KeySpec::validate`, `CompareRules::validate`); raised before any scan and shown with its own `errors.compare_setup_invalid` text |

When a duplicate key stops a compare, the UI can run **Find duplicate keys** (slice 1b): a server-side
`$group` on the key with a count above one, limited to the first 20, so the user sees which keys collide and
how often and can pick a better key. It uses one extra `DocumentStore` method, `duplicate_keys(ns, filter, key,
limit)`, and the keys are shown in the UI only, never logged. The compare still never pairs duplicates up.

Connection and authentication failures reuse the existing variants. Cancellation is an outcome
(`ComparePairCancelled`, `CompareRunCancelled`), not an error. A pair that does not run is not an error either:
it is `NotRun` with a reason (`Cancelled`, `BudgetExhausted`, `SessionFailed` or `Refused(error)`).

## Apply (slice 2, designed now)

- **Plan.** From the selection, build an `ApplyPlan`. For direction *source to target*: insert documents only in
  source, replace changed ones, delete documents only in target. *Target to source* is the mirror. The user
  switches each operation kind on or off, and can narrow the selection. Unselected documents are never touched.
- **Re-verify before writing.** For each planned operation, re-read the document from the side being written
  and from the side being copied, recompute the fingerprints with the compare's rules, and compare them with the
  recorded ones. A mismatch skips the operation with the reason "changed since the compare"; apply never
  overwrites a document that changed in the meantime. A small window between the re-read and the write
  remains and is documented.
- **Writes.** Unordered batches of 500 through `bulk_write`. Every operation reports `Applied`,
  `Skipped(Stale)` or `Failed(reason)`. A re-run after a partial failure converges, because it plans only what
  still differs.
- **Replace and `_id`.** `_id` is immutable, so when the key is not `_id` the replacement document carries the
  destination's existing `_id`. Replace matches by the key fields.
- **Insert and `_id`.** Keep the source `_id` by default; a collision is a per-document failure. The user can
  choose "let the target assign new `_id`s".
- **Confirmation.** A dialog shows the counts per operation kind and names the target connection, database and
  collection. If any delete is planned, the user must type the name of the target collection when exactly one target
  collection is affected, or the name of the target database when several are, to proceed. The dialog groups
  the counts by pair.
- **Field-level operations.** Besides whole-document insert, replace and delete, an operation can copy a single
  field path from one side to the other (`$set`, or `$unset` when the field is absent at the origin). This changes
  only that field and leaves the rest of the destination document alone. It goes through the same fingerprint
  re-check, and is reported like any other operation.
- **Pending changes.** The plan is an ordered list of per-document operations the user builds up. A selection
  plus an action ("copy to target", "copy to source", "delete here", "copy this field") appends operations, so one
  apply can send some documents each way. Nothing is written until the user reviews the list and confirms it.
- **Across pairs.** Each pending change names its pair. Apply runs the pairs one at a time, each with that pair's
  rules and key for the fingerprint re-check, and reports per pair. A pair that has been discarded cannot
  contribute changes. A view cannot be the destination of any write; apply refuses it up front.
- **Progress and cancel** like the compare job, as a second job kind.

## Export (slice 3, designed now)

Two formats of the plan, written only to a file the user chooses (never to a temp file):

- A `mongosh` script that applies the plan with one `bulkWrite` per target collection and the fingerprints as
  comments.
- A JSON file: `{ header, pairs: [ { source, target, operations } ] }` with canonical extended JSON documents.

A third output, the **report**, lists the compare's differences (pair, key, category, changed paths) as CSV or to the
clipboard, and includes the summary (counts and duration). All three are written only on the user's action.

None of them contains credentials, connection strings or hosts; the header names the database and collection and
carries a free-text label. The UI warns that the files contain document data (the report contains keys, which can be personal data).

## Security and privacy

- No document data is written to disk by this feature. Results are keys and fingerprints in memory only.
- Logs carry job ids, counts, durations and error kinds. They never carry keys, field values or documents.
  This matters because keys may be personal data such as emails.
- Secrets come only from the Keychain. Each job's tunnels and connections are closed on completion,
  cancellation, close and app exit.
- The host-key trust rules from the Browse feature apply unchanged to jobs.
- Apply never writes without the re-verification step and the confirmation.

## Testing

1. **Comparator:** table-driven tests for every BSON type pair and boundary numbers (Int64 above 2^53, Decimal128
   with differing exponents, `NaN`, `-0.0`); **the server conformance test** described above.
2. **Normalize and fingerprint:** each rule alone and combined, all three field scopes (all, all except, only), nested arrays, arrays of documents, ignored paths
   through arrays, exact numeric equality including the `0.1` case.
3. **Diff:** nested documents, arrays with and without order, every change kind, type-only differences.
4. **Scanner:** keyset paging over the in-memory store, compound keys, a user filter combined with keyset paging,
   missing key, duplicate key, out-of-order stream, batch boundaries (exactly one batch, one more than a batch).
5. **Engine:** a property test that, for randomly generated pairs of collections and random rules, the
   merge-join result equals a brute-force set comparison; a memory-bound test with a counting store that asserts
   at most two batches of documents are ever in flight.
6. **Job and session:** open, list, check, start, cancel at each stage, commands in the wrong state answer
   `Failed`, events for closed jobs are dropped, two sessions at once, browsing Disconnect while a job runs, a
   connection that drops mid-run, prompt gating for a cancelled job, and connections released on close, cancel and
   app exit. **Multi-pair:** exactly one terminal event per pair and one run terminal event per run, all pair
   terminals before the run terminal; a failing pair does not stop its neighbours; the shared budget runs out and
   later pairs become `NotRun(BudgetExhausted)`; `DiscardPair` returns budget; cancel mid-run leaves the running
   pair `Cancelled` and the rest `NotRun(Cancelled)`; a second run continues the `PairId` numbering; a missing
   collection behaves as empty on either side.
6b. **Pairing helper:** exact-name matching, collections only on one side, `system.*` excluded unless enabled,
   views and time-series kinds, and the defaults-versus-overrides rules for pair settings.
7. **View-models:** form validation (including filter JSON and field scope), category switching, selection per
   pair, the only-differing-fields toggle, detail loading and stale detail answers, the sidebar "Compare to..." and
   "Compare database to..." prefills, the pair-list state machine and the run-overview rows for every status.
8. **Integration:** tier 1 (two throwaway databases on one local server, including a database-level compare of
   several collections where one pair fails); tier 2 (two containers, cross-server, including an SSH-tunnelled
   side). Click-driven UI flows and snapshots for the Compare views, including the pair list and the run
   overview, use the existing headless UI test infrastructure.
9. **Duplicate-key helper (slice 1b):** `duplicate_keys` against the in-memory store and a real server, including
   compound keys and the limit of 20.
10. **Scale:** a synthetic one-million-document compare against the in-memory store runs within the memory bound
   and finishes in reasonable time (measured and recorded, not asserted to a wall-clock number).

The default `cargo test --workspace` stays offline, database-free and GPU-free, as before.

## Cross-check with public documentation

The design was checked against the published documentation of an existing commercial tool, Studio 3T
([Data Compare and Sync](https://studio3t.com/knowledge-base/articles/data-compare-and-sync/),
[How to compare MongoDB collections](https://studio3t.com/knowledge-base/articles/compare-mongodb-collections/)),
to see which user-visible behaviour people expect. Only documented, user-facing behaviour was used. Nothing
here was derived from the product's binaries or code, which would conflict with this project's licence and its
"built from public documentation" statement.

| Their documented behaviour | In this design |
|---|---|
| Match on `_id` by default or chosen fields; advice to use unique, indexed fields | Same default. Stronger: the key must be unique (duplicates are an error) and the index is checked in the preflight |
| Filter to limit which documents are compared | **Added** (`source_filter`, `target_filter`) |
| Compare all fields, or specify fields to compare or ignore | `FieldScope`: all, all except, or **only** (the "only" mode is new) |
| Option to treat number types (Int32/Int64) as equal | Same, with exact numeric equality |
| Batch size, default 1,000 | Same default, in `CompareLimits` and an advanced setting |
| Start from a collection's context menu or the toolbar | **Added** the sidebar "Compare to..." entry |
| Overview with duration and counts | `CompareSummary` gained `duration_ms` |
| Side-by-side differences with colour categories, only-differing-fields, expand/collapse | **Added** to the detail view, with non-colour markers |
| Multiple Matches view for non-unique keys | **Differs on purpose:** duplicates are an error, with a "Find duplicate keys" helper (slice 1b), not pairing |
| Copy or delete a document, copy a single field, either direction | Document operations as designed; **added** field-level operations and a per-document pending-changes list |
| Confirmation before copying | Required, and stricter for deletes |
| Export to CSV or clipboard | **Added** the report export (slice 3), alongside the script and JSON plan |
| Schedule comparisons with a task scheduler | Request types are serializable and secret-free so #11 can store them |
| Compare many collection pairs or whole databases in one run, one tab per pair | **Designed in** (slice 1c): a pair list with same-name pairing and hand pairing, pairs run one after another, a run overview, and one results view per pair reached from it instead of one tab per pair |
| In-place editing of a field inside the diff view | **Not included.** The Browse editor edits documents |

Not documented there and decided here independently: array order, field order, behaviour on large
collections, failure handling when applying, and what happens to documents changed since the compare.

## Slice 1 plan outline

Small pull requests, each independently tested and useful:

1. `key`, `normalize`, `fingerprint`, `diff` (pure core, no store).
2. `DocumentStore` additions (`estimated_count`, `scan_sorted`, `key_index_status`) for the in-memory and Mongo
   stores, plus the comparator conformance test.
3. `scan` and `engine`, with the property and memory-bound tests.
4. Runtime compare session: open, list, check, start (a list of pairs, run sequentially with per-pair isolation
   and the shared budget), cancel, rows, detail, discard and close, with pair and run events.
5. Compare view-models.
6. Compare views, the workspace switch, headless UI flows and snapshots.
7. Docs: README, site, testing notes.

Slice 1b (small, after slice 1): **Find duplicate keys**.

Slice 1c (after slice 1): **multi-pair UI**: the pairing helper, the pair list and per-pair settings, the run
overview, per-pair results and the "Compare database to..." entry. The protocol and runner are already
pair-aware from slice 1, so this slice changes no events.

Slices 2 and 3 get their own plans after slice 1 ships.

## Acceptance for the whole feature (from #8)

- A design doc is merged and linked from #8 (this document).
- Two collections of one million documents compare with bounded memory, a progress indicator and cancel.
- Field-level differences are correct for nested documents, arrays and every BSON type.
- Apply is previewed, confirmed, batched, reports per-document failures, and a re-run converges (slice 2).
- Cross-server compare works with two saved connections and secrets never leave the Keychain.
- A database-level compare of several collection pairs runs with isolated failures, a shared result budget and
  a run overview (slice 1c).
- Unit tests, view-model tests and integration tests as listed above.
- All text uses i18n keys; new failures are typed errors with i18n keys.
- The README roadmap and the landing page are updated when each slice ships.

## Risks

- **Comparator drift from the server's order** is the main risk. Mitigated by the conformance test and the
  runtime order check, which turn it into a loud failure.
- **Unindexed keys** make keyset paging slow. Mitigated by the preflight, which refuses large unindexed compares.
- **Very long keys** raise the memory per difference. Mitigated by the byte-based cap, not only an entry count.
- **Drift between compare and apply** is bounded by re-verification, with a documented residual window.
- **Many pairs** multiply run time and retained results. Mitigated by the shared budget, sequential execution,
  per-pair progress and cancel, and `DiscardPair`.
- **Event complexity** grows with pairs. Mitigated by the invariants (one terminal event per pair, one per run,
  pair terminals first) being tested explicitly, and by building the protocol pair-aware in slice 1.
