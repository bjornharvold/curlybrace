# Export file lifecycle refactor (PR 0 of #54) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the generic export-file code (path validation, owner-only create with remove-on-drop, the buffered sink and their test hooks) out of `runtime/compare` into a shared module, with no behavior change.

**Architecture:** A new `crates/core/src/runtime/export_file/` module owns everything about writing a file safely. `runtime/compare` keeps only what is about compare's own documents (the in-place header overwrite, sections, tally, format writers) and imports the rest. This lets the document export (PRs 1 to 3) reuse the same security-sensitive code instead of copying it.

**Tech Stack:** Rust 1.87 (CI pin: verify with `cargo +1.87.0`, no let-chains), tokio, `tempfile` (tests), `libc` (unix).

**Spec:** `docs/superpowers/specs/2026-11-25-export-documents-design.md` (section "The plan in four PRs", PR 0). Later PRs get their own plans.

## Global Constraints

- Behavior-preserving: no production logic changes, no new behavior. Moved code is moved verbatim except visibility, paths and the one signature change in Task 2.
- Visibility of moved items: `pub(in crate::runtime)` (reachable by `runtime::compare` and the future `runtime::export`, nowhere else).
- Test-only hooks stay behind `#[cfg(test)]`; none may exist in a non-test build.
- Paths, system error text and file contents are never logged or carried in an error (unchanged; do not add any `Debug`/log of them).
- Every moved test keeps its name and assertions; compare's existing tests pass unmodified except import paths.
- Files under 800 lines; commit messages `refactor(export): ...`, lowercase, imperative.
- Run Cargo with the CI commands: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.

## Review Focus

- A symlink, hard link or FIFO at the chosen path is still refused and the other file is never truncated (tests move with `FileGuard`; Task 1 re-runs them in the new module).
- A failure right after the file opens still removes the file, and a failure to remove it still reports `ExportFileNotRemoved` (the `fail_after_open(path, true)` test, which locks the parent directory; it must restore permissions so the temp dir can be deleted).
- A write failing mid-export (`fail_writes_after`) still leaves no file for both the plan/script export and the CSV report job (Task 2 re-runs `export_lifecycle_tests` and `report_lifecycle_tests`).
- No test hook is compiled into a release build (Task 3: `cargo build --release -p curlybrace-core` has no warnings, and `grep` finds no `test_hooks` outside `cfg(test)`).
- The non-unix `cfg` branches (`open_checked`, `restrict`) are moved verbatim and still compile in principle (Task 1: keep both `#[cfg(unix)]` and `#[cfg(not(unix))]` variants; clippy on Linux/macOS cannot check Windows, so review the diff of those two functions by eye).

---

## File structure

| File | Responsibility |
| --- | --- |
| `runtime/export_file/mod.rs` (new) | Module doc; re-exports `validate_path`, `FileGuard`, `ExportSink`, `Out`, `buffered`, `into_file`; declares submodules |
| `runtime/export_file/path.rs` (new) | `validate_path` |
| `runtime/export_file/guard.rs` (new) | `FileGuard`, `open_checked`, `restrict`, `prepare` |
| `runtime/export_file/sink.rs` (new) | `ExportSink`, `Out`, `buffered`, `into_file`, `BUFFER` |
| `runtime/export_file/test_hooks.rs` (new, `cfg(test)`) | `fail_after_open`, `fail_writes_after` and their statics |
| `runtime/export_file/guard_tests.rs` (new, moved) | The path and guard tests from `compare/export_file_tests.rs` |
| `runtime/compare/export_header.rs` (renamed from `export_file.rs`) | `overwrite_header` only |
| `runtime/compare/export_header_tests.rs` (renamed from `export_file_tests.rs`) | The two `overwrite_header` tests and their helpers |
| `runtime/compare/export_sink.rs` | Keeps compare-specific items; imports `ExportSink`/`Out`/`buffered` from `export_file`; keeps a wrapper `into_file` returning `RenderError` |
| `runtime/compare/export.rs`, `report_job.rs` | Import paths only |
| `runtime/compare/export_lifecycle_tests.rs`, `report_lifecycle_tests.rs` | Import path of `fail_writes_after` only |
| `runtime/mod.rs` | `mod export_file;` (next to `mod copy;`) |

---

### Task 1: Move path validation and `FileGuard` into `runtime/export_file`

**Files:**
- Create: `crates/core/src/runtime/export_file/{mod.rs,path.rs,guard.rs,test_hooks.rs,guard_tests.rs}`
- Modify: `crates/core/src/runtime/mod.rs` (add `mod export_file;`), `compare/export_file.rs` (remove moved items), `compare/export_file_tests.rs` (remove moved tests), `compare/export.rs:58`, `compare/report_job.rs:63`

**Interfaces:**
- Produces (all `pub(in crate::runtime)`):
  - `fn validate_path(path: &Path) -> crate::error::Result<()>` in `export_file::path`, re-exported from `export_file`.
  - `struct FileGuard` with `fn create(path: &Path) -> Result<(File, Self)>`, `fn commit(self)`, `fn remove(self) -> bool`, and `Drop`, in `export_file::guard`, re-exported.
  - `export_file::test_hooks::fail_after_open(path: &Path, lock_parent: bool)` (`#[cfg(test)]`).
- Consumes: `CoreError::{Unsupported, ExportWriteFailed, ExportFileNotRemoved}` (unchanged).

- [x] **Step 1: Move the tests first.** Create `export_file/guard_tests.rs` with every test from `compare/export_file_tests.rs` EXCEPT the two `overwrite_header_*` tests and their helpers `file_with` and `contents` (those stay). Change its imports to `use super::{FileGuard, validate_path};` and the hook path to `super::test_hooks::fail_after_open`. Declare it in `export_file/mod.rs` as `#[cfg(test)] mod guard_tests;`.
- [x] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib runtime::export_file` → fails to compile (`FileGuard`, `validate_path`, `test_hooks` not found in the new module).
- [x] **Step 3: Move the code.** Cut `validate_path` (and `invalid_path`) into `path.rs`; `FileGuard`, `prepare`, `open_checked` (both cfg variants) and `restrict` (both cfg variants) into `guard.rs`, importing `invalid_path` from `path.rs` (make it `pub(super)` there); the `#[cfg(test)] test_hooks` module into `test_hooks.rs` (keep `inject` callable from `guard.rs` as `test_hooks::inject`; keep `fail_after_open` as `pub(in crate::runtime)`). `mod.rs` re-exports `validate_path` and `FileGuard` with `pub(in crate::runtime) use`. Move the module doc comment of `compare/export_file.rs` that describes path, permissions and removal into `mod.rs` (drop the sentence about "replacing the header"). Leave `overwrite_header` in `compare/export_file.rs` for now.
- [x] **Step 4: Rewire.** `runtime/mod.rs`: `mod export_file;`. In `compare/export.rs` and `compare/report_job.rs` import `FileGuard`/`validate_path` from `crate::runtime::export_file`; `overwrite_header` still from `super::export_file`. In `compare/export_file_tests.rs` keep only the header tests, importing `overwrite_header` from `super::export_file`.
- [x] **Step 5: Verify.** `cargo test -p curlybrace-core --lib runtime::export_file runtime::compare` → all pass (the moved tests now run under `runtime::export_file::guard_tests`, same names). `cargo clippy -p curlybrace-core --all-targets --locked -- -D warnings` clean.
- [x] **Step 6: Commit.** `git add -A crates/core && git commit -m "refactor(export): move path validation and the file guard out of compare"` (end with the `Co-Authored-By` trailer from the session's attribution instructions).

---

### Task 2: Move the buffered sink and its test hook

**Files:**
- Create: `crates/core/src/runtime/export_file/sink.rs`
- Modify: `export_file/mod.rs`, `export_file/test_hooks.rs`, `compare/export_sink.rs`, `compare/report_job.rs:64`, `compare/export_lifecycle_tests.rs:347`, `compare/report_lifecycle_tests.rs:9`

**Interfaces:**
- Produces (`pub(in crate::runtime)`, re-exported from `export_file`):
  - `struct ExportSink` with `fn new(file: File, path: &Path) -> Self` and `impl std::io::Write`.
  - `type Out = BufWriter<ExportSink>`; `fn buffered(sink: ExportSink) -> Out` (64 KiB buffer, `const BUFFER: usize = 64 * 1024`).
  - `fn into_file(out: Out) -> std::io::Result<File>` (**signature change**: the old one returned `Result<File, RenderError>`).
  - `export_file::test_hooks::fail_writes_after(path: &Path, bytes: u64)` (`#[cfg(test)]`).
- Compare keeps `export_sink::into_file(out: Out) -> Result<File, RenderError>` as a one-line wrapper over the shared one: `export_file::into_file(out).map_err(|e| RenderError::Io(e.kind()))`, so its call sites (`FormatWriter::finish`, `report_job.rs:308`) do not change.

- [x] **Step 1: Write the failing test.** In `export_file/sink_tests.rs` (`#[cfg(test)] mod sink_tests;` in `mod.rs`), add:
  - `buffered_writes_reach_the_file_after_into_file`: create a temp file, `ExportSink::new(file, &path)`, `buffered(sink)`, write `b"hello"`, `into_file(out).unwrap()`, assert the file content is `hello`.
  - `a_write_failure_injected_for_the_path_surfaces_from_into_file`: call `test_hooks::fail_writes_after(&path, 0)` before `ExportSink::new`, write `b"x"` through `buffered`, assert `into_file(out)` is `Err`.
- [x] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib runtime::export_file::sink_tests` → compile error (`ExportSink`, `buffered`, `into_file`, `fail_writes_after` not in `export_file`).
- [x] **Step 3: Move the code.** Cut `ExportSink`, its `Write` impl, `Out`, `BUFFER`, `buffered` and the generic `into_file` (returning `io::Result<File>`, mapping the error with `e.into_error()`) into `sink.rs`; move the `fail_writes_after` hook and its static into `test_hooks.rs` (the `fail_after` lookup becomes `pub(super)` there). In `compare/export_sink.rs` delete those items, import `ExportSink`, `Out`, `buffered` from `crate::runtime::export_file`, and add the `into_file` wrapper above. Update the three import paths named under Files.
- [x] **Step 4: Verify.** `cargo test -p curlybrace-core --lib runtime::export_file runtime::compare` → pass, including `export_lifecycle_tests` and `report_lifecycle_tests` (they inject mid-write failures and assert no file is left).
- [x] **Step 5: Commit.** `refactor(export): move the buffered export sink out of compare`.

---

### Task 3: Rename the leftover, update docs, full verification

**Files:**
- Rename: `compare/export_file.rs` → `compare/export_header.rs`, `compare/export_file_tests.rs` → `compare/export_header_tests.rs`
- Modify: `compare/mod.rs` (the `mod export_file;` / `mod export_file_tests;` lines become `export_header` / `export_header_tests`), `compare/export.rs` (import and the module doc line mentioning `FileGuard` keeps working; update only if it names a moved path)

**Interfaces:**
- Produces: `compare::export_header::overwrite_header(file: &mut File, expected_len: u64, header: &[u8]) -> io::Result<()>` (`pub(super)`, unchanged body). Module doc: "Replacing the header of a compare export in place once the true counts are known."

- [x] **Step 1: Rename with `git mv`**, fix the module doc of `export_header.rs` and the doc line of `export_header_tests.rs` (`//! overwrite_header on its own.`), and the `mod` lines. `cargo test -p curlybrace-core --lib runtime::compare::export_header` first fails to find the module before the `mod` edit, then passes after it.
- [x] **Step 2: No leaked hooks.** `grep -rn "test_hooks" crates/core/src | grep -v "cfg(test)"` is reviewed by eye: every definition and use sits under `#[cfg(test)]` or in a `_tests.rs` file. `cargo build --release -p curlybrace-core` prints no warnings.
- [x] **Step 3: Full verification on the CI toolchain.** `cargo +1.87.0 fmt --all --check`; `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`; `cargo +1.87.0 test --workspace --locked` → all pass; `cargo test -p curlybrace-core --test local_compare_export_files --test local_compare_export` pass (these write real files through the moved code).
- [x] **Step 4: Diff review.** `git diff main...HEAD --stat` shows moves and import edits only; `git diff main...HEAD -M` shows the moved functions unchanged except visibility, import paths and the `into_file` signature. Eyeball the `#[cfg(not(unix))]` variants of `open_checked` and `restrict`: identical to before.
- [x] **Step 5: Commit and PR.** `refactor(export): name the compare header module for what it holds`. Add `docs/superpowers/specs/2026-11-25-export-documents-design.md` and this plan to the branch (`docs: design and plan for exporting documents (#54)`). Branch `refactor/export-file-lifecycle`; push with `-u`; open the PR with a body that says "no behavior change, moves only", a test plan, and the attribution line from the session's instructions.

---

## Self-review

- Spec coverage: PR 0 of the spec (move `validate_path`, `FileGuard`, the buffered sink, `into_file` with `io::Error`, and the hooks; compare keeps `overwrite_header`, `Resolved`, `Section`, `Tally`, `FormatWriter`, `header_bytes`, `render_error`) is Tasks 1 to 3. PRs 1 to 3 are intentionally not in this plan.
- Type consistency: `into_file` is defined once (Task 2) with the shared `io::Result<File>` signature and a compare wrapper under the same name; every other name is unchanged.
- Proportion: moves only; no function body is reproduced in this plan.
