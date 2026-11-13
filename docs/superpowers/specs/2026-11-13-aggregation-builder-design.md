# Aggregation pipeline builder: design (slice 1)

Status: Draft, written from issue #67 without a Q&A round (the owner asked to start). Calls that are mine are marked **(my call)**.

## Intent

Build an aggregation pipeline stage by stage and see what each stage returns, inside curlybrace, without writing a `mongosh` script. The README lists this as a gap versus Compass/Studio 3T (#38).

## Goals

- A third collection tab, **Aggregate** (next to Documents | Indexes), holding one pipeline draft per open session.
- A pipeline is an ordered list of stages; each stage is JSON text. Add, remove, move up/down, enable/disable a stage.
- A **text mode** for the whole pipeline (a JSON array) with syntax errors shown inline (message, line and column). Switching modes keeps the content.
- **Preview**: the documents after the selected stage (the enabled stages up to and including it), at most `PREVIEW_LIMIT` (20) documents, shown in the existing Tree / Table / JSON views.
- Every preview is a read: bounded by `maxTimeMS`, never writes, never touches the saved pipeline.
- Errors say what is wrong in words; a timeout says so and keeps the last good preview on screen.

## Non-goals (slice 1)

- Running `$out` / `$merge` (a real write run). Slice 1 only previews; a pipeline with such a stage previews the stages before it and says the write stage was not run. The confirmed write run (typed confirmation, production guard) is slice 2.
- Saving / opening named pipelines, export as driver code, explain view, collation / hint / allowDiskUse options (slices 2-4).
- A visual per-field stage wizard. Stage text is JSON; completion offers stage names (and operators already known to the shell completion).
- SQL-to-pipeline (#10).

## Anti-goals

- A preview that can write, even by accident, even from a stage nested in `$facet`/`$lookup`.
- A "Run" that silently differs from what the person built (the preview limit must never leak into a saved or exported pipeline).
- Lost work: switching tabs, sessions or modes must not drop the draft.

## Behaviour

**Stages.** A new stage opens from an **Add stage** menu of common stage names (`$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`, `$unwind`, `$lookup`, `$addFields`, `$count`, `$facet`, `$out`, `$merge`, ...) with a template body (for example `{ "$match": {} }`). **(my call)** The menu offers names only; filling the body is the author's job in slice 1.

A stage card shows: an enable checkbox, its position, the stage name (read from the JSON when it parses), the JSON text box, Move up, Move down, Remove, and a one-line status (valid / the parse error). A disabled stage is skipped when building the preview and exports.

**Preview of stage *i*.** Built in core as `preview_pipeline(stages, upto, limit)`:
1. Take the enabled stages with index <= *i*, each parsed as one BSON document with exactly one key that starts with `$`; anything else is an error naming the stage.
2. Drop a trailing write stage (`$out`, `$merge`) and report it as "not run". A write stage anywhere else, or inside `$facet` / `$lookup` / `$unionWith` sub-pipelines, makes the preview **refuse** with a clear error (the server would reject it too; we never send it).
3. Append `{ "$limit": limit }`. **(my call)** `$limit`, not `$sample`: a sample changes what the stage sees.
Result: the pipeline to send, plus the list of stages that were not run.

**When it runs.** Selecting a stage, editing a stage (debounced), toggling or moving a stage, and a **Refresh** button run a preview of the selected stage. **(my call)** Only the selected stage is previewed (one aggregate per request), not every stage at once: N stages would cost N full-prefix runs on a large collection. The selection defaults to the last stage.

**Request handling.** A numbered request; a newer request or a changed pipeline makes an older answer stale and it is ignored. A timeout (`PREVIEW_MAX_TIME_MS`, 10 s server side, a client bound a little longer) shows "the preview took too long" and keeps the last preview. A read-only connection can preview (it is a read). A production connection previews too; the preview never writes. **(my call)** No confirmation for previews.

**Text mode.** One multi-line editor holding the pipeline as a JSON array of stage objects. Parse errors show line and column. Valid text round-trips to the stage list (formatting normalised to 2-space JSON); invalid text blocks switching back with the error shown.

**Where state lives.** A per-session `AggregateState` (stages, selection, mode, last preview, request counter) kept with the session view so switching sessions and tabs keeps the draft. Nothing is persisted in slice 1. **(my call)**

## Safety

- The preview command is a read: it is not a write in `Command::is_write`, runs on read-only sessions, and is never parked behind a production confirmation.
- Defence in depth: the core preview builder strips/refuses write stages, **and** the store method `aggregate_preview` rejects any pipeline that contains a write stage at any depth. The in-memory test store records the stages it receives so a test can assert none was forwarded.
- Pipelines and filters are never logged (log `error_variant` only), and error messages shown to the user do not echo server text beyond the existing mapped messages.
- No new dependency; no change to `history.sqlite` or other persisted files.

## Constraints

- Rust 1.87.0 (CI pin); no let-chains. Every user-facing string an i18n key (`aggregate.*`, nested in `crates/ui/locales/en.json`). Files under 400 lines (800 max). Real objects over mocks; the in-memory store scripts results.
- A real `mongod` is used for integration tests only on a throwaway instance (spare port, temp dbpath), never an existing one.

## Left to the builder

Exact layout of the cards (compact, 32 pt controls), the stage menu order, keyboard operation of move up/down, and completion details, within the accessibility rules (every control named, focus order, roles).

## Later slices

2. Saved pipelines per connection (`pipelines.json`), open/save/rename, and the confirmed `$out` / `$merge` run with typed confirmation and the production guard.
3. Export as driver code (#73), explain view (#69), per-stage document counts.
4. Options: collation, hint, allowDiskUse, read preference (#75).
