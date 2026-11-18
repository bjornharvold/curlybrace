# Denser Browse header Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** With More (and Query options) open the Browse page's first document row stays on screen: at
or above y=312 at 1000x700 and y=440 at 800x500 after PR A, y=275 and y=390 after PR B (full budget
table in the spec). "Compare" stays unelided in the sidebar switcher (guarded by a test). The other
pages take the global density step in PR B.

**Architecture:** PR A fixes two layout bugs and shortens three labels; no state or reducer change.
The More row lays its fields in fixed-height cells (as `query_row` does) and measures the Query options
toggle instead of guessing it; the verdict takes what is left of its row; the two switch labels and
"Select all" get shorter with the full sentence as hover text and accessible description. PR B lowers
`CONTROL_HEIGHT` and `ROW_GAP` and re-derives the page constants that assume today's values.

**Tech Stack:** Rust 1.87 (no let-chains), egui/eframe 0.32, egui_kittest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-18-browse-header-design.md` (measured state, budgets, owner
questions). Earlier: plan `indexed-watching-bird` (#150), `2026-11-18-find-query-options-plan.md` (#178).

## PR split

- **PR A, `feat(browse): fit the header with More open`** (Tasks A1-A7): switcher guard, More row,
  verdict row, shorter labels, budgets, snapshots. About 40 PNGs change.
- **PR B, `feat(ui): 28 pt controls and the pages that assume 32`** (Tasks B1-B6): global density,
  page constants, the other pages at 800x500, tighter budgets. Nearly all 126 PNGs change.

PR B starts from merged PR A. Each PR targets `main` from its own branch (never commit on `main`).

## Global Constraints

- `cargo +1.87.0` always (CI pins 1.87: no let-chains). Git as plain separate Bash calls.
- Files: new files 200-400 lines at most, never over 400. Do not grow files already over 400:
  `app/panels.rs` (625), `views/documents.rs` (610). `views/findbar.rs` is 358: new code for the More
  row goes into `views/findbar_more.rs` (76) or a new `views/findbar_cell.rs`, not into `findbar.rs`.
  `wc -l` every touched file before each commit.
- Immutable style: view state changes go through viewmodel ops that return new values; widths and
  budgets are pure functions with unit tests. Real objects over mocks (the `browsing` fixture, the
  in-memory store, `FindBarState { .. }`).
- TDD per step: write the test, run it and see it fail (or, for a guard of existing behaviour, see it
  pass and say so), implement, run it green. Coverage of new code at least 80%.
- i18n: every visible string, tooltip and accessible description is a key written as a literal
  (`i18n_usage_tests.rs`); `crates/ui/locales/en.json` nested and alphabetical at every level
  (`en_json_is_nested_and_alphabetical_at_every_level`). Report new keys in the PR body.
- Layout tests use accesskit bounds on the in-memory store (the `browse_layout.rs` harness). No GPU, no
  database.
- **Never connect to any MongoDB already on the machine.** Running the app by hand uses a throwaway
  `mongod` on a spare port with a temp dbpath, stopped by its own pid and deleted (recipe in
  `2026-11-18-find-query-options-plan.md`, "Global Constraints"). `scripts/container-tests.sh` only
  touches containers it creates.
- Commits: conventional, ending with the session's `Co-Authored-By` / `Claude-Session` trailers. Never
  push without the owner. Never touch `CHANGELOG.md`.

## Snapshots: how they are regenerated and reviewed

- Run (macOS with a GPU):
  `CURLYBRACE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture`
  and the same for `--test find_options_snapshots`, `--test aggregate_snapshots`,
  `--test code_export_snapshots`, `--test theme_gallery`. A failure writes `<name>.new.png` and
  `<name>.diff.png` next to the baseline in `crates/ui/tests/snapshots/`.
- List what changed: `ls crates/ui/tests/snapshots/*.new.png`. Compare it with the expected list of the
  task. **Anything outside the list is a regression to fix, not to accept.**
- Look at every changed pair (open the `.png` and the `.new.png`, and the `.diff.png`) in light and
  dark. Check: nothing clipped or overlapping, labels whole, the first row higher, focus rings visible.
- Accept deliberately: re-run with `UPDATE_SNAPSHOTS=1` added (it rewrites only images above the
  `snapshot_options()` pixel threshold), then re-run without it and see it pass.
- **Never commit `*.new.png` or `*.diff.png`.** Before each commit: `git status --short
  crates/ui/tests/snapshots` shows only modified `*.png` baselines; delete stray `*.new.png` /
  `*.diff.png` files. The snapshot commit is its own commit (`test(ui): regenerate snapshot baselines
  for ...`).
- CI's `ui-snapshots` job (macos-15, `continue-on-error`) only runs `--test snapshots`; run the other
  snapshot binaries locally.

---

## PR A: Browse header

### Task A1: Guard the switcher and write the Browse budgets (red)

**Files:** `crates/ui/src/views/compare/switcher_tests.rs` (32 lines), new
`crates/ui/tests/browse_header_budgets.rs` (about 200 lines), `crates/ui/tests/browse_layout.rs`,
`crates/ui/tests/find_options_layout.rs`.

- [ ] Test `compare_is_laid_out_in_full_at_the_default_sidebar_width` (switcher_tests): draw
      `show_workspace_switcher` in a `SidePanel` at `SIDEBAR_DEFAULT_WIDTH`; the Compare radio's rect
      width >= the label's width in `medium_font` + 2 x `SEGMENT_PADDING` (expose the constant
      `pub(crate)`). Passes today (76.3 = 56.3 + 20): a guard, say so in the commit.
- [ ] `browse_header_budgets.rs`: one harness taking a size and a `FindBarState` adjust (copy of
      `browse_layout.rs`'s, parameterised). Tests, each asserting the first row's top <= budget with the
      measured value in the message:
      - `more_closed_1000x700` <= 260, `more_open_1000x700` <= 312,
        `more_and_query_options_open_1000x700` <= 590 (`options_open: Some(true)`, nothing set),
      - `more_closed_800x500` <= 336, `more_open_800x500` <= 440.
      Budgets are named constants with a doc comment saying what sits above the row.
- [ ] Run: all five fail with today's 277, 438, 715, 391, 556.
- [ ] `browse_layout.rs`: `FIRST_ROW_BUDGET` refers to the new file's 1000x700 closed budget (or is
      removed there in favour of the new test); keep its shared-row tests.
- [ ] `find_options_layout.rs`: delete
      `the_first_row_with_more_open_is_no_lower_than_the_header_alone_puts_it` (the budgets replace the
      documented baseline) and point its `FIRST_ROW_BUDGET` at the new 1000x700 budget.
- [ ] Commit `test(browse): budgets for the first row with More open` (red tests marked
      `#[ignore = "red until Task A5"]` so the branch stays green; A5 removes the ignore).

### Task A2: An aligned More row that is one row at 1000

**Files:** `crates/ui/src/views/findbar_more.rs`, `crates/ui/src/views/findbar_options.rs`
(drop `TOGGLE_ROW_SHARE`), new `crates/ui/src/views/findbar_cell.rs` (the fixed-height field cell and
the row height, shared with `findbar.rs::query_row`), new `crates/ui/src/views/findbar_more_tests.rs`.

Tests first:
- [ ] Unit (pure) `json_width(available, gap, toggle)`:
      - 703 available, 8 gap, 134 toggle -> both JSON fields >= 160 and the row sum <= 703 (one row);
      - same with a 160 pt "Query options (3)" toggle -> still one row;
      - 504 available -> the row will wrap: Sort and Projection each `(504 - 8) / 2`;
      - never wider than `available`, never below `JSON_FIELD_MIN_WIDTH = 160` unless `available` is.
- [ ] kittest at 1000x700, More open: Sort, Projection, Skip, Limit and the Query options toggle tops
      within 1 pt of each other; again with two options set (toggle shows the count).
- [ ] kittest at 800x500, More open: two rows; tops within 1 pt inside each row; every control's
      `right() <= ` the central panel's right edge (a wrapped `ui.vertical` overflowed in the prototype).
- [ ] kittest: the Tab order from Filter to the Query options toggle is Filter, Find, Clear, Copy as
      code, Previous, Next, More options, Sort, Projection, Skip, Limit, Query options.
- [ ] Existing: `findbar.rs` idle-frame test (state unchanged), `find_options_layout.rs`
      (no overlap at 800x500 and 1000x700, the section adds at most its share) stay green.
- [ ] Run: alignment and width tests fail (staircase 114 / 129 / 137 / 140; toggle on its own row).

Implement:
- [ ] `findbar_cell.rs`: `field_row_height(ui)` (`CONTROL_HEIGHT` + 2 x half the Small text height)
      and `field_cell(ui, width, height, add)` using `allocate_ui_with_layout(vec2(width, height),
      Layout::top_down(Align::Min), ..)` with `set_min_height` (not `ui.vertical`, which cannot wrap).
      `query_row` uses the same helpers (no behaviour change there).
- [ ] `show_more_fields`: `set_row_height(field_row_height)`, each field in a `field_cell`; the toggle's
      width measured into egui temp memory (`ui.id().with("findbar_more_toggle_width")`) with
      `request_discard` when it changes by more than 0.5 pt, as `query_row` does; `json_width` takes it.
- [ ] Run green; `wc -l`; commit `fix(findbar): align the More row and keep it to one row at 1000 pt`.

### Task A3: Shorter switch labels with full descriptions

**Files:** `crates/ui/src/views/findbar.rs` (`show_automatic_verdict`, `show_guard_setting`: about 6
lines), `crates/ui/locales/en.json`, `crates/ui/tests/accessibility_findbar.rs` (two literal
constants), `site/index.html:327`.

- [ ] Tests: each checkbox's accesskit description equals `t("query.verdict.automatic_hint")` /
      `t("query.guard.setting_hint")`; hovering shows the same text; label is `t(..)` of the key.
- [ ] Run: fail (no description yet).
- [ ] en.json: `query.verdict.automatic` -> "Check index use", `query.guard.setting` -> "Warn before
      full scans", new `query.verdict.automatic_hint` (alphabetical after `automatic`). Set
      `on_hover_text` + `accesskit_node_builder(.., set_description)` for both.
- [ ] Update the two literal `LABEL` constants in `accessibility_findbar.rs` and the quoted setting name
      on the site.
- [ ] Run green (incl. `i18n_usage_tests`, the en.json order test); commit
      `feat(browse): shorter labels for the two query switches`.

### Task A4: The verdict shares the switches' row when it fits

**Files:** `crates/ui/src/views/verdict_badge.rs` (one line + doc), `crates/ui/src/views/verdict_badge_tests.rs`.

- [ ] Tests (kittest, explain on, a verdict present and absent):
      - at 1000x700 the reserved verdict area's top equals the checkbox row's (the chip's centre y
        within 2 pt of the "Check index use" box's), and the toolbar's y is identical with and without a
        verdict;
      - at 800x500 the verdict area starts a new line (not enough room), the toolbar's y again identical
        with and without a verdict;
      - `accessibility_verdict.rs` stays green (fixed row height).
- [ ] Run: the 1000x700 test fails (verdict on its own line, 44 pt lower).
- [ ] `show_verdict_row`: `ui.available_size_before_wrap().x.max(MIN_ROW_WIDTH)`.
- [ ] Run green; commit `fix(verdict): share the switches' row when it fits`.

### Task A5: "Select all" and the budgets go green

**Files:** `crates/ui/src/views/doc_select.rs`, `crates/ui/locales/en.json`, `docs/copy.md`,
`crates/ui/tests/browse_header_budgets.rs` (remove the ignores).

- [ ] Tests: the button's label is `t("doc.select_all_result")`, its description and hover text
      `t("doc.select_all_result_hint")`; at 800x500 the toolbar spans at most two rows (Tree and Copy
      tops differ by at most one row pitch); at 1000x700 at most two rows.
- [ ] Run: fail (three rows at 800x500; no description).
- [ ] en.json: `doc.select_all_result` -> "Select all", new `doc.select_all_result_hint` (after it).
      `docs/copy.md` names the button "Select all" and explains it selects the whole query.
- [ ] Remove the `#[ignore]`s from A1; run the budgets: expect about 253 / 305 / 582 / 329 / 433.
- [ ] Commit `feat(browse): a two-row documents toolbar at 800 pt`.

### Task A6: Snapshots for PR A

Expected to change (verify by the `*.new.png` list): `browser_table_view`, `browser_table_view_dark`,
`browser_tree_view`, `browser_json_view`, `browser_verdict_light`, `browser_verdict_dark`,
`guard_notice`, `guard_notice_dark`, `browser_find_options`, `browser_find_options_dark`,
`browser_find_options_small`, the `sessions_tree_*` four (they show the Browse page), and the dialogs
drawn over the Browse page: `bulk_*` (28), `copy_dialog_*`, `copy_progress`, `copy_report_*`,
`delete_confirmation`, `editor_modal*`, `production_write_warning`, `code_export_dialog*`,
`error_banner`. **Must not change:** `connection_*`, `compare_*`, `indexes_tab*`, `queries_view*`,
`index_*`, `shell_*`, `aggregate_*`, `theme_gallery_*`, `host_key_prompt*`, `saved_copies*`.

- [ ] Regenerate and review as in "Snapshots" above (light and dark; More open in
      `browser_find_options*`: one aligned row at 1000, two at 800).
- [ ] Commit `test(ui): regenerate snapshot baselines for the denser browse header`.

### Task A7: Verify PR A

- [ ] `cargo +1.87.0 fmt --all --check`; `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`;
      `cargo +1.87.0 test --workspace --locked`.
- [ ] `scripts/container-tests.sh` (labels changed; own containers only).
- [ ] Run the app (`run` skill) against a throwaway `mongod` (never an existing one) at 1000x700 and
      800x500: Compare unelided; More open shows one aligned row at 1000, two at 800; the first row is
      on screen in both; Query options opens and scrolls; Tab order as in A2; hover texts on the three
      relabelled controls.
- [ ] Code review loop to a fixpoint (`code-review`), re-walking every caller of `field_cell`,
      `show_verdict_row` (Browse only) and the relabelled keys.
- [ ] PR body: measured before / after table, new and changed i18n keys, snapshot list, test plan.

---

## PR B: global density and the other pages

### Task B1: 28 pt controls (and a 4 pt row gap if the focus ring allows it)

**Files:** `crates/ui/src/theme/widgets/mod.rs` (`CONTROL_HEIGHT`), `crates/ui/src/theme/install.rs`
(`ROW_GAP`), `crates/ui/src/theme/widgets/control_tests.rs`, `tabs_tests.rs`, `widgets_tests.rs`,
`crates/ui/tests/sidebar_refresh_button.rs`.

- [ ] Tests first: a button, a tab, a segmented option and a single-line outlined field are each 28 pt
      tall; the field's text row fits inside with at least 4 pt above and below; the floating label's
      overhang is unchanged; two stacked focused buttons with the theme's row gap: the upper ring's
      rect does not intersect the lower button's rect; `sidebar_refresh_button` asserts
      `CONTROL_HEIGHT`, not 32.0.
- [ ] Run: fail (32 pt).
- [ ] `CONTROL_HEIGHT = 28.0`; `ROW_GAP = 4.0`. If the ring test fails at 4, set `ROW_GAP` back to 6,
      keep the test at 6, and use the fallback budgets of the spec.
- [ ] Run the widget tests green; commit `feat(theme): 28 pt controls`.

### Task B2: Constants that assume 32 / 6

**Files:** `crates/ui/src/views/shell.rs` (`CHROME_GUESS`, `SPACING_SLACK`: two row gaps),
`crates/ui/src/views/shell_guard.rs`, `crates/ui/src/views/connections_edit_dialog.rs`
(`CHROME_HEIGHT`), `crates/ui/tests/shell_layout.rs`, `crates/ui/tests/indexes_layout.rs`
(`LIST_BUDGET` 120 -> 106), `crates/ui/tests/aggregate_layout.rs`, `crates/ui/tests/browse_header_budgets.rs`.

- [ ] Run the whole UI test suite after B1; list every failure. For each, decide: a constant derived
      from the old height (fix the constant, preferably to an expression of `CONTROL_HEIGHT` /
      `ROW_GAP`) or a real regression (fix the layout).
- [ ] Shell: re-measure the chrome with History open at 800x500 and 1280x800 and set `CHROME_GUESS`
      and `SPACING_SLACK` (= 2 x `ROW_GAP`); `shell_layout.rs` green.
- [ ] Edit dialog: at 800x500 with a long test error and a validation error the footer stays on screen
      (existing test); derive `CHROME_HEIGHT` from the new heights.
- [ ] Tighten the Browse budgets to 228 / 275 / 545 / 296 / 390 (or the spec's `ROW_GAP` 6 fallback).
- [ ] Commit `fix(ui): page constants for the 28 pt controls`.

### Task B3: The other pages at 800x500

**Files:** only where a check fails: `views/indexes.rs`, `views/queries.rs`, `views/compare/*.rs`,
`views/shell.rs`, `views/connections.rs`, `views/aggregate*.rs`, dialogs.

- [ ] Add 800x500 cases to the existing layout tests that only check 1000x700: `indexes_layout.rs`
      (list heading <= 118), `aggregate_layout.rs` (its budget at 800x500), and a Compare setup check
      (heading row and pickers on screen, no overlap) in `compare_restyle.rs` or a new
      `compare_layout.rs`.
- [ ] Connections manager and the connection edit dialog at 800x500: heading row one line, footer
      visible (existing tests, re-run).
- [ ] Fix only what fails; each fix its own commit (`fix(<page>): ...`).

### Task B4: Snapshots for PR B

Expected: nearly every baseline changes (controls are 4 pt shorter). **Must look at:** every page in
light and dark, the theme gallery (all control states, focus rings not clipped), dialogs at their sizes
(`copy_dialog_overwrite_production` at 1000x1100, `index_builder` at 1000x1000).

- [ ] Regenerate, list, review, accept, re-run as in "Snapshots"; commit
      `test(ui): regenerate snapshot baselines for 28 pt controls`.

### Task B5: Docs

- [ ] README and site screenshots only if they show these pages (check `site/` and `assets/`); no new
      docs files.

### Task B6: Verify PR B

- [ ] fmt, clippy `-D warnings`, `test --workspace --locked`, the five snapshot binaries,
      `scripts/container-tests.sh`.
- [ ] Run the app against a throwaway `mongod` at 800x500 and 1000x700: every page (Browse with More
      and Query options, Indexes, Queries, Aggregate, Compare, Shell, Connections, edit dialog, bulk and
      copy dialogs): nothing clipped or overlapping, keyboard focus visible on every control, the first
      document row at or above the budgets.
- [ ] Code review loop to a fixpoint; re-walk every user of `CONTROL_HEIGHT` / `item_spacing.y`
      (`rg -n "CONTROL_HEIGHT|item_spacing" crates/ui/src`, about 30 files) and say for each what 28 / 4
      means there.
- [ ] PR body: before / after table for every page, the touch-target note, the snapshot list.

## Done when

- PR A: the five budget tests pass at 260 / 312 / 590 / 336 / 440; the switcher test passes; the
  More row is one aligned row at 1000 and two at 800; no `*.new.png` committed.
- PR B: budgets 228 / 275 / 545 / 296 / 390 (or the `ROW_GAP` 6 fallback); every page's layout test
  green at 1000x700 and 800x500; snapshots reviewed and committed.
