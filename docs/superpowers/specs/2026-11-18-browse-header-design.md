# Denser Browse header (follow-up to #150 and #178)

Status: proposed. Plan: `docs/superpowers/plans/2026-11-18-browse-header-plan.md`.

## Intent

The owner asked for three things (plan `indexed-watching-bird`, partly done in #150): the sidebar's
Browse | Compare | Shell switcher shows "Compare" in full; data starts much higher on the Browse page;
every other page (Indexes, Compare, Shell, Connections, dialogs) gets the same review. Choices already
made with the owner: a slightly wider sidebar with tighter segment padding; Filter always visible with
Sort / Projection / Skip / Limit behind "More"; one toolbar row between the query bar and the results;
scope Browse, Indexes, Compare, Shell, dialogs / connection manager and global control heights and
spacing.

#150 shipped most of that. #178 then put a "Query options" toggle and section behind More, and a
reviewer's probe showed the result: with More open the first document row is off screen at 800x500.
This spec covers what is left: make the Browse header fit with More (and Query options) open, fix two
layout bugs the measurements turned up, and propose the global density step for the other pages.

## What #150 and later work already did

From `git show 5b8f3d4d` (#150) and the current code:

- `CONTROL_HEIGHT` 40 -> 32, `COMPACT_CONTROL_HEIGHT` 28 -> 24 (`theme/widgets/mod.rs:43`),
  `ROW_GAP` 8 -> 6 (`theme/install.rs:10`), `SEGMENT_PADDING` 12 -> 10 (`theme/widgets/segmented.rs:17`),
  `SIDEBAR_DEFAULT_WIDTH` 260 -> 280 (`views/sidebar_roots.rs:22`).
- `tabs_inline`: the connection header and Documents | Indexes | Aggregate share one row
  (`show_connection_header_with`, `app/browse_panel.rs`).
- Filter row: Filter + Find / Clear / Copy as code / Previous / Next / More options on one wrapping row
  whose trailing width is measured each frame (`views/findbar.rs:query_row`). More is a disclosure
  (`FindBarState.more`, opens by itself when a hidden field holds a value, counts set fields).
- Switches + verdict in one wrapping row (`show_switches_and_verdict`); the verdict keeps a fixed size.
- Documents toolbar: Tree | Table | JSON (compact) + Insert + Update / Delete matching + Select all /
  Clear selection / Copy in one wrapping row; reason line under it (`views/documents.rs:show_toolbar`).
- Indexes, Compare, Shell, Connections and the connection edit dialog were folded to fewer rows
  (`tests/indexes_layout.rs`, `tests/shell_layout.rs`, `CHROME_HEIGHT` 320 -> 270).
- Guards: `tests/browse_layout.rs` (`FIRST_ROW_BUDGET = 300` at 1000x700, More closed),
  `tests/indexes_layout.rs` (`LIST_BUDGET = 120`).

#178 added `views/findbar_options.rs`: a "Query options" toggle at the end of the More row
(`TOGGLE_ROW_SHARE = 170` reserved for it) and a framed section under it, capped at
`OPTIONS_MAX_HEIGHT_SHARE = 0.35` of the window (min 140 pt), scrolling inside, plus status lines after
the verdict. `tests/find_options_layout.rs` documents the off-screen row with
`HEADER_ONLY_BASELINE = 557` (800x500, More open) as "the header layout owns that".

## Measured current state (main at e0cb431c)

Measured from accesskit bounds of the real app on the in-memory store (the `browse_layout.rs` harness:
`browsing(ViewMode::Table)`, light theme, pixels_per_point 1, 6 frames), with a throwaway probe test
that is not in the tree. y is the top of "Select document 1".

| Window   | More closed | More open | More + Query options open (nothing set) |
|----------|-------------|-----------|------------------------------------------|
| 1000x700 | **277**     | **438**   | **715** (below the 700 window)            |
| 800x500  | **391**     | **556** (below the 500 window) | **739** (below the window)  |

This matches the reviewer's numbers (391, 556, about 437). Where the height goes at 1000x700 with More
open (control tops):

| Band | y | Note |
|------|---|------|
| Connection + Documents / Indexes / Aggregate | 8-40 | one row, fine |
| Filter + Find ... More options | 53-85 | one row |
| Sort 114, Projection 129, Skip 137, Limit 140 | 114-172 | **staircase**: each field 15 / 7.5 / 3.7 pt lower than the last |
| Query options toggle | 199-231 | **wrapped to its own row**: Sort/Projection are held at their 200 pt minimum, so the 134 pt toggle does not fit |
| Check index use / Warn before ... | 259-277 | |
| (empty, 44 pt) | 277-321 | **the verdict's reserved row wraps to its own line** even when it would fit |
| Toolbar row 1: Tree / Table / JSON, Insert document, Update matching, Delete matching | 321-353 | |
| Toolbar row 2: Select all documents of the query, Clear selection, Copy | 359-391 | |
| Table column header | 414-430 | |
| First row | 438 | |

At 800x500 the Filter row is two rows (the buttons need about 475 pt, the Filter at least 220, the
content is 504 wide), the More area takes three staggered rows (Sort 166 ... Limit 277), the two
switches are two rows plus the verdict's row, and the toolbar is three rows.

Two of these are bugs, not density:

1. **More row staircase.** `show_more_fields` (`views/findbar_more.rs:59`) lays four
   `sized_field`s into `horizontal_wrapped` without the fixed row height that `query_row` sets, so each
   field is centred on a row that grew with the previous one. `json_width` also reserves 170 pt for a
   toggle that is 134 pt wide (`TOGGLE_ROW_SHARE`), so at 1000 pt the toggle always wraps.
2. **Verdict never shares the switches' row.** `show_verdict_row` (`views/verdict_badge.rs:35`) takes
   `ui.available_width()`, which in a wrapping layout is the whole row's width, not what is left of it,
   so the reserved 320+ pt area always starts a new line. #150's comment says it should only wrap "when
   less than its minimum width is left".

Other pages (same harness): the Indexes list heading is at y=118 at 1000x700 and y=134 at 800x500
(budget 120 at 1000x700; fine). Shell, Compare, Connections and the edit dialog have their own layout
tests from #150 and are not re-measured here; the Aggregate tab (new since #150) has
`tests/aggregate_layout.rs`.

**Switcher.** At the default sidebar width (280) the three segments are 76.3 pt each (8.0-236.9), and
"Compare" in the segment font (Roboto Medium, button size) is 56.3 pt + 2 x 10 padding = 76.3 pt. The
segments are at the widest label's natural width, not shrunk, so **"Compare" is not elided**; about 35
pt of the sidebar is spare. It elides only if the person drags the sidebar below about 245 pt (then
the segment carries a tooltip with the full label). No test guards this today.

## Prototype numbers (scratch copy, not committed)

Applying the PR A changes below to a scratch copy and re-running the probe:

| Change set | 1000x700 closed | 1000x700 More | 1000x700 More + options | 800x500 closed | 800x500 More | 800x500 More + options |
|---|---|---|---|---|---|---|
| main today | 277 | 438 | 715 | 391 | 556 | 739 |
| More row aligned, one row at 1000 | 277 | 329 | 606 | 391 | 495 | 702 |
| + short switch labels, "Select all", verdict shares the row | **253** | **305** | **582** | **329** | **433** | 640 |
| + `CONTROL_HEIGHT` 28 (PR B) | 233 | 281 | 558 | 305 | 401 | 608 |
| + `ROW_GAP` 4 (PR B) | **221** | **267** | **536** | **289** | **381** | 586 |

A table row is about 18 pt plus the row gap, so at 800x500 with More open PR A leaves about 3 rows
visible and PR A + B about 5. The Indexes list heading moves from 118 / 134 to 102 / 114 with PR B.

## Goals and budgets

Budgets are the y of the first document row's top (the accesskit rect of "Select document 1") in the
`browse_layout.rs` harness. Each is the prototype's number rounded up with a few points of slack.

PR A (Browse header, no global density change):

| Window   | More closed | More open | More + Query options open |
|----------|-------------|-----------|---------------------------|
| 1000x700 | <= 260      | <= 312    | <= 590 (on screen, >= 4 rows) |
| 800x500  | <= 336      | <= 440 (on screen, >= 2 rows) | non-goal (see below) |

PR B (global density) tightens them to:

| Window   | More closed | More open | More + Query options open |
|----------|-------------|-----------|---------------------------|
| 1000x700 | <= 228      | <= 275    | <= 545 |
| 800x500  | <= 296      | <= 390 (>= 4 rows) | non-goal |

(If the focus-ring check in PR B keeps `ROW_GAP` at 6, the PR B budgets are 1000x700 240 / 288 / 565
and 800x500 312 / 408, from the 28 pt row of the table above.)

Also:

- At 1000x700 with More open, Sort, Projection, Skip, Limit and the Query options toggle are one row
  whose control tops differ by at most 1 pt, including when the toggle reads "Query options (N)". At
  800x500 they are two rows, each aligned, and nothing extends past the panel's right edge.
- At 1000x700 the two switches and the verdict area are one row; at 800x500 the verdict may wrap.
- The documents toolbar is at most two rows at 1000x700 and at 800x500.
- "Compare" is laid out unelided at the default sidebar width (a new test, no code change).
- No header row moves when a verdict appears or disappears (the #150 invariant stays).
- Indexes list heading <= 120 at 1000x700 (unchanged) and, after PR B, <= 106 at 1000x700 and
  <= 118 at 800x500; every other page's existing layout tests stay green, with their constants
  re-derived in PR B.

## Non-goals

- Making the first row visible at 800x500 with the Query options section open. The section alone is
  175 pt there (35% of 500) plus its frame; even with every saving the row lands at about 586. The
  section scrolls inside and stays usable; closing it brings the data back. See owner question 5.
- Moving the two global switches out of the Browse page (for example into Settings).
- A one-row documents toolbar at 1000x700: the seven controls need about 1100 pt with their current
  labels; two rows is the target.
- Changing the sidebar width or the segment padding again: the switcher already fits.
- Other locales: only `en.json` exists; the fits are measured in English (see risks).

## Behaviour and UI

### PR A: Browse header

1. **More row, aligned and one row when it fits** (`views/findbar_more.rs`, `views/findbar_options.rs`).
   Each field in the More row sits in a cell of the Filter row's height (control height plus the
   floating label's overhang above and below, the same computation as `query_row`), allocated with its
   width up front so `horizontal_wrapped` can wrap it. The Query options toggle's width is measured the
   way `query_row` measures its trailing buttons (egui temp memory + `request_discard` on change)
   instead of the 170 pt guess. Sort and Projection share what Skip, Limit and the toggle leave, at
   least 160 pt each **(my call: was 200)**; when that does not fit, the row wraps after Projection and
   Sort and Projection each take half the row instead of sitting at their minimum. Field order, labels,
   Enter-to-find and the toggle's accesskit expanded state do not change.
2. **Switches and verdict on one row at 1000.** `show_verdict_row` takes what is left of the current
   row (`available_size_before_wrap`), still at least its 320 pt minimum, so it wraps only when it does
   not fit. The two checkbox labels get shorter (**my call**, owner question 1): "Check index use"
   (`query.verdict.automatic`) and "Warn before full scans" (`query.guard.setting`). The full sentences
   move to hover text and accessible description: `query.verdict.automatic_hint` (new, "Check index use
   automatically: explain each query and show whether an index serves it") and the existing
   `query.guard.setting_hint`, now also set as the description.
3. **Toolbar at most two rows at 800.** "Select all documents of the query" becomes "Select all"
   (`doc.select_all_result`), with the full meaning as hover text and accessible description in a new
   `doc.select_all_result_hint` ("Select every document the query matches, on all pages, not only this
   one") **(my call**, owner question 2). "Insert document", "Update matching..." and "Delete
   matching..." keep their labels.
4. **Switcher guard.** A unit test asserts the Compare segment is at least the label's natural width
   plus padding at `SIDEBAR_DEFAULT_WIDTH`. No code change.
5. **Budgets as tests.** The new budgets replace `FIRST_ROW_BUDGET = 300` and the
   `HEADER_ONLY_BASELINE = 557` measurement in `find_options_layout.rs`.

### PR B: global density and the other pages

- `CONTROL_HEIGHT` 32 -> 28 (buttons, tabs, segmented, single-line outlined fields), `ROW_GAP` 6 -> 4;
  `COMPACT_CONTROL_HEIGHT` stays 24; horizontal spacing stays on the 8 pt grid; `button_padding.y`
  stays 4. This is Material's density -3 for a desktop app.
- Re-derive the page constants that assume 32 / 6: Shell `CHROME_GUESS` / `SPACING_SLACK`
  (`views/shell.rs`), the connection edit dialog's `CHROME_HEIGHT` (`views/connections_edit_dialog.rs`),
  the verdict row height (it uses `CONTROL_HEIGHT`), `tests/sidebar_refresh_button.rs` (asserts 32.0),
  `tests/indexes_layout.rs`, `tests/shell_layout.rs`, `tests/aggregate_layout.rs`, control/tabs widget
  tests.
- Review each page at 800x500 and 1000x700: Indexes, Queries, Compare (setup, results, apply), Shell,
  Connections manager, connection edit dialog (footer on screen with a long test error), Aggregate tab,
  bulk / copy / editor / code-export dialogs. Fix only what the review finds clipped or overlapping.

**Touch-target tradeoff.** 28 pt is above WCAG 2.2's 24x24 minimum target (SC 2.5.8, AA) and above
macOS's regular push button (about 22 pt), but below the 44 pt Apple and 48 dp Material touch guidance.
curlybrace is a desktop app driven by mouse and keyboard, so the data-density gain wins; do not go below
28 for primary controls, and keep checkboxes and the compact 24 pt segmented control where they are.
The focus ring sits 2 pt outside a control and is 2 pt wide, so with a 4 pt row gap a focused control's
ring reaches the next row's edge. PR B checks this (test) and keeps `ROW_GAP` at 6 if it overlaps.

## What stays accessible and keyboard-reachable

- Every control keeps its role and accessible name from i18n; the renamed checkboxes and "Select all"
  get descriptions with the full sentence, so a screen reader hears the meaning that the shorter label
  drops. Tests look up by i18n key, so they follow the rename; the literal-label constants in
  `tests/accessibility_findbar.rs` are updated.
- Tab order is unchanged: Filter, Find, Clear, Copy as code, Previous, Next, More options, then (open)
  Sort, Projection, Skip, Limit, Query options, the section's controls, then the switches, Why, the
  view switcher and the toolbar. A test walks it.
- More and Query options keep their expanded state; a closed More with values set still counts them;
  nothing that holds a value is hidden without a count.
- The disabled-reason line under the toolbar stays a visible line.
- Nothing becomes hover-only: shortened labels add hover text and a description, they do not move
  information into a tooltip alone.

## i18n

Changed values in `crates/ui/locales/en.json` (keys unchanged, so call sites do not change):
`query.verdict.automatic`, `query.guard.setting`, `doc.select_all_result`. New keys (nested, kept in
alphabetical order): `doc.select_all_result_hint`, `query.verdict.automatic_hint`. No new string is
built in code. The site (`site/index.html:327`) and `docs/copy.md` quote the old labels and are updated
in the same PR.

## Risks

- **English-only fit.** The one-row fits at 1000 hold for the English labels with tens of points to
  spare at most (the More row has about 25 pt). A longer locale wraps sooner; nothing overlaps, it only
  costs a row. The layout tests run in English.
- **Extra measuring frame.** The More row adds a second `request_discard` beside `query_row`'s. egui
  caps discards per frame; both settle after one pass, and a test checks an idle frame changes nothing.
- **Verdict wrap change.** Letting the verdict share the row must not reintroduce a jump when it
  appears; its fixed minimum width keeps "wraps or not" a function of the window only. Test kept.
- **Label changes** affect the site, `docs/copy.md`, the literal constants in
  `tests/accessibility_findbar.rs`, and the ignored local and container flows (they use i18n keys;
  run `scripts/container-tests.sh`, which only uses containers it creates).
- **Snapshot churn.** PR A changes every PNG that shows the Browse documents page or a dialog over it
  (about 40); PR B changes nearly all of the 126. They are rendered on macOS; CI's `ui-snapshots` job
  (macos-15, `continue-on-error`) is the check.
- **Density.** Single-line outlined fields at 28 pt leave about 5 pt above and below the body text;
  the floating label's overhang math (`outlined_field.rs:75-90`) is unchanged but must be re-checked by
  test. Focus rings with a 4 pt row gap (above).
- **Query options at small heights** stay below the fold at 800x500 (non-goal).

## Owner questions (defaults taken)

1. Shorter switch labels "Check index use" / "Warn before full scans", full sentences as hover text
   and description? **Default: yes (my call).** Alternative: keep the sentences and accept that the
   verdict wraps to its own row at 1000 (24 pt).
2. "Select all" instead of "Select all documents of the query", with the full meaning as hover text and
   description? **Default: yes (my call).** Without it the toolbar is three rows at 800x500 (38 pt).
3. Global density 28 pt controls and a 4 pt row gap (PR B)? **Default: yes for 28; 4 pt only if the
   focus-ring test passes, else 6 (my call).**
4. Sort / Projection minimum width 160 pt (was 200)? **Default: yes (my call)**; the fields scroll
   their text, and they grow to half a row when the row wraps.
5. Query options open at 800x500: accept the data below the fold, or cap the whole query block (Filter,
   More and section) to a share of the window and scroll it as one? **Default: accept (non-goal)**;
   revisit if the owner works in small windows.
6. Sidebar narrower than about 245 pt elides "Compare" (with a tooltip). Set a sidebar minimum width?
   **Default: no (my call).**
7. Two PRs (A: Browse header, B: density + other pages)? **Default: yes.**
