# Queries that miss an index: what is left of #116

Status: Draft, written from issues #116 and #118 and the code on `main` (a7211c1a, v0.16.0), without a Q&A round. Calls that are mine are marked **(my call)**. Plan: `docs/superpowers/plans/2026-11-18-missed-index-queries-plan.md`. Earlier design, still binding where it applies: `docs/superpowers/specs/2026-11-07-index-miss-detection-design.md` (releases A and B of #116), plus the find-options ruling in `2026-11-18-find-query-options-design.md` (a verdict under a hint or a collation records no typed shape).

## Most of #116 is already built

#116 is still open only because both PRs said "Part of #116", so neither closed it. Both are on `main`, squash-merged as `ec967ce9` (2026-11-08):

- **#127, release A:** after every Run in the query bar, an automatic `executionStats` explain (bounded at 2 s server side, falling back to the plan only) gives a verdict badge with reasons and an equality-sort-range suggested `createIndex` command with a Copy button. It works on read-only connections and has a global opt-out (`explain.json`).
- **#128, release B:** value-free query shapes with a SHA-256 hash (`crates/core/src/shape`), a profiler read and consent-gated enable/disable through the `DocumentStore` (`mongo_profile.rs`, `read_only_store.rs` refuses the write), a background poller with a persisted watermark and roll-off gap detection (`runtime/profiler*.rs`), own traffic excluded by `appName`, a v2 history schema (`shape`, `shape_bucket`, `shape_gap`, `profile_mark`) that shares the 100 MiB cap and the raw 48 h / hourly 90 d / daily 2 y retention, typed-query shapes recorded under `source = typed` (`runtime/typed_record.rs`), and the **Indexes | Queries** toggle with a per-shape list (latest verdict, runs, misses, slowest, suggestion, copy buttons), a miss-trend chart over 24 h / 7 d / 30 d, and the profiler consent dialog.

The brief's "builds on" list needs correcting. #60 (slow query viewer), #69 (visual explain) and #72 (query history) are open and not built. #116 never needed them. The profiler read path #60 was meant to bring was built *inside* #116 release B. The aggregation builder's Plan tab (#175) already gives pipelines a plan view, a suggestion and a **Create index...** button. Also, the aggregate explain's bound is 10 s (`AGGREGATE_EXPLAIN_MAX_TIME_MS`), not 2 s. Only the find explain and plan check use 2 s.

So this spec does not redesign capture, shapes, storage or the Queries view. It covers **the gaps between the issue and the code**, found by reading the code and the "Known limits" of #128:

| # | Gap | Where it shows |
|---|---|---|
| G1 | A miss can only be copied as text. The issue's "act on it" needs **Create index...**, which the index builder can already do pre-filled (`IndexBuilderState::from_suggested`, used by the Aggregate Plan tab) | Queries rows; Browse verdict "Why" popup |
| G2 | The issue's headline case, **fields in the wrong order**, is only caught by the examined/returned ratio (≥100 examined and ≥10x returned). On small dev data the ratio never trips, so a range-before-equality index reads as "Index used" | `explain/classify.rs`, `profile/classify.rs` |
| G3 | Pipelines explained in the Aggregate tab (built after release B) are never recorded as typed shapes, so they are missing from the Queries list and trend | `runtime/aggregate_explain.rs` |
| G4 | A literal sub-document whose keys all start with `$` is treated as an operator document and its keys are kept (#128 known limit). That bends the "never store a literal" rule | `shape/normalize.rs` |
| G5 | A stored profiler watermark that is ahead of the server clock is not guarded (#128 known limit). Entries are skipped until the server clock catches up | `runtime/profiler/pass.rs` |
| G6 | Rows do not say when a shape last ran | `views/queries_rows.rs` |

## Recommended slicing (my call)

Three PRs, each useful on its own, in this order. The last one closes #116.

1. **Slice 1, act on a miss** (UI only): G1 and G6.
2. **Slice 2, field order** (core and UI): G2, a new verdict judged from the used index's key pattern.
3. **Slice 3, what gets recorded** (core): G3, G4 and G5. Closes #116.

The brief suggested "slice 2 = profiler with consent". That already shipped in #128 and is not repeated.

## What is captured (unchanged, restated so the slices stay inside it)

- **(a) The app's own queries, at no extra server load:** the Browse find's automatic verdict explain (already recorded) and, new in slice 3, the Aggregate tab's explain when the person presses Explain. Nothing new is sent to the server to record a shape. **(my call)** Aggregate previews and runs are not explained, so they are not recorded. Adding an explain to every preview would be new server load.
- **(b) The author's apps, through `system.profile`:** read-only (a `find` on `system.profile` plus `hello`). It works on read-only connections. Turning the profiler on or off is the only server write. It always goes through the consent dialog with the exact `setProfilingLevel` command, and `ReadOnlyStore` refuses it. This is unchanged. Production connections additionally show the existing production write warning (`write_warning.message_set_profiling_level`). No slice here adds a server write.

## Query shapes (unchanged, plus G4)

A filter, sort and projection are reduced to field names and operators. Every literal becomes `"?"`, `$in`/`$nin`/`$all` lists become `["?"]`, filter keys are sorted, sort key order and direction are kept, and projection keeps names only. The canonical JSON is hashed with SHA-256. `skip`, `limit` and every query option are not part of the shape. Profiler commands are shaped by verb (`find`, `count`, `distinct`, `update`, `delete`, `findAndModify`, `aggregate` = leading `$match` plus a `$sort` right after it). Rows are keyed by connection, host, db, collection, hash and source. Per connection, per namespace: yes. File: `history.sqlite`, created `0600` like the rest of the history store. Literals are never stored and never logged: failures log `variant_of(&error)` only.

**G4, operator allowlist (my call).** A document counts as an operator document only when every key is a known **query** operator: `$eq $ne $gt $gte $lt $lte $in $nin $exists $type $regex $options $all $elemMatch $size $not $mod $bitsAllSet $bitsAllClear $bitsAnySet $bitsAnyClear $geoWithin $within $geoIntersects $near $nearSphere $maxDistance $minDistance` (the operators mongod 9.0.8 accepts in a field's operator document, found by probing it; `$maxDistance` and `$minDistance` only valid beside `$near`/`$nearSphere`). `$comment` is not one: the server rejects it as a field-level operator. Nor are `$geometry $box $center $centerSphere $polygon $uniqueDocs`, which are operands of the geo operators and collapse with them. Any other `$` key makes the whole value a literal, which collapses to `"?"`. In query position the logical operators (`$and $or $nor`) and `$expr $text $where $jsonSchema $comment` behave as they do today. `Shape::from_json` validation uses the same list. Existing rows whose stored JSON no longer validates are deleted by an idempotent cleanup on open, inside the schema transaction like `PROFILE_MARK`, **without** bumping `user_version` **(my call)**. A version bump would make an older build refuse the whole file ("written by a newer build"). `ON DELETE CASCADE` removes their buckets. A shape that was split by an unknown operator becomes one shape from then on. This hashes differently, so old and new rows do not merge. That is acceptable: the rule is privacy first.

## How a miss is judged

Existing verdicts and thresholds stay: `INEFFICIENT_MIN_EXAMINED = 100`, `INEFFICIENT_RATIO = 10`, find explain `maxTimeMS` 2000 then plan only, the profiler judging from `planSummary` and `hasSortStage` with the same thresholds, and severity `NoFilter`/`EmptyCollection` 0 < `IndexUsed`/`Covered` 1 < `InefficientIndex` 2 < `SortNotCovered` 3 < `CollectionScan` 4. A collection scan on a small collection is still flagged.

**G2, a new verdict `FieldOrder`** (code `field_order`, persisted, severity 2, `is_miss` true) **(my call: a variant, not only a reason)**. The codes are persisted, and the existing rule is "add a variant, never rename". An older build reads the unknown code as neutral `NoFilter` (`shapes_read.rs`) and ranks it 0 in SQL (`severity_sql`'s `ELSE 0`). That is a safe downgrade.

- **Input:** the key pattern of the one index the winning plan scans, plus the query's equality, range and sort fields. Field classes come from the existing `suggest::is_equality` / `is_range` (so `$in` counts as equality, and `$ne`/`$regex` as range, exactly as the suggestion treats them), using top-level filter fields only.
  - Find explain: `keyPattern` of the single `IXSCAN`/`EXPRESS_IXSCAN` node (present in the captured fixtures, not read today).
  - Aggregate explain: the same node in the input plan. That is used in slice 3 only to record the verdict; the Plan tab's hints are unchanged.
  - Profiler: parsed from `planSummary` (`IXSCAN { a: 1, b: -1 }`), and only when the summary names exactly one `IXSCAN`. A name with `,`, `:`, `{` or `}` fails to parse, and then there is no field-order judgement (fail closed: no false alarm).
- **Rules, conservative (my call):** look only at the index keys from the start up to the first key the query does not use.
  - **R1:** a range field comes before an equality field among those keys. The verdict is `FieldOrder`, unless a stronger verdict applies.
  - **R2:** a range field comes before a sort field. The server then sorts in memory, so the verdict stays `SortNotCovered` and gains a field-order reason.
  - **Not judged:** indexes whose key values are not all ±1 (hashed, text, 2dsphere, wildcard), `_id_`, and plans with several index scans (`$or`).
  - **Judged:** plan-only explains as well, since the rule is structural and needs no counts. **(my call)** That is also why it fires on small data.
- **Precedence:** `EmptyCollection` > `NoFilter` > `SortNotCovered` (+R2 reason) > `CollectionScan` > **`FieldOrder`** (R1, with the `inefficient` reason too when the ratio also trips) > `InefficientIndex` > `Covered` > `IndexUsed`.
- **Plain-words reasons** (i18n, params are field and index names only, never values):
  - R1: "The index {index} puts {range}, a range, before {field}, an exact match. Exact matches should come first, then sort fields, then ranges."
  - R2: "The index {index} puts {range}, a range, before the sort field {field}, so the server sorts in memory."
  - `{index}` is the index name from explain, or the key list for the profiler, which has no name.
- **Suggestion:** the existing equality-sort-range suggestion (`suggest`, `suggest_for_shape`). `create_index_command` carries the query's collation where one is known (Browse verdict). A hint is never second-guessed: a typed query with a hint records no shape (existing rule), and its badge still judges what the server did.
- **The index-miss guard (#148) is unchanged:** it still warns on `CollectionScan` and `SortNotCovered` only. A field-order problem is advice, not a reason to stop a query. **(my call)**

## UI

**Where it shows:** the existing **Indexes | Queries** toggle in the collection's Indexes tab and the Browse verdict badge. No new tab or panel.

- **Queries rows (slice 1):**
  - A **Create index...** tonal compact button after the suggestion's Copy, on rows that have a suggestion. It opens the existing index builder pre-filled through `index_write_actions::open_with` and `IndexBuilderState::from_suggested`. Nothing is built until Create is pressed in the builder, and the builder keeps its production write guard.
  - Its accessible name carries the row position, like the copy buttons.
  - On a read-only connection the button is shown off with a tooltip and accessible description ("This connection is read-only. Copy the command to create the index elsewhere."), and `open_with` also refuses (defence in depth). While disconnected or while an index write or prompt is out, it is off.
  - A shape from the profiler gets no collation (the profiler command's collation is not part of the shape). That is stated in the out-of-scope list.
- **"Last run {when}"** on each row (slice 1), from the newest bucket of the range that ran the shape, worded with the Indexes tab's existing `indexes.ago_*` keys **(my call)**. Precision is the bucket width: 10 min, 1 h or 1 d.
- **Ranking stays** latest severity, then misses, then slowest, then shape text **(my call)**. "Cost" for typed rows has no duration (the explain's time is not the query's), so a cost sort would mix units. A sort picker is owner question 1.
- **Trend:** the existing miss chart above the rows and the usage chart in the Indexes view. Unchanged.
- **Browse verdict "Why" popup (slice 1):** **Create index...** beside Copy for a suggestion, with the collation of the query the verdict was computed for (`VerdictState::Ready.query.options.collation`). Off with the same read-only reason. The guard notice and the shell guard, which share `show_suggestion`, do not get the button **(my call)**. Their context is "about to run", not the collection's Indexes.
- **FieldOrder in the UI (slice 2):** the title "Index fields in the wrong order", warning tone, ⚠ icon plus words (never colour alone). The Browse verdict badge shows the reasons as above in its "Why" popup. A Queries row shows the title, the counts and the suggested index but no reason: history stores only the verdict code, not the reasons, so a row never invents one. The Queries list picks the title up through `row_title` / `kind_tone`.
- **States (unchanged):** empty (profiler off / on with nothing yet / unavailable), loading, read failed (inline), disconnected (status row), read-only (enable off with the reason; the view is fully readable). New buttons follow them.
- **Size:** at 800x500 with the 28 pt controls (`CONTROL_HEIGHT = 28`, #182), the row's buttons wrap (`horizontal_wrapped`) and the shape text keeps its own line. A layout test asserts that nothing crosses the right edge and that the first row stays on screen.
- **Strings:** every string is an i18n key (list in the plan). No literal in the views.

## Safety and security

- No new server write. Create index... only opens the builder.
- Query values never reach `history.sqlite`, a log line, an error or a `Debug`. Field and index names are allowed: they are already stored and shown today. G4 tightens the one hole that is known.
- Bounded:
  - Rows: the existing 100 MiB shared cap and tiered thinning (#117 tracks keeping charts at scale).
  - Profiler reads: `PROFILE_READ_LIMIT = 1000` per page and `MAX_PAGES_PER_TICK = 100`.
  - Explains: 2 s for find, 10 s for aggregate `executionStats`.
  - The field-order check is pure and linear in the key count.
- G5: `ProfileRead` gains `server_now_millis` from the `hello` it already sends (`localTime`). When a stored mark is more than `MARK_AHEAD_TOLERANCE_MS = 60_000` ahead of it, the mark is moved back to the server's now, unanchored (no false "missed entries"), and logged by variant name. No extra round trip.
- No new dependency (`bson`, `serde_json`, `sha2` and `rusqlite` cover everything).

## Tests

TDD per task (names in the plan). Coverage of new code at least 80% (`cargo llvm-cov`, installed).

- **Unit:**
  - the field-order rules (R1, R2, prefix stop, non-±1 keys, several index scans, plan-only);
  - `planSummary` key parsing (good, quoted or odd names give `None`);
  - classify precedence with real captured fixtures;
  - the operator allowlist (an unknown `$` key collapses, a known operator is kept, `from_json` rejects the old form);
  - the cleanup on open;
  - the aggregate verdict mapping;
  - the clamp of a mark that is ahead.
- **In-memory store:** `InMemoryStore` gives the explain and profile documents. The read-only wrapper still forwards `read_profile` with the new field and refuses `set_profiling_level`.
- **Real server:** only a throwaway `/opt/homebrew/bin/mongod` on a spare port with a temp dbpath, opt-in (`CURLYBRACE_IT=1`). Never an existing mongod.
  - `local_explain`: a range-before-equality index gives `FieldOrder` on 10 documents.
  - `local_profiler`: the same through `planSummary`.
  - `local_profiler_poller`: an Aggregate explain is recorded as typed.
- **UI:** viewmodel and reducer tests; kittest accessibility (`accessibility_queries.rs`, `accessibility_verdict.rs`); layout at 800x500 (`indexes_layout.rs`, `layout_audit_800.rs`); `queries_view_{light,dark}` snapshots regenerated, plus one new `queries_view_field_order_light`; i18n usage and order tests.

## Out of scope

- The slow query viewer (#60), the visual explain (#69), and query history and saved queries (#72).
- Charts at scale under the cap (#117).
- Collation or hint as part of an app shape. The profiler's command carries them, but storing a locale is storing a value (owner question 2).
- Shell queries as typed shapes.
- An explain on every aggregate preview.
- A cost sort.
- Profiling through `mongos`.
- Alerts.
- Cross-machine sync.

**#118 leftovers:**
- The `docker_e2e_failures.rs` "Connect" ambiguity is already fixed on `main`: no `click_text("Connect")` remains (#180). It can be ticked.
- None of the others touch the Queries view or verdicts ("1 restarts", the `$indexStats` 4.2.4 note, Hide before 4.4, the unformatted count, destructive Drop styling, the Sampling snapshot, the same-keys check, the late lost-connection reply, gap-hour roll-up). None is folded in **(my call: no scope growth)**.

## Calls I made (owner may overrule; the defaults are what gets built)

1. Treat #116 as mostly done; ship three gap slices; slice 3 closes it.
2. `FieldOrder` is a new persisted verdict (severity 2, a miss), judged structurally from the used index's key pattern, so it fires on small data.
3. The field-order check is conservative: the used prefix only, ±1 keys only, one index scan only, and the profiler only when the summary parses.
4. Create index... from Queries rows and the Browse verdict reuses `from_suggested` and `open_with`, not the guard notices.
5. Aggregate explains are recorded as typed shapes; previews are not explained.
6. The operator allowlist, and an idempotent cleanup on open without bumping the schema version.
7. A mark ahead of the server clock is clamped back to the server's `localTime` from the existing `hello`.
8. Ranking unchanged; "Last run" from the newest bucket.

## Owner questions (not blocking; defaults above)

1. Should the Queries list get a sort picker (misses / slowest / runs / last run), or keep the fixed order?
2. Should app shapes record that a collation or hint was used (a value-free marker), so they group apart and the suggestion can say "create it with the query's collation"?
3. Should the guard (#148) also warn on `FieldOrder`, or stay limited to scans and in-memory sorts?
4. Should #116 be closed after slice 3, with any later wishes in new issues?
