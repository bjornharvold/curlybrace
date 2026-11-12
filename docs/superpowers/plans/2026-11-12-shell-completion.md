# Shell completion and syntax check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Suggest names while typing in the Shell tab's editor, and check the script's shape before a Run.

**Architecture:** Two pure, egui-free viewmodel modules (`shell_check`, `shell_complete`) decide; the editor view draws the popup and routes keys; `ShellState` gets one notice and two actions for the check. No new dependency, no core change.

**Tech Stack:** Rust, egui 0.32 / eframe, egui_kittest (accesskit + snapshot tests).

**Spec:** `docs/superpowers/specs/2026-11-12-shell-completion-design.md`

## Global Constraints

- Every user-visible string is an i18n key in `crates/ui/locales/en.json` (nested under `shell.complete.*`, `shell.check.*`); report new keys. Method and operator names are code, not translated.
- Immutability: state transitions return new values. Files 200–400 lines (800 max); tests in sibling `*_tests.rs` files like the rest of the crate.
- TDD, coverage ≥ 80 %. Prefer real objects over mocks.
- Viewmodel modules import no egui. No new crate dependency.
- Never alter the user's text except by an explicit pick; Enter/Tab belong to the popup only while it is open with a selected item.
- Do not edit `CHANGELOG.md` (release-please) or the specs.

## Review Focus

- Caret inside a string, template text, comment or regex literal: no popup, no check noise.
- A collection name that is not a plain identifier (`my-coll`, `1st`, empty-looking, with quotes): inserted as `db.getCollection("…")`, quotes and backslashes escaped.
- Multi-byte text before the caret (accents, emoji): byte/char conversion at the caret and the replace range never splits a character.
- A broken draft with a good selection: Run checks only the selection; Run with no selection checks all.
- A valid script containing a regex with a quote or bracket (`/["(]/`) is not reported, or if reported, **Run anyway** works.

---

### Task 1: Syntax check

**Files:** Create `crates/ui/src/viewmodel/shell_check.rs`, `shell_check_tests.rs`; modify `viewmodel/mod.rs`.

**Interfaces:**
- Produces: `pub enum ProblemKind { UnclosedBracket(char), UnexpectedClosing(char), MismatchedClosing { opener: char, found: char }, UnterminatedString, UnterminatedTemplate, UnterminatedComment }`; `pub struct SyntaxProblem { pub kind: ProblemKind, pub range: Range<usize> /*bytes*/, pub line: usize /*1-based*/, pub column: usize /*1-based, chars*/ }`; `pub fn check(text: &str) -> Option<SyntaxProblem>` (the first problem in text order).

- [ ] **Step 1: tests** — `check("db.a.find({})")` is `None`; `"db.a.find({"` is `UnclosedBracket('{')` at the `{` (line 1, column 11); `"x )"` is `UnexpectedClosing(')')`; `"( ]"` is `MismatchedClosing{opener:'(',found:']'}`; `"'abc"` and `"`a${b}"` and `"/* x"` give the unterminated kinds; brackets inside strings, template text, line and block comments and regex literals (`/[(]/`) are not counted; a `${ … }` placeholder's brackets are counted; CRLF input reports the right line; a multi-byte char before the problem gives the right column.
- [ ] **Step 2: run, expect FAIL.**
- [ ] **Step 3: implement `check`** with one left-to-right scan and a bracket stack, using `shell_format::{starts_regex, regex_end}` for regex literals so format, highlight and check agree on what is a literal.
- [ ] **Step 4: run, expect PASS.** Add a parity test over a corpus of scripts (the highlighter's test inputs) asserting `check` agrees with `shell_highlight::tokens` on where strings and comments are.
- [ ] **Step 5: commit** `feat(shell): check a script for unclosed brackets and strings`.

### Task 2: Completion data and context

**Files:** Create `viewmodel/shell_complete.rs`, `shell_complete_data.rs`, `shell_complete_tests.rs`; modify `viewmodel/mod.rs`.

**Interfaces:**
- Produces: `pub enum Place { StatementWord, ShowArg, DbMember, CollectionMember, Operator }`; `pub struct Suggestion { pub label: String, pub insert: String, pub kind: SuggestionKind }` with `SuggestionKind { Collection, DbMethod, CollectionMethod, Operator, Word }`; `pub struct Completion { pub replace: Range<usize> /*bytes*/, pub items: Vec<Suggestion> }`; `pub fn complete(text: &str, caret: usize, collections: &[String]) -> Option<Completion>`. `collections` is the union of loaded collection names, already sorted and deduplicated by the caller.
- `shell_complete_data.rs`: `pub const` slices of statement words, `show` arguments, db methods, collection methods, query/update/aggregation operators (the mongosh API of the version floor in `curlybrace_core::tools`).

- [ ] **Step 1: tests** — one per row of the spec table; the prefix filters case-insensitively and caps at 12; `db.` offers collections before db methods; `db.users.` offers collection methods; `{ $ma` offers `$match`; no result in a string, comment, template text or regex literal; no result when the word before the caret is empty and the caret does not follow `.` or `$`; replace range covers only the typed prefix (not the `db.`), except the `getCollection` rewrite below; collection `my-coll` inserts `db.getCollection("my-coll")` with `replace` covering `db.` plus the prefix; `a"b` inserts `db.getCollection("a\"b")`; caret after multi-byte text keeps byte offsets on char boundaries.
- [ ] **Step 2: run, expect FAIL.**
- [ ] **Step 3: implement** `complete` on top of a small backward scan from the caret (identifier chars, `.`, `$`) after confirming the caret is in code via the same lexical rule as Task 1 (factor the shared "is this byte in code" helper into `shell_format`, tested there).
- [ ] **Step 4: run, expect PASS.**
- [ ] **Step 5: commit** `feat(shell): suggest names and operators at the caret`.

### Task 3: Check on Run (state and reducer)

**Files:** Modify `viewmodel/shell.rs`, `app/shell_actions.rs`, `views/shell_notice.rs`; tests in the existing `shell_tests.rs` and `shell_actions` test files.

**Interfaces:**
- Consumes: `check`, `SyntaxProblem` (Task 1).
- Produces: `ShellNotice::SyntaxProblem(SyntaxProblem)`; `ShellAction::RunAnyway`, `ShellAction::GoToProblem`; `ShellState::run_requested` returns the notice and no request when the text to run has a problem; `run_anyway()` skips the check once.

- [ ] **Step 1: tests** — a script with a problem: `Run` sends no command and sets the notice; `RunAnyway` starts the run exactly as `Run` would (including the production confirmation and every other guard of `shell_actions`); `GoToProblem` sets the selection to the problem range; with a good selection inside a broken script `Run` runs; edit or format clears the notice as the other notices do.
- [ ] **Step 2: run, expect FAIL.** **Step 3: implement.** **Step 4: run, expect PASS.**
- [ ] **Step 5: commit** `feat(shell): stop a run whose script is visibly broken`.

### Task 4: Editor popup

**Files:** Create `views/shell_complete.rs` (popup), `shell_complete_tests.rs`; modify `views/shell_editor.rs`, `views/shell.rs`, `app/panels.rs`, `app/shell_actions.rs`.

**Interfaces:**
- Consumes: `complete` (Task 2). `ShellViewContext` gains `collections: &'a [String]` (built in `show_shell_tab` from `state.sidebar.databases`, loaded ones only).
- Produces: `ShellAction::Complete { script: String, selection: Option<Range<usize>> }` — or reuse `Edit` if the picked text can be applied as one; the builder picks the smaller change and records why.

- [ ] **Step 1: tests (egui_kittest)** — typing `db.us` shows the list with `users`; Down + Enter inserts and closes it and puts the caret after the insertion; Esc closes it and leaves the text; Enter with the list closed inserts a newline; Tab with the list closed indents; `Ctrl+Space` opens it on an empty word; the list is exposed to accesskit (role list, items labelled, selected item marked) and has a label key; the popup stays inside the window near the bottom edge.
- [ ] **Step 2: run, expect FAIL.**
- [ ] **Step 3: implement**: compute `complete` from the editor's caret each frame the text or caret changed; keep the open list and selected index in egui temp data keyed by `editor_id(session).with("complete")`; consume Up/Down/Enter/Tab/Esc only while open (`ui.input_mut`, before the `TextEdit` is shown so the keys do not reach it).
- [ ] **Step 4: run, expect PASS.**
- [ ] **Step 5: commit** `feat(shell): completion list in the editor`.

### Task 5: Live problem line, strings, accessibility, docs

**Files:** Modify `views/shell.rs`, `locales/en.json`, `README.md`; add snapshots under the existing UI snapshot folder; modify `crates/ui/tests/accessibility_shell.rs`.

- [ ] **Step 1: tests** — the status line under the editor shows the first problem of the whole editor (line and column) and nothing for a good script; the notice shows message, **Go to problem** and **Run anyway**; the accessibility test covers the popup and the notice; snapshots for the list open and for the notice.
- [ ] **Step 2: run, expect FAIL.** **Step 3: implement** the line (cached by script hash like the highlight) and add every key (`shell.complete.list_label`, `shell.complete.item_collection` …, `shell.check.*` one per `ProblemKind`, `shell.check.go_to`, `shell.check.run_anyway`). **Step 4: run, expect PASS**; regenerate baselines with the repo's documented snapshot command and review each image.
- [ ] **Step 5:** README: one line in the Shell tab section. Commit `feat(shell): show syntax problems and document completion`.

### Task 6: Gates

- [ ] Run the repo's full gate (fmt, clippy `-D warnings`, `cargo test --workspace`, the gated integration script) and record the totals in the PR body. No core change is expected; if the gated shell suites are untouched, say so.
