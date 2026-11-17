# Browse find query: query options (#75)

Status: Draft, written from issue #75 and the aggregate slice 4 design (`2026-11-16-aggregation-slice-4-design.md`, binding where it applies), without a Q&A round. Calls that are mine are marked **(my call)**. Plan: `docs/superpowers/plans/2026-11-18-find-query-options-plan.md`.

## Intent

Reproduce a production `find` exactly and diagnose a slow one from the Browse page: collation, index hint, a lower `maxTimeMS`, read preference, `comment` and `allowDiskUse`, set in the query bar and applied consistently to the page, paging, the index verdict, the index-miss guard and Copy as code. The page says when an option shaped what it shows. The aggregation builder already has all of this (slices 4a to 4c); this slice reuses its core type, policy shape, driver mapping, typed errors, form, hint picker and code rendering rather than building a second set.

## What exists today (read from the code)

- **Find path.** `views/findbar.rs` (Filter row with Find / Clear / Copy as code / Previous / Next and the **More options** toggle) and `views/findbar_more.rs` (Sort, Projection, Skip, Limit on one wrapping row). The denser-header plan (`~/.claude/plans/indexed-watching-bird.md`) is **already built** (#150): `FindBarState.more`, `findbar_more::{hidden_count, more_open, more_settled}`, "More options (N set)". `FindAction::Run(Query)` goes through `guard_actions::run` (plan-only guard check) and `explain_actions` (automatic verdict, `Command::ExplainFind`) to `actions::issue_find` (`Command::Find { session, ns, query }`). `AppState.{issued_query, page_query}` drive staleness (`reducer.rs`: `issued_query == query`), paging (`next_page`/`prev_page` clone the query), and every re-fetch after a write.
- **Store.** `DocumentStore::find(&Namespace, &Query) -> Page`, `explain_find`, `explain_plan` (both `mongo_explain.rs`, fixed `EXPLAIN_MAX_TIME_MS = 2000`). `MongoStore::find` sends no server-side `maxTimeMS`; the only bound is the client's `command_timeout` (30 s, `runtime::config::COMMAND_TIMEOUT`). Errors go through `db_err` (no option-aware classification).
- **Aggregate options (reusable as is).** `pipeline_options.rs` (`PipelineOptions`, validation, serde in MongoDB spellings, value-free `Debug`), `pipeline_options_apply.rs` (`AppliedOptions` with crate-private fields, `MaxTimeEffect`, `clamped_time`), `mongo_pipeline_options.rs` (`driver_collation`, `driver_hint`, `read_criteria`, `explain_fields`, `classify_with` giving `HintNotFound` / `CollationRejected` / `ReadPreferenceUnavailable`), `pipeline_options_doc.rs`, codegen `agg_options.rs` (`CodeAggOptions`, `plain_entries`, the mode/collation word tables), UI `viewmodel/aggregate_options.rs` (`OptionsForm`, `OptionsEdit`, `Tri`, `problem_message`, `keys_text`), `aggregate_hint_list.rs` + `app/aggregate_hint_actions.rs` (numbered `ListIndexes` above `HINT_REQUEST_BASE`, reusing the Indexes tab's list), `aggregate_options_view.rs` (`options_status`, `read_preference_key`), views `aggregate_options*.rs`, `code_export_options.rs`.
- **Query history (#72) is not built.** `crates/core/src/history` is the index-usage and shell history; no query history or saved queries exist (grep for history, saved queries, favourites: none). See "Saved with the query" below.

## Goals

- A **Query options** section behind **More options**, collapsed by default, with an "(N)" count, holding collation, hint (with the collection's indexes), time limit, read preference, comment and allow disk use.
- The options are part of the query: Find sends them, paging and every re-fetch keep them, the verdict and the guard judge the query with them, Copy as code carries them.
- The page says which options shaped the shown page (read preference, hint, collation, lowered time limit, disk use) and words the failures they cause inline.
- Typed errors from slice 4a reused: a hint naming no index, a refused collation, an unsatisfiable read preference; a timeout under the person's own limit says so.

## Non-goals

Query history and saved queries (#72), per-connection defaults (#45), `batchSize`, `let`, read concern, `noCursorTimeout`, `tailable`, `min` / `max` / `returnKey` / `showRecordId`, read preference tags and `maxStalenessSeconds`, carrying a collation into bulk update/delete or the copy of a whole result (they are blocked instead, below), raising any time limit, a visual query builder (#74), and the denser-header plan itself (built).

## The options

Exactly the aggregate's set and validation **(my call: one type)**: `pub type QueryOptions = PipelineOptions;` in a new `query_options.rs`. Same rules (locale, strength 1-5, case level, case first, numeric ordering, alternate; hint name or key pattern; `maxTimeMS` 1-60 000; comment 1-256 printable chars; read preference unset or one of five modes; allowDiskUse tri-state), same `OptionsError` variants and problem texts, same serde (so #72 can persist them unchanged).

`Query` gains `pub options: QueryOptions` (default empty) **(my call)**, rather than a parallel field beside `Query` on `Command::Find`, `ExplainFind`, `Event::Results`, `GuardTarget` and `AppState`. Reasons: staleness (`issued_query == query`), paging, re-fetch after a write, the verdict's `same_run`, the guard target and `CodeRequest::Find` then carry the options with no further plumbing; store signatures stay as they are; and only ten `Query { .. }` literals spell every field (the rest use `..Query::default()`), against 55 `Command::Find` and 19 `Event::Results` literals for the alternative. Paths that build their own `Query` (compare, copy, bulk, shell guard, profiler) get the empty default, which sends exactly what they send today.

## Which operation sends what

`QueryOptions::find_applied(FindOperation) -> AppliedFindOptions` is the one pure policy, with `FindOperation::{Find, Explain, PlanCheck}` (the page, the automatic verdict's `executionStats` explain, the guard's `queryPlanner` explain). `AppliedFindOptions` wraps 4a's `AppliedOptions` plus whether a server time limit is sent, so `driver_collation`, `driver_hint`, `read_criteria`, `explain_fields` and `classify_with` are reused unchanged.

| | Find (page, paging, re-fetch) | Explain (verdict) | PlanCheck (guard) | Copy as code |
|---|---|---|---|---|
| Collation | yes | yes | yes | yes |
| Hint | yes | yes | yes | yes |
| allowDiskUse | as set | as set | as set | as set |
| maxTimeMS | unset: none (as today); set: min(set, 30 s) | min(set, 2 s) | min(set, 2 s) | as set |
| Comment | yes | yes | yes | yes |
| Read preference | as set | as set | as set | as set |

- **Default sends today's bytes.** With no option set, find, explain and plan check send exactly what they send now (the 4a review rule). In particular the find still sends **no** `maxTimeMS` **(my call)**; adding a default 30 s server bound is a separate change (owner question 2).
- **maxTimeMS only lowers (binding).** The find's bound is the client `COMMAND_TIMEOUT` (30 s), named `FIND_MAX_TIME_MS = 30_000` and pinned to it by a test; a larger value is capped and the field says "Finds still stop at 30 s". The verdict and the guard keep their 2 s bound and only go lower. Validation stays 1-60 000 (shared type).
- **allowDiskUse is sent as set on the find (my call).** The binding rule "preview never sends allowDiskUse true" is about the aggregate's preview, which stands in for a later confirmed run. A Browse find has no later run: it **is** the person's query, already bounded by its page limit (at most 1000) and 30 s. Unset sends nothing (server default). Owner question 1. MongoDB before 4.4 refuses the field: that is an ordinary database error.
- **Read preference applies everywhere, including the verdict and the guard (my call)**, so they judge the plan of the node that answers the find (today they follow the connection's preference; the option overrides it the same way: `read_criteria(o).or_else(|| explain_criteria(db))`). Writes (insert, edit, delete, bulk) never carry it; they go to the primary as today.
- The store applies the policy itself (`find` and the two explains read `q.options`), so no caller can send options another way; the clamp of `clamped_time` stays the driver boundary's guard.

## Saved with the query (ruling, since #72 is not built)

"Saved with the query in history" cannot mean a stored history today. **Ruling (my call):** in this slice the options are saved **with the query in the session** and **in the shareable code**, and the persisted shape is decided for #72:

1. The options are part of the issued `Query`: paging, refresh, re-fetch after any write, switching sessions (the find bar is parked per session) and the verdict all keep them. Opening another collection and **Clear** reset them, like the Filter **(my call)**.
2. Copy as code carries them in all nine languages (below), with the dialog's "Include options" check.
3. For #72: a history or favourite entry stores the query text **and** `options` serialized with `PipelineOptions`' serde (MongoDB spellings, absent when empty, unknown keys ignored, an invalid known value refuses the entry), and reloading one restores the form with `OptionsForm::from_options`. Nothing is written to disk by this slice.

## UI

Builds on the existing **More options** area; the denser-header plan is not reimplemented.

- **Where.** At the end of the Sort / Projection / Skip / Limit row, a **Query options** toggle (`query.options.toggle`, "Query options (N)" with the accent when N > 0, named with its expanded state like the aggregate toggle). Open, a framed section under that row **(my call: a second disclosure inside More, not a fourth row of the header)**.
- **More's count and auto-open.** "More options (N set)" counts the set options too, and More opens by itself when an option is set (the existing `more_settled` rule: shown open once, it sticks until Clear). The options section follows the same rule with its own `Option<bool>` choice.
- **Layout at 800x500 (my call).** Wrapping rows, not two columns: (1) Collation check, Locale, Common locales, Strength, and while ticked Case level, Case first, Numeric ordering, Alternate; (2) Existing indexes, Hint, Clear, Time limit, Read preference, Allow disk use; (3) Comment, then the notes (comment is sent to the server; time limit capped; hint not listed), problems under their fields, **Reset options**. The section scrolls inside a bounded height (`OPTIONS_MAX_HEIGHT_SHARE = 0.35` of the window height, at least 140 pt) so at 800x500 the first document row stays on screen. The collation flags show only while Collation is ticked (the aggregate disables them; here they are hidden to save rows).
- **Applying.** Unlike the aggregate preview, nothing runs on an option edit **(my call)**: the options are part of the query and apply on **Find** (or Enter in any field of the bar, the options' text fields included), like Sort and Limit. Paging keeps the options the shown page was found with, as it keeps its Sort.
- **Invalid form.** Problems show under their fields in words (`problem_message`, same texts). While the form has a problem, **Find** and Copy as code are off with `query.options.blocked` as tooltip and accessible description, Enter does nothing, and More plus the section open so the problem is on screen. The last good page stays.
- **Hint picker.** The collection's index names, from the Indexes tab's list when it holds this collection, else one numbered `ListIndexes` when the section first opens; loading and "could not list" states; a name not in the list is a note, never a block. **(my call)** The list is the one the Aggregate tab's Options already keep for the selected collection (`AggregateState.options_ui.hint`, reset on a collection change): one list per session and collection, one request space, no second router.
- **What shaped the page.** In the wrapping row of the two switches and the verdict, weak text after the verdict, from the **shown page's** query (`page_query.options`, not the form): "Read from secondaryPreferred", "hint: status_1", "Collation: en", "Time limit: 500 ms" (only when lowered), "Disk use allowed"; with a non-primary read preference also "Recent writes may not show yet." Nothing when no option shaped the page.
- **Errors inline.** A find answered with `HintNotFound`, `CollationRejected` or `ReadPreferenceUnavailable`, or a timeout while a lowered time limit was sent, shows in the find bar's error line (with More and the section opened), not as a banner **(my call)**; the last page stays. Other find failures keep today's banner.
- **Bulk and copy (my call).** A collation changes which documents a filter matches, and bulk update/delete and "Copy all documents of the query" re-run the filter without it. While the shown page was found with a collation, Update matching / Delete matching are off with `query.options.bulk_collation` (new `BulkEntry::Collated`), and the whole-result copy is refused with `query.options.copy_collation` (new `PasteBlock::CollatedResult`). Hand-picked documents, edit, clone and delete by `_id` are unaffected. Hint, time limit, comment and read preference do not change the matched set, so they do not block.
- **Read-only and production.** Every option is usable on a read-only connection (all are reads); no extra confirmation on production.
- **Copy as code dialog.** For find code with options, the include check reads `codegen.options.include_find_options` ("Include the query's options") and the summary lists what the code sets; the read preference is never left out (find code never writes).

## Copy as code

`CodeRequest::Find { ns, query }` already carries the options inside `query`. `generate` computes `CodeAggOptions::of(&query.options, include_options, false)` and the find renderers gain the `agg` parameter. Byte-identical code when no option is set or the check is off (every existing `find*.txt` golden unchanged). Spellings (driver names to verify like 4c; option order: collation, hint, allowDiskUse, maxTimeMS, comment, read preference):

| Language | Find with options |
|---|---|
| mongosh | cursor methods after `find(..)`: `.collation({..})`, `.hint(..)`, `.allowDiskUse(..)`, `.maxTimeMS(n)`, `.comment("..")`, `.readPref("mode")` |
| Node.js | `find(filter, { readPreference: "mode" })` when set, then cursor methods `.collation`, `.hint`, `.allowDiskUse`, `.maxTimeMS`, `.comment` |
| Python | keyword arguments `collation=`, `hint=` (name or list of pairs), `allow_disk_use=`, `max_time_ms=`, `comment=`; read preference on `with_options(read_preference=ReadPreference.X)` |
| Java | `FindIterable` chain `.collation(..)`, `.hintString`/`.hint(Document)`, `.allowDiskUse`, `.maxTime(n, TimeUnit.MILLISECONDS)`, `.comment`; handle `withReadPreference` |
| C# | `new FindOptions { Collation, Hint, AllowDiskUse, MaxTime, Comment }` passed to `Find(filter, options)`; handle `WithReadPreference` |
| Go (v2) | `options.Find()` chain `SetCollation`, `SetHint`, `SetAllowDiskUse`, `SetComment`; time limit as v2 offers it (to verify: `SetMaxTime` is gone in v2; else a context deadline); handle `Clone(options.Collection().SetReadPreference(..))` |
| PHP | the find options array gains `collation`, `hint`, `allowDiskUse`, `maxTimeMS`, `comment`, `readPreference` (a `MongoDB\Driver\ReadPreference`) |
| Ruby | `find(filter, collation:, hint:, allow_disk_use:, max_time_ms:, comment:)`, handle `with(read: { mode: :x })` |
| Rust | setters on the `find` action: `.collation`, `.hint`, `.allow_disk_use`, `.max_time(Duration)`, `.comment`, `.selection_criteria` |

4c's test `options_never_reach_find_code` is replaced by the opposite rule.

## Safety

- Options never turn a read into a write; find, explain and plan check stay reads, allowed on read-only connections.
- What is sent is what was shown: the store derives every option from `q.options` through the one policy; Find refuses an invalid form; staleness compares the whole query, options included (`issued_query == query` does by derive; `explain_actions::same_run` lists fields by hand and must add `options`, or a verdict made under other options would show).
- Option values are user data: never logged (log option **names** or `OptionsError::error_variant()` only), never in an error message, never in a logged `Debug` (`PipelineOptions`' `Debug` already prints only which options are set, and `Query`'s derived `Debug` inherits it). Server text is matched, never kept. The comment reaches the server's logs and profiler by design; the section says so.
- The typed-shape record of a verdict is skipped when the query carries a hint or a collation **(my call)**: that verdict describes the options, not the shape.

## Error mapping

Reused from 4a through `classify_with` on find, explain and plan check:

| Server answer | When | `CoreError` (key) | Where shown |
|---|---|---|---|
| code 2, hint phrase | a hint was sent | `HintNotFound` (`errors.hint_not_found`) | find bar error line; verdict "unavailable" |
| code 2, locale phrase | a collation was sent | `CollationRejected` (`errors.collation_rejected`) | find bar error line |
| server selection, servers found but none suitable | non-primary read preference | `ReadPreferenceUnavailable` | find bar error line, **not** a lost connection |
| `MaxTimeMSExpired` | a lowered time limit was sent | unchanged `Database("timeout")`, worded `query.options.timeout_set` | find bar error line |

Everything else keeps today's classification and banner. A guard check that fails lets the find run as today (the find then answers with its own typed error).

## i18n keys

New, under `query.options` (between `query.guard` and `query.verdict`): `blocked`, `bulk_collation`, `comment_placeholder`, `copy_collation`, `disk_use_note`, `max_time.{clamped, placeholder}`, `status.{collation, disk_use, max_time, secondary_note}`, `timeout_set`, `toggle`, `toggle_count`, `toggle_hint`. Under `codegen.options`: `include_find_options`. Everything else is reused from `aggregate.options.*` (field labels, values, problems, `tri.*`, `hint.*`, `read_preference.*`, `comment_sent_note`, `reset`, `status.{read_from, hint}`) and `errors.*` **(my call: the shared labels keep their `aggregate.options` keys; moving them to a neutral namespace is owner question 4)**.

## Constraints

Rust 1.87.0 (no let-chains); no new dependency (`FindOptions` of `mongodb` 3.x covers everything); every user-facing string an i18n key, nested and alphabetical in `crates/ui/locales/en.json`, written as literals; immutable style; TDD with coverage of new code at least 80%; real objects over mocks (the in-memory store records the queries `find` received). Files 200-400 lines (800 max). Files already over 800 must not grow: `runtime/mod.rs` (1057), `test_support/memory_store.rs` (926), `mongo_store.rs` (841, its `find` moves out to `mongo_find.rs`), `runtime/handlers.rs` (809), `app/bulk_actions.rs` (849, `bulk_entry` moves out first), `app/mod.rs` (1007, `mod` lines only); `app/reducer.rs` (722) gets only a one-line delegation. Integration tests only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath (a `--replSet` one for read preference), never any MongoDB already running on the machine.

## What is left to the builder

Exact pixel sizes of the section fields and its height share (checked by the layout tests and screenshots), the egui id scoping of the shared form controls, whether the shared view extraction is one or two files, the Go v2 time-limit spelling after checking the driver, and the wording of the new English strings within the meaning given here.

## Open questions only the owner can answer

1. **allowDiskUse on the Browse find:** sent as set (default here), or forced off like the aggregate preview?
2. **Server-side time limit by default:** keep sending none when unset (default here, today's bytes), or always send `maxTimeMS` 30 s so a timed-out find stops on the server too?
3. **Reset on collection change and on Clear:** options reset with the Filter (default here), or stick per session until #45's per-connection defaults?
4. **i18n namespace of shared labels:** reuse `aggregate.options.*` keys in the find bar (default here), or move them to a neutral `options.*` now?
5. **Editing a page read on a secondary:** allowed, with the "Recent writes may not show yet" note (default here), or Edit / Clone off while a non-primary read preference shaped the page?
6. **Bulk and whole-result copy under a collation:** blocked with a reason (default here), or carry the collation into bulk count, preview, write and the copy job?
7. **Typed-shape history with a hint or collation:** skipped (default here), or recorded like any other verdict?
