# Copy as driver code (slice 3a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** Generate runnable driver code (mongosh, Node.js, Python, Java, C#, Go, PHP, Ruby, Rust) for an aggregation pipeline or a find query, shown in a dialog with a Copy button.

**Architecture:** A pure core module `codegen`: a language-neutral value model over `bson::Bson`, one renderer per language (type-syntax table + string escaping + snippet frame), `generate(&CodeRequest) -> Result<String, CodegenError>`. The UI adds a per-session dialog state, actions, a dialog view and two entry points (Aggregate tab, Browse query bar). No server access.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, egui. No new dependency, no templating crate.

**Spec:** `docs/superpowers/specs/2026-11-15-aggregation-slice-3-design.md` (slice 3a section).

## Global Constraints

- Every user-facing string an i18n key under `codegen.*` in `crates/ui/locales/en.json` (nested); report new keys. Generated code itself is not UI text.
- Files 200-400 lines (800 max); immutable style; real objects over mocks; TDD; coverage of new code at least 80%. No new dependency.
- The snippet always connects with the placeholder `<your connection string>`; never a real URI, host, user or password; the namespace comes from the request.
- Deterministic output. Golden files under `crates/core/tests/codegen/<language>/<case>.txt`, written by hand-reviewed first run (an `UPDATE_GOLDEN=1` env switch may regenerate them; review diffs, do not regenerate blindly). Cases: `all_types` (every BSON type), `pipeline` (realistic `$match`/`$lookup`/`$group`/`$facet`), `find` (filter + sort + projection + skip + limit), `strings` (quotes, backslashes, newlines, tabs, control chars, unicode, language interpolation sigils `$` `#{}` `${}` `%` `{}`), `write_stage` (trailing `$out`), each with and without the setup/import lines.
- The preview `$limit` is never part of an exported pipeline; the exported stages are the enabled stages exactly as the run plan would send them (use the parsed documents of `pipeline` helpers; do not re-parse differently).
- Never log request or output text; errors name the language or the stage index, never content.
- `cargo +1.87.0`; run git as plain separate Bash calls; conventional commits (`feat(codegen): ...`) ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never connect to any MongoDB (none is needed in this slice).

## Review Focus

- Every BSON type renders to syntax that actually compiles/runs in that language: `ObjectId`, DateTime (incl. pre-1970 and out of range), `Decimal128`, `Int32` vs `Int64` vs double (`1.0` stays a double; NaN and infinities), Binary (all subtypes incl. UUID), regex with flags, Timestamp, MinKey/MaxKey, Code (with and without scope), Symbol, DbPointer, Undefined, Null, nested arrays/documents, empty documents and arrays, keys that need quoting, keys with `$` and `.`.
- Escaping cannot break out of a string literal or inject code in any language (quotes, backslash, newline, ` `, NUL, lone surrogates cannot occur in Rust strings but emoji and combining characters can, interpolation sigils in Ruby `#{}`, PHP `$var` in double quotes, C# `{}` in interpolated strings, JS template literals, Python f-strings are never used).
- Option handling: setup lines off gives only the operation; custom variable names are validated (identifier-safe per language, else an error) so an option cannot inject code.
- A pipeline with a write stage exports it when the option is on and omits it when off; an empty pipeline and a zero-stage selection give a clear error, not empty code.
- The clipboard text equals the preview text exactly.

### Task 1: model, frame and the first three renderers (mongosh, Node.js, Python)

**Files:** Create `crates/core/src/codegen/mod.rs`, `model.rs` (request/options/error types), `escape.rs` shared helpers, `mongosh.rs`, `node.rs`, `python.rs` (+ `_tests.rs` each as needed), `crates/core/tests/codegen_golden.rs` and fixtures `crates/core/tests/codegen/**`; modify `crates/core/src/lib.rs`.

**Interfaces — Produces:**
- `pub enum Language { Mongosh, Node, Python, Java, CSharp, Go, Php, Ruby, Rust }` with `Language::ALL: [Language; 9]` and a stable `Language::key() -> &'static str` (used for i18n and tests).
- `pub struct CodeOptions { pub setup: bool, pub client_var: String, pub db_var: String, pub collection_var: String, pub write_stage: bool }` with `Default` (setup true; names `client`, `db`, `collection`; write_stage true) and `CodeOptions::validated(self) -> Result<Self, CodegenError>` (identifier-safe: `[A-Za-z_][A-Za-z0-9_]{0,40}`, not a keyword of any of the nine languages).
- `pub enum CodeRequest { Find { ns: Namespace, query: Query }, Aggregate { ns: Namespace, stages: Vec<Document> } }` and `pub fn generate(language: Language, request: &CodeRequest, options: &CodeOptions) -> Result<String, CodegenError>`.
- `pub enum CodegenError { InvalidName(&'static str /* which option */), EmptyPipeline, UnsupportedValue(&'static str /* bson type name */) }` with a stable `error_variant()`; no content in messages.
- An internal `trait Renderer { fn value(&self, v: &Bson, indent: usize) -> String; fn find(...); fn aggregate(...); }` (shape is yours; one renderer per language, shared indentation/escape helpers in `escape.rs`).

- [ ] Failing golden tests for the three languages (all cases, setup on/off) and unit tests for `validated`, escaping helpers, empty pipeline, write-stage option.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core codegen`; commit `feat(codegen): generate mongosh, Node.js and Python code`.

### Task 2: Java, C#, Go

**Files:** Create `java.rs`, `csharp.rs`, `go.rs` (+ tests) under `crates/core/src/codegen/`, golden fixtures for the three languages; modify `mod.rs` dispatch.

**Interfaces:** Consumes Task 1's `Renderer`, `escape.rs`, `CodeOptions`. Use each driver's idiomatic BSON builders (Java: `Document`/`Arrays.asList`, `Aggregates`-free plain `Document` stages; C#: `BsonDocument.Parse`-free explicit `new BsonDocument { ... }`; Go: `bson.D`/`bson.A`/`bson.M` with `primitive` types).

- [ ] Failing golden and unit tests (same cases and Review Focus as Task 1).
- [ ] Implement; commit `feat(codegen): generate Java, C# and Go code`.

### Task 3: PHP, Ruby, Rust

**Files:** Create `php.rs`, `ruby.rs`, `rust.rs` (+ tests) under `crates/core/src/codegen/`, golden fixtures; modify `mod.rs` dispatch.

**Interfaces:** Same as Task 2. PHP: `MongoDB\BSON\*` classes with single-quoted strings (escape `\` and `'` only; never interpolate); Ruby: `BSON::ObjectId.from_string` etc., single-quoted or `%q` literals so `#{}` can never interpolate; Rust: the `bson::doc!`/`bson::bson!` macros with explicit type constructors where the macro cannot express a type (`bson::oid::ObjectId::parse_str`, `bson::DateTime::from_millis`, `Decimal128` via `parse`).

- [ ] Failing golden and unit tests (same cases and Review Focus).
- [ ] Implement; add a core test asserting `Language::ALL` has a renderer for every variant and every renderer renders the `all_types` fixture without error; commit `feat(codegen): generate PHP, Ruby and Rust code`.

### Task 4: UI dialog, entry points, i18n, layout, docs

**Files:** Create `crates/ui/src/viewmodel/code_export.rs` (+ tests), `crates/ui/src/app/code_export_actions.rs` (+ tests), `crates/ui/src/views/code_export.rs` (+ tests); modify the aggregate tab toolbar (`views/aggregate.rs`, keeping it under 400 lines: put the button in a new small file if needed), `views/findbar.rs` (a **Copy as code** button next to Find/Clear), the reducer dispatch, `crates/ui/locales/en.json` (`codegen.*`), `crates/ui/tests/` (accessibility at 1000x700 and 800x500 for the dialog; a light/dark snapshot of the dialog; regenerate only baselines that legitimately change and report others), `README.md` (a short Copy as code bullet under features; roadmap wording if #73 is listed).

**Interfaces — Consumes:** Task 1-3 `generate`, `Language`, `CodeOptions`, `CodeRequest`. **Produces:** `CodeExportUi { dialog: Option<CodeDialog> , last: (Language, CodeOptions) }` (language and options remembered per app run, not persisted); `CodeDialog { request: CodeRequest, language, options, text: Result<String, CodegenError> }`; `CodeAction::{ Open(CodeSource), SetLanguage(Language), SetSetup(bool), SetName(NameField, String), SetWriteStage(bool), Copy, Close }` where `CodeSource` is `Aggregate` or `Find`; the text is regenerated on every change (pure, cheap). `Copy` puts exactly the previewed text on the clipboard through the existing clipboard path (find how other views copy). Aggregate entry: disabled with the preview's reason when a stage does not parse or no enabled stage exists; enabled otherwise (never sends anything to the server, no connection needed). Find entry: uses the same parsed query the Find button would send; disabled with the parse error when the query text is invalid.

- [ ] Failing tests: reducer (open from both sources, switching language regenerates, an invalid variable name shows its error in place of the code and disables Copy, Copy puts the exact text on the clipboard output, Close), view (named controls, Enter copies, Escape closes, selectable preview, works at 800x500), entry points disabled/enabled matrix.
- [ ] Implement; `cargo +1.87.0 fmt --all`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`; commit `feat(codegen): add the copy as code dialog` and `docs(readme): describe copy as code`.
- [ ] The site is updated in a separate docs PR after the release, not here.
