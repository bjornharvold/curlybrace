# Material 3 restyle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the whole app with a Material 3 look, light and dark (following the system), one green accent, Material default density.

**Architecture:** One theme module in `curlybrace-ui` holds Material's tokens (a color `Scheme` per mode, type scale, shapes, spacing, state layers) and installs them into egui's `Style`/`Visuals` once at startup. A few custom widgets cover what egui lacks. Views then call the theme and the widgets; no view owns a color or a size. Ships as one PR.

**Tech Stack:** Rust, egui/eframe 0.32.3, `egui_extras`, `egui_plot` 0.33, `egui_kittest` (accessibility and screenshot tests), Roboto and Material Symbols (bundled fonts).

**Spec:** `docs/superpowers/specs/2026-11-08-material-3-restyle-design.md` (issue #131)

## Global Constraints

- Theme code lives in `crates/ui/src/theme/`; the app installs it from `crates/app/src/main.rs` (`cc.egui_ctx`), and the screenshot harness (`crates/ui/tests/support/shots.rs`) installs the same theme.
- Mode is controlled by the system only: egui's system theme preference. No manual switch, no stored mode setting.
- Color tokens come from the mockup values in the spec ("The mockups are the reference for the look"). Light: primary `#2C6B30`, on-primary `#FFFFFF`, primary container `#B0F2AC`, secondary container `#D4E8CF`, background `#FBFDF6`, surface-low `#F0F5EA`, surface-high `#E4E9DE`, outline `#72796F`, outline-variant `#C2C9BD`. Dark: primary `#8BD88A`, on-primary `#00390F`, primary container `#1E5127`, secondary container `#3A4B39`, background `#10140F`, surface-low `#181D17`, surface-high `#262B24`, outline `#8B9388`, outline-variant `#414940`. Roles the mockups did not need (on-surface, on-variant, surface container levels, error, success, warning and their containers) follow Material 3's tone mapping for a green seed; the contrast tests arbitrate. No web tool is available to the implementer, so the tables are written by hand from these values; the seed and "regenerate with Theme Builder later" are noted in a comment.
- Material 3 default spacing and shapes (shape scale: 4, 8, 12, 16, 28 dp; fully rounded for buttons and chips). No density options.
- AA contrast in both modes: 4.5:1 for text, 3:1 for large text and component boundaries (tested from token values).
- Outside `theme/`, no `Color32` constructor that takes channel values and no literal font size. Allow-listed: `views/connection_badge.rs` (user-chosen tag color). `Color32::TRANSPARENT` is allowed.
- No hardcoded user-facing strings: new labels are i18n keys in `crates/ui/locales/en.json`; report new keys.
- Files under 800 lines (200–400 typical); immutable data; TDD; at least 80% coverage for new code.
- Clippy and fmt exactly as CI: `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings` and `cargo fmt --check` (CI's toolchain is 1.87.0; the local default is newer and flags unrelated code).
- Screenshots: `CURLYBRACE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots --locked -- --include-ignored --nocapture`; add `UPDATE_SNAPSHOTS=1` to rewrite. Each task regenerates only the baselines it changes and the reviewer looks at the changed PNGs.
- Commit format `<type>(<scope>): <description>`, subject lowercase, trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Never commit `.superpowers/`.
- Branch: `feat/131-material-3-restyle` from `main`; PR targets `main` (the repo's integration branch).

## Review Focus

Failure modes the spec implies that no task's tests would otherwise exercise; each has a pinning test in the owning task.

1. **The system appearance flips while the app runs.** Both modes must render fully and the theme must re-apply (Task 2: `switching_the_theme_changes_the_panel_color`; gallery in both modes, Task 4).
2. **Long or translated labels.** Chips, the segmented control and buttons must not clip, overlap or grow the row (Task 4: `segmented_with_long_labels_stays_inside_its_width`, `chip_with_long_text_truncates`).
3. **Glyphs that stop rendering.** Every non-ASCII character the UI draws (the gear, check mark, disclosure arrows, `□`) must still resolve in the new font stack, or it shows as a box (Task 3: `every_glyph_the_ui_draws_resolves`).
4. **Keyboard focus is invisible on custom widgets.** Tab must reach every custom widget and draw a visible focus ring (Task 4: `custom_widgets_are_reachable_by_tab_and_show_focus`).
5. **Retina rendering.** Roboto at 2x pixels per point must lay out without clipped descenders or wrapped one-line labels (Task 2: `text_styles_lay_out_at_two_pixels_per_point`).

---

### Task 1: Tokens, schemes and contrast

**Files:**
- Create: `crates/ui/src/theme/mod.rs`, `crates/ui/src/theme/scheme.rs`, `crates/ui/src/theme/shape.rs`, `crates/ui/src/theme/contrast.rs`, `crates/ui/src/theme/scheme_tests.rs`
- Modify: `crates/ui/src/lib.rs` (add `pub mod theme;`)

**Interfaces:**
- Produces:
  - `pub enum Mode { Light, Dark }` and `pub fn scheme(mode: Mode) -> &'static Scheme`.
  - `pub struct Scheme` with `egui::Color32` fields: `primary, on_primary, primary_container, on_primary_container, secondary_container, on_secondary_container, background, surface, surface_container_low, surface_container, surface_container_high, on_surface, on_surface_variant, outline, outline_variant, error, on_error, error_container, on_error_container, success, on_success, success_container, on_success_container, warning, on_warning, warning_container, on_warning_container, shadow`.
  - `pub enum Tone { Good, Warning, Bad, Neutral }` and `Scheme::tone(&self, Tone) -> ToneColors { fg, container, on_container }`.
  - `shape`: `pub const SMALL: f32 = 8.0; MEDIUM = 12.0; LARGE = 16.0; EXTRA_LARGE = 28.0; FULL = 999.0`; spacing constants `SPACE_1 = 4.0 … SPACE_6 = 24.0` (Material 4 dp grid).
  - `contrast::ratio(a: Color32, b: Color32) -> f32` (WCAG relative luminance).
  - `Scheme::mode(&self) -> Mode`.

- [ ] **Step 1: Write the failing tests** in `scheme_tests.rs`:
  `light_and_dark_use_the_mockup_primaries` (light primary `#2C6B30`, dark `#8BD88A`), `ratio_of_black_on_white_is_21`, and `every_text_pair_meets_aa_in_both_modes`: for both modes assert ≥ 4.5 for (on_surface, background/surface/containers), (on_surface_variant, surface and containers), (on_primary, primary), (on_primary_container, primary_container), (on_secondary_container, secondary_container), (on_error, error), (on_error_container, error_container), and each tone's (on_container, container); assert ≥ 3.0 for (outline, background) and (primary, background).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui theme::` — expect FAIL (module missing).
- [ ] **Step 3: Implement** the files above. The tables are constants; add a comment with the seed note from Global Constraints. Tune any role that fails the AA test (change the role, not the test).
- [ ] **Step 4: Run** the same command — PASS; then clippy and fmt as in Global Constraints.
- [ ] **Step 5: Commit** `feat(theme): add the Material 3 color schemes, shapes and contrast check`.

### Task 2: Install the theme into egui

**Files:**
- Create: `crates/ui/src/theme/install.rs`, `crates/ui/src/theme/typography.rs`, `crates/ui/src/theme/install_tests.rs`
- Modify: `crates/ui/src/theme/mod.rs`, `crates/app/src/main.rs`, `crates/ui/tests/support/shots.rs`

**Interfaces:**
- Consumes: Task 1 `Scheme`, `Mode`, `shape`.
- Produces:
  - `pub fn install(ctx: &egui::Context)` — sets egui's system theme preference, then `ctx.set_visuals_of(egui::Theme::Light/Dark, visuals_for(scheme))` and `ctx.set_style_of(...)` with text styles and spacing.
  - `pub fn visuals_for(scheme: &Scheme) -> egui::Visuals` (panel/window/faint/extreme fills from the surface levels; widget states with state-layer fills; selection from `secondary_container`/`primary`; rounding from `shape`; `window_shadow` and `popup_shadow` kept light).
  - `pub fn state_layer(base: Color32, on: Color32, state: StateLayer) -> Color32` with `enum StateLayer { Hover, Focus, Pressed }` (opacity 8%, 10%, 10%).
  - `pub fn current(ctx: &egui::Context) -> &'static Scheme` (from `ctx.theme()`), and `pub fn current_ui(ui: &egui::Ui) -> &'static Scheme`.
  - `typography`: `pub fn text_styles() -> BTreeMap<TextStyle, FontId>` mapping Material's scale (body 14, label 12, title 16/22, headline 24; monospace 13) onto egui's styles.
  - Shot harness: `Shot::mode(self, Mode) -> Self` (default `Mode::Light`) that calls `theme::install` and `harness.ctx.set_theme(...)` before the first frame.

- [ ] **Step 1: Write the failing tests** in `install_tests.rs`: `visuals_use_the_scheme_surfaces` (panel_fill == surface_container_low of the mode's scheme; selection fill == secondary_container), `state_layer_blends_toward_the_on_color` (hover at 8% of on over base), `switching_the_theme_changes_the_panel_color` (run a kittest `Harness` frame in Light, `ctx.set_theme(Dark)`, run another, assert `ctx.style().visuals.panel_fill` changed), `text_styles_lay_out_at_two_pixels_per_point` (Harness at `with_pixels_per_point(2.0)`; a one-line label at each text style keeps a single row).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui theme::install` — FAIL.
- [ ] **Step 3: Implement** `install`, `visuals_for`, `text_styles`, `state_layer`, `current`; call `curlybrace_ui::theme::install(&cc.egui_ctx)` in `main.rs` before building the app; make `Shot` install the theme and honor `.mode(...)`. Fonts stay egui's defaults here (Task 3).
- [ ] **Step 4: Run** the new tests, then `cargo test --workspace`, clippy, fmt — PASS.
- [ ] **Step 5: Regenerate every baseline** (`UPDATE_SNAPSHOTS=1`, whole suite): the whole app now has Material colors on egui's default widgets. Look at `browser_table_view.png` and `connection_manager_list.png`.
- [ ] **Step 6: Commit** `feat(theme): install the Material 3 theme at startup and in the screenshot harness`.

### Task 3: Fonts and icons

**Files:**
- Create: `crates/ui/assets/fonts/Roboto-Regular.ttf`, `Roboto-Medium.ttf`, `MaterialSymbolsSubset.ttf`, `crates/ui/assets/fonts/LICENSE-Roboto.txt`, `LICENSE-MaterialSymbols.txt`, `scripts/subset-icons.sh`, `crates/ui/src/theme/fonts.rs`, `crates/ui/src/theme/icons.rs`, `crates/ui/src/theme/fonts_tests.rs`
- Modify: `crates/ui/src/theme/install.rs`, `crates/ui/src/theme/mod.rs`

**Interfaces:**
- Consumes: Task 2 `install`.
- Produces:
  - `pub fn font_definitions() -> egui::FontDefinitions`: Roboto Regular as `Proportional` first, Roboto Medium as a named family `"roboto-medium"` used for `RichText::strong`/titles, Material Symbols as family `"symbols"`, egui's default fonts kept after Roboto as fallback (emoji and symbols in data still render).
  - `icons`: `pub const GEAR: char`, `CHECK`, `CHEVRON_RIGHT`, `CHEVRON_DOWN`, `CLOSE`, `WARNING`, `ERROR`, `INFO` … one `char` per glyph the views use, and `pub fn icon(c: char, size: f32) -> egui::RichText` in the `"symbols"` family.

- [ ] **Step 1: Write the failing tests** in `fonts_tests.rs`: `fonts_stay_within_the_size_budget` (sum of the three bundled files plus nothing else ≤ 1_500_000 bytes), `every_glyph_the_ui_draws_resolves` (collect every non-ASCII character from `locales/en.json` and the `icons` constants; for each, `fonts.has_glyph` in the family it is drawn in — Review Focus 3), `roboto_text_style_uses_roboto`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** Download Roboto (Apache 2.0) and Material Symbols Outlined (Apache 2.0) into `assets/fonts/`; write `scripts/subset-icons.sh` (documented `pyftsubset` call and the glyph list, so the subset is reproducible); `include_bytes!` them in `fonts.rs`; call `ctx.set_fonts(font_definitions())` in `install`. If the budget is exceeded, drop a weight or the icon subset and report it.
- [ ] **Step 4: Run** tests, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate all baselines** (text metrics change) and look at one.
- [ ] **Step 6: Commit** `feat(theme): bundle Roboto and a Material Symbols subset`.

### Task 4: Custom widgets and the theme gallery

**Files:**
- Create: `crates/ui/src/theme/widgets/{mod,segmented,chip,outlined_field,pill,card,button}.rs`, `crates/ui/src/theme/widgets/widgets_tests.rs`, `crates/ui/tests/accessibility_theme_widgets.rs`, `crates/ui/tests/theme_gallery.rs` (gated like `snapshots.rs`)
- Modify: `crates/ui/src/theme/mod.rs`

**Interfaces:**
- Consumes: Tasks 1–3 (`current_ui`, `state_layer`, `shape`, `icons`).
- Produces (all draw from the current scheme; all call `Response::widget_info` so the accessibility tree has the role and label the replaced egui widget had):
  - `segmented<T: PartialEq + Copy>(ui, items: &[(T, String)], selected: T) -> Option<T>` (a row of mutually exclusive buttons; role `RadioButton`, one per item, `selected` state reported).
  - `chip(ui, tone: Tone, text: &str) -> egui::Response` (a tonal, fully rounded, non-interactive label; truncates with an ellipsis).
  - `outlined_field(ui, label: &str, text: &mut String, hint: &str) -> egui::Response` (outlined `TextEdit` with the label floating on the border; the label is the accessible name).
  - `pill(ui, selected: bool, text: impl Into<WidgetText>) -> egui::Response` (a sidebar row; selected = `secondary_container` pill; role as `selectable_label`).
  - `card<R>(ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R>` (rounded `surface_container_low` frame).
  - `filled_button`, `tonal_button`, `text_button` (`ui, text) -> Response`, fully rounded; disabled state follows Material (38% opacity on-surface).
  - Focus ring: 2 dp `primary` outline on keyboard focus for all of the above.

- [ ] **Step 1: Write the failing tests.** `widgets_tests.rs`: `segmented_reports_the_clicked_item`, `segmented_with_long_labels_stays_inside_its_width`, `chip_with_long_text_truncates`, `outlined_field_edits_the_string`, `pill_selected_uses_the_secondary_container`. `accessibility_theme_widgets.rs`: each widget is found by role and label via the kittest tree (`get_by_role_and_label`), segmented exposes `selected`, and `custom_widgets_are_reachable_by_tab_and_show_focus` (Tab through the gallery; assert the focused widget's rect draws the focus ring color).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui widgets` and the accessibility target — FAIL.
- [ ] **Step 3: Implement** the widgets. Painting uses cached `Galley`s from `ui.painter()`; no per-frame font or large allocation.
- [ ] **Step 4: Gallery.** `theme_gallery.rs` renders every widget in enabled, hover, pressed, focused, disabled and error states, in both modes, as `theme_gallery_light` and `theme_gallery_dark`. Generate the two baselines and look at both.
- [ ] **Step 5: Run** all new tests, workspace tests, clippy, fmt.
- [ ] **Step 6: Commit** `feat(theme): add the segmented control, chip, outlined field, pill, card and buttons`.

### Task 5: Semantic colors in verdicts, charts and JSON

**Files:**
- Modify: `crates/ui/src/views/verdict_badge.rs`, `views/json.rs`, `views/index_chart.rs`, `views/queries_chart.rs`, `views/metrics.rs` (if it picks colors), `crates/ui/src/viewmodel/verdict.rs` (tone mapping only)
- Modify tests: `crates/ui/src/views/` unit tests that assert colors; `crates/ui/tests/accessibility_verdict.rs`

**Interfaces:**
- Consumes: Task 1 `Tone`, `Scheme::tone`; Task 4 `chip`.
- Produces: `theme::json_colors(scheme) -> JsonColors { key, string, number, boolean, null, punctuation }` and `theme::chart_colors(scheme) -> ChartColors { series: [Color32; 4], no_data, restart, grid }` (roles-derived; every series reaches 3:1 against `surface_container_low`).

- [ ] **Step 1: Write the failing tests:** `verdict_chip_uses_the_tone_container` for each of the seven verdict kinds (maps to Good/Warning/Bad/Neutral: IndexUsed/CoveredByIndex → Good; InefficientIndex/SortNotCovered → Warning; CollectionScan → Bad; NoFilter/EmptyCollection → Neutral), `chart_series_reach_3_to_1_in_both_modes`, `json_colors_reach_aa_on_the_surface`, and a chart-legend test that series stay distinguishable by label, not only hue (the existing legend entries remain).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** `json_colors` and `chart_colors` in `theme/`, replace `tone_color`, the chart fills and the JSON colors; the verdict badge shows a `chip`.
- [ ] **Step 4: Run** the verdict/queries/indexes accessibility tests (they must stay green), workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** the baselines for browse (`browser_*`), `indexes_tab` and any Queries shots; look at them.
- [ ] **Step 6: Commit** `refactor(theme): move verdict, chart and JSON colors onto theme roles`.

### Task 6: App shell — sidebar, switchers, tabs, banner

**Files:**
- Modify: `crates/ui/src/app/panels.rs`, `views/sidebar.rs`, `views/sidebar_roots.rs`, `views/sidebar_drag.rs`, `views/sidebar_copy.rs`, `views/collection_tabs.rs`, `views/documents.rs` (mode switcher, connection header), `views/indexes.rs` (the Indexes | Queries and range switches), `views/compare/mod.rs` (workspace switcher), `views/connection_badge.rs`

**Interfaces:**
- Consumes: Task 4 `segmented`, `pill`, `text_button`, `icons`.
- Produces: no new public API; `show_workspace_switcher`, `show_mode_switcher`, `show_collection_tabs` keep their signatures.

- [ ] **Step 1: Write the failing tests.** In the existing accessibility tests (`accessibility.rs`, `sessions_shell.rs`, `accessibility_indexes.rs`) keep every assertion; add: the Browse/Compare switcher exposes two radio-button items with the selected one marked; the sidebar's selected collection is a selected pill; the gear is a button named by `tools.settings_button`.
- [ ] **Step 2: Run** those targets — the new assertions FAIL.
- [ ] **Step 3: Implement** the swaps: `selectable_label` → `segmented`/`pill`, the gear button → `icons::GEAR` text button with the same accessible name, tabs get the `primary` underline, banners use `error_container`/`warning_container`. Sidebar uses `surface_container_low`.
- [ ] **Step 4: Run** all accessibility and sessions tests, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** `sessions_tree_*`, `error_banner`, `browser_*` and add `browser_table_view_dark`; look at them.
- [ ] **Step 6: Commit** `feat(ui): restyle the sidebar, switchers and tabs`.

### Task 7: Browse — query bar, tree, table, JSON, editor and delete

**Files:**
- Modify: `views/findbar.rs`, `views/tree.rs`, `views/table.rs`, `views/json.rs`, `views/doc_select.rs`, `views/context_menu.rs`, `views/editor.rs`, `views/error_label.rs`, `views/write_warning.rs`

**Interfaces:**
- Consumes: Task 4 `outlined_field`, `card`, `filled_button`, `tonal_button`, `text_button`; Task 5 `json_colors`.
- Produces: none.

- [ ] **Step 1: Write the failing tests.** In `accessibility_findbar.rs` keep all assertions and add: Filter, Sort, Projection, Skip and Limit are found by their labels as editable text fields; the Find button is found by role and label. In a `views/table` unit test: the table header row is painted with `surface_container_high` and a hovered row with the hover state layer.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** Query bar fields become `outlined_field`; Find is filled, Clear/Previous/Next tonal or text; the table sits in a `card` with a painted header row and hover fill (layout unchanged; if a table cannot be styled this way without relayout, keep the layout and restyle colors and the frame only); editor and delete modals use the themed `Modal` frame with `filled_button`/`text_button` actions.
- [ ] **Step 4: Run** findbar/read-only/accessibility tests, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** `browser_*`, `editor_modal*`, `delete_confirmation`, `production_write_warning`; add `editor_modal_dark`. Look at them.
- [ ] **Step 6: Commit** `feat(ui): restyle the query bar, document views and editors`.

### Task 8: Connections

**Files:**
- Modify: `views/connections.rs`, `views/connections_list.rs`, `views/connections_edit.rs`, `views/connections_edit_dialog.rs`, `views/connections_edit_mode.rs`, `views/connections_edit_tools.rs`, `views/connections_from_uri.rs`, `views/connections_folder_prompt.rs`, `views/tool_row.rs`, `views/tools_settings.rs`, `app/panels.rs` (host key prompt)

**Interfaces:**
- Consumes: Task 4 widgets. Produces: none.

- [ ] **Step 1: Write the failing tests.** Keep `accessibility_connections.rs`, `accessibility_connection_tools.rs`, `accessibility_tools.rs` assertions; add: every field of the connection form is an `outlined_field` found by its label; the auth-method and mode choices are `segmented`; the Test Connection result still appears as a labelled status.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** the swaps; the connection list rows become `pill`s; dialogs use the themed modal and button set; the tool path rows keep their status text.
- [ ] **Step 4: Run** the three accessibility targets, `wiring_connections.rs`, `connections_*` tests, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** `connection_*`, `host_key_prompt*`; add `connection_manager_list_dark`; look at them.
- [ ] **Step 6: Commit** `feat(ui): restyle the connection manager and settings dialogs`.

### Task 9: Compare

**Files:**
- Modify: `crates/ui/src/views/compare/*.rs` (apply, apply_result, export, export_result, fields, overview, pending, preflight, results, mod and any other file there)

**Interfaces:**
- Consumes: Task 4 widgets, Task 5 tone/chip. Produces: none.

- [ ] **Step 1: Write the failing tests.** Keep every `accessibility_apply*.rs`, `accessibility_export.rs`, `accessibility_fields.rs`, `compare_production_review.rs` assertion; add: compare step progress and result states use `chip`s whose accessible text is unchanged; the destructive Apply on production keeps its confirm text.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** the restyle (cards for result groups, tonal headers, buttons from the set); no layout changes.
- [ ] **Step 4: Run** the compare accessibility targets, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** `compare_*`; add `compare_overview_mixed_dark`; look at several.
- [ ] **Step 6: Commit** `feat(ui): restyle Compare`.

### Task 10: Copy and Indexes

**Files:**
- Modify: `views/copy_dialog.rs`, `views/copy_progress.rs`, `views/index_builder.rs`, `views/index_detail.rs`, `views/index_dialogs.rs`, `views/index_sampling.rs`, `views/indexes.rs`, `views/queries.rs`, `views/queries_rows.rs`, `views/queries_consent.rs`

**Interfaces:**
- Consumes: Task 4 widgets, Task 5 chart colors. Produces: none.

- [ ] **Step 1: Write the failing tests.** Keep `accessibility_copy.rs`, `accessibility_index_*.rs`, `accessibility_indexes.rs`, `accessibility_queries*.rs`, `accessibility_sampling.rs` assertions; add: builder key rows use `outlined_field`; the key-type choice is `segmented`; the Queries rows show their verdict as a `chip` with unchanged accessible text; the consent dialog keeps its exact-command text.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** the restyle.
- [ ] **Step 4: Run** the listed accessibility targets, workspace tests, clippy, fmt.
- [ ] **Step 5: Regenerate** `copy_*`, `index_*`, `indexes_tab`; add `copy_dialog_overwrite_production_dark` and `indexes_tab_dark`; look at them.
- [ ] **Step 6: Commit** `feat(ui): restyle Copy and Indexes`.

### Task 11: Drift guard, cleanup and docs

**Files:**
- Create: `crates/ui/tests/theme_drift.rs`
- Modify: `CONTRIBUTING.md` (one paragraph: where the theme lives, the drift rule, the screenshot-both-modes convention), `README.md` and `site/` screenshots only if they show the old look (report, do not restyle the site)

**Interfaces:**
- Consumes: everything above.
- Produces: `theme_drift` test that reads `crates/ui/src` and `crates/app/src` (excluding `theme/` and test files) and fails on any `Color32::from_rgb|from_rgba_*|from_gray|from_hex`, any `RichText::size(` or `FontId::new(`/`FontId::proportional(` with a literal, and any `.fill(` or `.color(` call given a literal; allow-list is the file `views/connection_badge.rs`.

- [ ] **Step 1: Write the failing test** `theme_drift`, and `theme_drift_allow_list_is_one_file`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui --test theme_drift` — expect FAIL listing any leftover literal.
- [ ] **Step 3: Fix** every leftover by moving it onto a role or text style (never loosen the test).
- [ ] **Step 4: Full verification:** `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --check`, `cargo test --workspace --locked`, and the full screenshot suite (`CURLYBRACE_SNAPSHOTS=1 ... --include-ignored`) with no `UPDATE_SNAPSHOTS` — all green.
- [ ] **Step 5: Run the app** (`cargo run -p curlybrace`) in both macOS appearances and open Browse, Compare, Copy, Indexes (Usage and Queries) and a dialog; report what you saw (egui drawing can only be judged on screen).
- [ ] **Step 6: Commit** `test(theme): fail on hard-coded colors and font sizes outside the theme`; then open the PR against `main` ("Closes #131") with the changed screenshots listed.
