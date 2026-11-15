# Aggregation builder, slice 3: driver code, explain, per-stage counts

Status: Draft, written from issues #73 and #69 and the slice 1 and 2 designs without a Q&A round (the owner said "slice 3"). Calls that are mine are marked **(my call)**. Earlier slices: `2026-11-13-aggregation-builder-design.md`, `2026-11-14-aggregation-slice-2-design.md`.

## Intent

Take a pipeline (or a query) out of the app into code, understand why it is slow, and see how many documents each stage lets through. Three independent pieces, three PRs, in this order: **3a** copy as driver code, **3b** explain for a pipeline, **3c** per-stage counts. 3a also covers the find query (#73), because the generator works on a filter, sort, projection, skip and limit as well.

## Slice 3a: copy as driver code (#73)

### Goals
- A **Copy as code** action on the Aggregate tab (the enabled stages) and on the Browse query bar (filter, sort, projection, skip, limit). It opens a small dialog: a language picker, options, a read-only preview of the generated code, and a **Copy** button.
- Languages **(my call, from #73)**: mongosh, Node.js, Python, Java, C#, Go, PHP, Ruby, Rust.
- Options: include import/setup lines (default on), the variable names for client, database and collection, and for a pipeline whether to include the trailing write stage (default on; a run is the user's decision, copying code never runs anything).
- Correct BSON syntax per language: `ObjectId`, dates, `Decimal128`, `Int64`, `Int32` versus doubles, binary, regex with flags, `Timestamp`, `MinKey`/`MaxKey`, `Code`, `UUID`, nested documents and arrays, `null`. Strings are escaped for the target language (quotes, backslashes, control characters, unicode, and the interpolation sigils of each language: `$`, `#{}`, `${}` where they matter).
- The generated code is a runnable snippet that connects with a **placeholder connection string** (`"<your connection string>"`), never the real one and never a password. The database and collection come from the current namespace.

### Non-goals
Running the code, importing code, generating write operations other than a trailing `$out`/`$merge` inside a pipeline, driver version selection, formatting beyond a fixed style per language, any network use.

### Behaviour
- **Core**: a pure module `codegen` with one renderer per language over a language-neutral model of `bson::Bson` values (a `Doc` tree), so the type-syntax tables are separate from the structure. `generate(request) -> Result<String, CodegenError>` where the request is `Find { filter, sort, projection, skip, limit }` or `Aggregate { stages }`, plus the namespace, language and options. Input stages come from the same parsed documents the preview uses (`pipeline::enabled_stages`-style helper; the preview `$limit` is **never** included).
- Deterministic output, golden files per language and per request kind under `crates/core/tests/codegen/` (one fixture holding **every** BSON type, one realistic pipeline with `$lookup`/`$group`/`$facet`, one find with sort and projection, one with awkward strings).
- The clipboard text is not logged; errors name the language or the stage, never the content.
- **UI**: a view model `CodeExportUi { dialog: Option<CodeDialog{language, options, text}> }` kept per session like the draft; the last language and options are remembered per app run **(my call)**, not persisted. Every label an i18n key under `codegen.*`; the dialog works at 800x500 and is keyboard operable (Escape closes, Enter copies); the preview text is selectable.
- A pipeline with a stage that does not parse cannot be exported: the button is disabled with the same reason the preview shows.

### Safety
Generating code reads nothing from the server and sends nothing. The connection string is always the placeholder.

## Slice 3b: explain for a pipeline (#69, aggregate part)

### Goals
- An **Explain** button on the Aggregate tab (the enabled stages up to and including the selected one, like the preview, without a trailing write stage) opening the plan as a tree: stages, index used, keys and documents examined versus returned, time per stage when the server reports it, a summary line, plain-language hints (collection scan, in-memory sort).
- The find explain that already exists (`Command::ExplainFind`, `explain/` verdict module) keeps working; 3b reuses its classification and hint vocabulary where the plan shapes match.
- **(my call)** `queryPlanner` by default (cheap, never executes), `executionStats` on request with a visible cost note; never `allPlansExecution` in this slice. An explain with `executionStats` of a pipeline **never** carries a write stage (the plan builder already strips it; the store refuses it again).
- Raw explain JSON one click away; sharded explain per shard if the server returns one.

### Non-goals
Hint selector and suggested-index link (they need the index dialog, left to slice 4's options), `allPlansExecution`, comparing plans, explain for write commands.

### Behaviour
- New `DocumentStore::aggregate_explain(ns, pipeline, verbosity, max_time) -> Result<Explain>`; a read, allowed read-only; `ReadOnlyStore` passes it through; defence in depth refuses a write stage at any depth like `aggregate_preview`. New `Command::AggregateExplain` and `Event::AggregateExplained`, numbered, stale answers ignored, never a banner (an inline outcome like the preview).
- Core parses the aggregate explain document into a typed plan tree (`AggStage { name, detail, keys_examined, docs_examined, n_returned, millis }` plus the cursor stage and `$cursor.queryPlanner` winning plan for the leading pushed-down part), tested on stored explain outputs under `crates/core/tests/fixtures/explain_aggregate/` (COLLSCAN, IXSCAN, `$group`, `$lookup`, `$facet`, sharded; shapes from several server versions: the old `stages` array and the newer single `queryPlanner` form).
- UI: an **Explain** section under the preview (tabs Preview | Plan | Raw **(my call)**), every string `aggregate.explain.*`.

## Slice 3c: per-stage counts

### Goals
- For each stage card, the number of documents that stage lets through for the current draft, shown on the card as "N documents" (or "at least N" when capped), computed on demand with a **Count stages** button, never automatically.

### Non-goals
Live counting while typing, counting after a write stage, exact counts on very large collections.

### Behaviour
- **(my call)** One bounded aggregation per stage (`stages[..=i] + {$count}`), run one after another (never in parallel), each with the preview's `maxTimeMS` and `$limit` cap `COUNT_CAP` (100 000) injected before `$count` so a huge collection yields "at least 100 000"; a stage that fails or times out shows its error on that card and the sequence goes on to the next. A newer draft cancels the sequence (stale answers ignored by request number). Read-only, allowed on read-only connections; never sends a write stage (the same plan builder).
- Core: `count_pipeline(stages, upto, cap)` next to `preview_pipeline`; reuse `DocumentStore::aggregate_preview` for the transport (a `$count` pipeline is a read). New `Command::AggregateCounts` is **not** needed: the UI issues one `AggregatePreview`-style read per stage through a small sequencer in the view model **(my call)**, so the runtime stays unchanged.

## Constraints
Rust 1.87.0 (CI pin, no let-chains); every user-facing string an i18n key (nested in `crates/ui/locales/en.json`); files 200-400 lines (800 max); no new dependency (no templating crate: renderers are plain Rust); real objects over mocks; TDD; coverage of new code at least 80%; golden files reviewed by hand, not regenerated blindly. Real `mongod` only as a throwaway on a spare port, for 3b/3c integration tests. Nothing logs pipeline text, clipboard text or explain documents.

## Left to the builder
Dialog layout, the exact hint wording within the i18n keys, the plan tree's visual style, the order of languages in the picker.

## Later
Slice 4: options (collation, hint, allowDiskUse, read preference; #75), hint selector and suggested-index link for explain.
