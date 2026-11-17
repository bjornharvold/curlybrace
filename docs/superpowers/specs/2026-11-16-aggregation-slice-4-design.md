# Aggregation builder, slice 4: aggregate options

Status: Draft, written from issue #75 (its find-query scope is left for later; this is the aggregate side) and the slice 1 to 3 designs, without a Q&A round (the owner said "just start"). Calls that are mine are marked **(my call)**. Earlier slices: `2026-11-13-aggregation-builder-design.md`, `2026-11-14-aggregation-slice-2-design.md`, `2026-11-15-aggregation-slice-3-design.md`.

## Intent

Reproduce a production aggregate exactly and diagnose a slow one: collation, index hint, `allowDiskUse`, a lower `maxTimeMS`, a `comment`, and the read preference, set once on the draft and applied consistently to preview, per-stage counts, explain, the confirmed run and the copied code. Plus the explain follow-ups slice 3 left here: a hint selector and a suggested-index link.

## Goals

- One **Options** section per draft (collapsed by default), saved with the pipeline, shown by an "options set" indicator in the header.
- Every operation sends exactly the options the screen shows, minus the ones that operation must not carry (table below), and the page says when an option was left out or lowered.
- Typed errors in words for the failures options introduce: a hint naming no index, a collation the server refuses, a read preference no member satisfies.

## Non-goals

Options on the Browse find query (#75 proper; the core types are built to be reused there), per-connection defaults (#45), `batchSize`, `let` variables, read concern, write concern, `bypassDocumentValidation`, read preference tags and `maxStalenessSeconds`, raising any time limit above today's bounds, building an index from the explain without the existing index dialog.

## The options

| Option | Type and validation **(my call)** |
|---|---|
| Collation | `locale` (required when collation is on; `simple` or an ICU-style id: letters, digits, `_`, an optional `@collation=<name>`, 1-32 chars) plus a small typed set, each optional (server default when unset): `strength` 1-5, `caseLevel`, `caseFirst` (upper, lower, off), `numericOrdering`, `alternate` (non-ignorable, shifted). `maxVariable`, `normalization`, `backwards` are not offered. |
| Hint | An index **name** (1-256 chars, no control characters) or a **key pattern** (typed text starting with `{`: a non-empty JSON object whose values are `1`, `-1` or an index type string such as `"text"`, `"2dsphere"`, `"hashed"`; `{"$natural": 1}` allowed). |
| allowDiskUse | Unset (server default), on, off. |
| maxTimeMS | Unset, or 1 to 60 000 ms. It can only **lower** an operation's own bound, never raise it (below). |
| Comment | A string, 1-256 chars, no control characters (not an arbitrary BSON value). |
| Read preference | Unset (the connection's own), `primary`, `primaryPreferred`, `secondary`, `secondaryPreferred`, `nearest`. |

Core type `PipelineOptions` (`crates/core/src/pipeline_options.rs`), every field an `Option`; `PipelineOptions::default()` is "nothing set" and sends exactly what slices 1-3 send today. Validation returns `OptionsError` with a stable `error_variant()`; the UI maps each to an `aggregate.options.problem.*` key.

## Which operation sends what

`PipelineOptions::applied(Operation) -> AppliedOptions` is the one pure policy function, with `Operation::{Preview, Count, Explain, Run}`. The store re-enforces the two safety rows (defence in depth).

| | Preview | Count | Explain | Run (`$out`/`$merge`) | Copy as code |
|---|---|---|---|---|---|
| Collation | yes | yes | yes | yes | yes |
| Hint | yes | yes | yes | yes | yes |
| allowDiskUse | **always false** | **always false** | as set | as set | as set |
| maxTimeMS | min(set, 10 s) | min(set, 10 s) | min(set, 10 s) | min(set, 60 s) | as set |
| Comment | yes | yes | yes | yes | yes |
| Read preference | as set | as set | as set | **never: always primary** | as set |

- **allowDiskUse on preview stays false (my call)**: slice 1's promise that a preview is cheap on a production server holds; a preview that needs a spill is a 10 s timeout anyway. The Options section says "applies to Explain and Run only". Explain carries it so an `executionStats` plan matches what Run would do.
- **maxTimeMS only lowers (my call)**: the client-side bounds (`AGGREGATE_PREVIEW_TIMEOUT`, `..._EXPLAIN_...`, `..._RUN_...`) are derived from the defaults; raising them is a separate decision (open question). A value above an operation's default is clamped and the field says so ("previews stop at 10 s").
- **Read preference never on the run (my call)**: `aggregate_write` always selects the **primary** explicitly, ignoring the option **and** the connection URI's own read preference (today the run inherits the connection's; this slice pins it). Even on 5.0+, where `$out`/`$merge` may read on a secondary, a confirmed write from this app reads and writes on the primary: predictable, and what the confirmation dialog describes. With a non-primary read preference set, the run dialog says "A run always reads and writes on the primary". Standalone and `mongos` topologies behave as the server does (a standalone ignores it; `mongos` forwards it).
- Copy as code emits the options **as set** (not the preview's policy), because the code is the person's real query; maxTimeMS is never above 60 s by validation.

## Where options live

- **Draft:** `AggregateState.options: PipelineOptions` (the validated, applied value) plus a form state with the typed text and its problems (`viewmodel/aggregate_options.rs`). A valid edit replaces the draft's options through `changed()`: the preview re-runs (debounced like a stage edit), counts and the plan are marked out of date, the saved "modified" mark updates. An **invalid** form blocks preview, count, explain, run and copy with the reason (like a stage that does not parse) **(my call)**: never silently send something other than what is shown. The last good preview stays.
- **Saved pipelines:** `SavedPipeline` gains `options: PipelineOptions`, `#[serde(default, skip_serializing_if = "PipelineOptions::is_empty")]`, JSON names as MongoDB spells them (`collation.locale`, `hint.name` / `hint.keys`, `allowDiskUse`, `maxTimeMS`, `comment`, `readPreference`). **`VERSION` stays 1 (my call):** the field is additive and optional, older builds ignore unknown fields (`SavedPipeline` does not deny them), so a file written by this build still opens in the previous one; the previous build drops the options only if it re-saves that entry. Rules: unknown keys **inside** `options` are ignored (forward compatible); a known key with an invalid value (strength 9, a malformed hint) refuses the whole file, which is then backed up and replaced with a recovery banner, exactly the existing rule for broken limits. A future change of a field's meaning or a new enum value bumps `VERSION`. Opening a saved pipeline restores its options; "modified" compares stages and options. A pipeline saved before this slice opens with no options.
- Not persisted elsewhere; per-session like the draft.

## UI (slice 4b)

- Header: an **Options** toggle button; when any option is set it reads "Options (N)" with the accent tone (the indicator), named for accessibility with its expanded state. The section opens above the stage list, inside the editor share, scrolls with it, and lays out in two columns above 900 pt width and one column below, so it fits at 800x500.
- Controls: collation on/off with locale (a combo of common locales plus free text), strength, the flags as tri-state (default, on, off) combos; hint as a combo listing the collection's indexes by name (from the Indexes tab's list when it holds this namespace, else one `Command::ListIndexes` when the section first opens, with loading and "could not list" states) plus a text field for a name or key pattern, and **Clear**; allowDiskUse tri-state with the "Explain and Run only" note; maxTimeMS number field with the defaults as placeholder; comment; read preference combo. **Reset options** clears all. Every control is named, in Tab order, operable by keyboard; problems show under the field in words.
- Status: the preview status line says "Read from secondaryPreferred" (or the mode set) when a read preference is set, and "hint: <name>" when a hint is set, so a result changed by options is visibly so (#75 asks for this).
- Run dialog: lists the options the run carries and the primary rule above.
- Read-only connections: every option is usable (all are reads; the run is disabled as before). Production: no extra confirmation for previews with options; the run keeps its typed confirmation.

## Explain hint selector and suggested index (slice 4c)

- In the Plan tab, when the plan shows a collection scan or an in-memory sort: **Try an index**, a combo of the collection's indexes; picking one sets the draft's hint (visible in Options, "modified" appears) and re-runs the explain at the same verbosity. **Clear hint** removes it.
- **Suggested index:** core `explain::suggest_for_pipeline(stages) -> Option<SuggestedIndex>` from the leading `$match` and a `$sort` directly after it, through the existing equality-sort-range rule (`suggest_from`). The Plan tab shows the `createIndex` command (`SuggestedIndex::command`) with **Copy**, and **Create index...** which opens the **existing index builder** pre-filled with those keys through `index_write_actions::open_with` (it already exists). Read-only: disabled with the reason; production: the builder's own write guard applies. Nothing is built without that dialog.

## Safety

- Options never turn a read into a write: no option adds a stage; the preview, count and explain paths keep refusing write stages at any depth; the run path keeps its shape check and typed confirmation.
- The store forces `allowDiskUse: false` on previews and the primary on writes whatever `AppliedOptions` says; tests assert both on the in-memory store's record and on the driver options built.
- Option values are user data: never logged (log `OptionsError::error_variant()` and option **names** only), never echoed from server text. The comment is sent to the server by design (that is its point: it shows in the server's logs and profiler); the section says so.

## Error mapping (in core, from the driver error and the applied options)

| Server answer | When | `CoreError` (UI key) |
|---|---|---|
| code 2 `BadValue` mentioning the hint | a hint was sent | `HintNotFound` (`errors.hint_not_found`) |
| code 2 `BadValue` mentioning collation/locale | a collation was sent | `CollationRejected` (`errors.collation_rejected`) |
| server selection failure | a non-primary read preference was sent | `ReadPreferenceUnavailable` (`errors.read_preference_unavailable`), shown inline, **not** a lost connection |
| `MaxTimeMSExpired` | any | unchanged (`Database("timeout")`), worded "took longer than the time limit you set" when maxTimeMS was set |

Anything else keeps today's classification. The server-selection wait is bounded by the driver's `serverSelectionTimeoutMS` (30 s default), so in practice the client bound may answer first as "took too long"; that is acceptable and stated in the README.

## i18n keys

- 4a: `errors.hint_not_found`, `errors.collation_rejected`, `errors.read_preference_unavailable`.
- 4b, under `aggregate.options.`: `toggle`, `toggle_count`, `toggle_hint`, `reset`, `comment_sent_note`, `blocked`; `collation.{enabled, locale, locale_hint, strength, strength_value.{1..5}, case_level, case_first, case_first_value.{upper, lower, off}, numeric_ordering, alternate, alternate_value.{non_ignorable, shifted}, server_default}`; `tri.{default, on, off}`; `hint.{label, placeholder, existing, loading, list_failed, not_listed, clear}`; `allow_disk_use.{label, preview_note}`; `max_time.{label, placeholder, clamped}`; `comment.{label, placeholder}`; `read_preference.{label, value.{default, primary, primary_preferred, secondary, secondary_preferred, nearest}}`; `problem.{locale_empty, locale_invalid, hint_empty, hint_name_too_long, hint_keys_invalid, hint_json, max_time_range, comment_empty, comment_too_long, comment_control}`; `status.{read_from, hint}`; `run.{applies, primary}`; `timeout_set`.
- 4c: `aggregate.options.explain.{try_index, clear_hint, suggested, copy_create_index, create_index, create_index_read_only}` and `codegen.options.include_options`.

## Constraints

Rust 1.87.0 (CI pin, no let-chains); no new dependency (the driver's `AggregateOptions`, `Collation`, `Hint`, `SelectionCriteria` cover everything); every user-facing string an i18n key, form keys under `aggregate.options.*`, core error keys under the existing `errors.*` convention **(my call)**; immutable style; TDD; coverage of new code at least 80%; real objects over mocks (the in-memory store records the applied options). Files 200-400 lines (800 max). Pre-existing files already over 800 (`runtime/mod.rs` 1050, `test_support/memory_store.rs` 927, `mongo_store.rs` 846, `runtime/handlers.rs` 814) must not grow: new code goes into new modules, and 4a moves the preview handler out of `handlers.rs`. Integration tests only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath (a `--replSet` one for read preference), never any MongoDB already running.

## Slices and PRs

- **4a, core (plan: `docs/superpowers/plans/2026-11-16-aggregation-options-4a.md`).** `PipelineOptions`, validation, serde, the `applied` policy, driver mapping, the store signatures (`&AppliedOptions` in place of `max_time`) for preview, explain and write, the in-memory record, `options` on the four aggregate commands (the UI sends the default, so behaviour is unchanged except the run now pins the primary), the three new `CoreError` variants and keys, container tests. No visible UI change.
- **4b, UI and saved.** Options section, form state and problems, indicator, status line, wiring into preview, counts, explain and the run dialog, hint index list, `SavedPipeline.options` with load rules and tests on old/new files, accessibility and snapshot tests at 1000x700 and 800x500, README.
- **4c, code and explain.** `CodeRequest::Aggregate` carries `options`, a `codegen` option "Include options" (default on); all nine languages render options idiomatically (options object or builder; read preference on the collection handle; hint name versus keys; collation fields), with new golden files for one "every option" fixture per language and **unchanged** goldens when no option is set. Explain **Try an index** / **Clear hint**, `suggest_for_pipeline`, Copy and **Create index...**; README and roadmap wording.

## Open questions only the owner can answer

1. May maxTimeMS raise the confirmed run above 60 s (with the client bound following)? Default here: no, lower only.
2. Pin the run to the primary even when the connection URI says otherwise? Default here: yes.
3. Keep `pipelines.json` at version 1 with an additive field, rather than bumping to 2 (which makes an older build back the file up)? Default here: keep 1.
