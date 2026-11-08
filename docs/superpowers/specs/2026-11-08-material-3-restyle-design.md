# Material 3 restyle — Design

Date: 2026-11-08
Status: Draft, awaiting user review
Issue: #131 (`feat(ui): restyle the whole app with a Material 3 look (light and dark, green accent)`)

## Intent

Curlybrace works, but it looks plain: egui's default widgets on a flat grey. The author likes the look of
Google's Material 3 (https://m3.material.io/) — its colors, rounded shapes, spacing and typography — and
wants the whole app to feel like a polished desktop app in that style.

egui has no Material widget kit. This project builds a Material 3 *look* on egui: one theme module that
holds Material's design tokens (color roles, type scale, shape, spacing, state layers), a startup step that
maps them onto egui's `Style`/`Visuals`, and a small set of custom widgets for the pieces egui lacks
(segmented control, chip, outlined field with floating label, tonal pill, card). It is not a port of the
Material component library.

## Goals

- The whole app, restyled in one go: connection manager and its dialogs, Browse (sidebar tree, query bar,
  verdict badge, tree/table/JSON views, editor and delete modals), Compare, Copy, Indexes (usage chart,
  Queries view, builder, drop confirm), banners, and every other modal.
- Light and dark modes, derived from one **green** seed, so the accent is the same in both. (The author
  looked at a light and a dark treatment in mockups, with green and with blue, and chose green for both.)
- Material 3's default spacing and shapes. Compactness for data-heavy screens (document table, tree, index
  lists) is a later change, not part of this project. Spacing is therefore a token, so that later change is
  one edit.
- Every color, size, radius and font comes from the theme module. No literals in views.
- Both modes are visible in the screenshot tests.

## Non-goals

- A port of Material's component library, or pixel parity with the web components.
- Ripple effects, shared-element transitions and Material's motion system. egui's built-in hover and press
  feedback is used as is.
- Changing what each screen does or how it is laid out. This restyles; it does not move to a Material
  navigation rail or drawer, add features or rearrange panels.
- Density options. Material defaults now; compactness later.
- The native window frame and title bar (macOS keeps its own).
- Windows and Linux (the app is macOS only).

## Anti-goals

- Losing accessibility. Every existing `accessibility_*` test stays green; custom widgets must expose the
  same role and label to the accessibility tree that the widget they replace did. Text and key controls meet
  WCAG AA contrast (4.5:1 normal text, 3:1 large text and component boundaries) in both modes.
- A theme that is only a skin of constants scattered through the views. If a view needs a new color, it
  asks the theme for a role; it never invents one.
- Slower frames. egui repaints every frame; custom painting must stay cheap (no per-frame allocation of fonts
  or large shapes).
- Hardcoded user-facing strings. Any new label is an i18n key in `en.json`, and new keys are reported.
- Colors that change meaning between modes. A verdict that is "bad" is recognisable as bad in both.

## Constraints

- Rust workspace: `curlybrace-core`, `curlybrace-ui` (egui 0.32.3, `egui_extras`, `egui_plot` 0.33),
  `curlybrace-app` (eframe 0.32.3), `curlybrace-testkit`. macOS only.
- Today there is **no theme code**: no `Visuals`, `Style` or font setup anywhere; egui's defaults and system
  dark/light are used. Hand-picked colors are few: connection tag colors (chosen by the user, stay as they
  are), the JSON syntax colors in `views/json.rs`, the usage chart fills in `views/index_chart.rs` and
  `views/queries_chart.rs`, and the verdict tones in `views/verdict_badge.rs`. The restyle therefore has one
  natural seam: install the theme once at startup in `curlybrace-app`/`CurlybraceApp::new`, and move those
  few call sites onto roles.
- UI is a pure reducer plus view-models plus egui views (`crates/ui/src/{app,viewmodel,views}`); colors
  never enter the view-models.
- Tests: `egui_kittest` accessibility tests and 58 screenshot tests (57 baselines plus a gated one) that
  run on macOS with Metal (`CURLYBRACE_SNAPSHOTS=1`; CI job `ui-snapshots`, non-required but currently
  green). The screenshot harness must install the same theme as the app, or the baselines would not show
  what users see.
- Project rules: immutable data, files under 800 lines (200–400 typical), TDD, at least 80% coverage, i18n
  keys for strings, no hardcoded user-facing text.

## Decisions made with the author

- **Material 3**, whole app, in one go.
- **Light and dark**, **one accent: green**, in both.
- **Look** (from the reviewed mockups): segmented Browse/Compare switch; selected sidebar rows as tonal
  pills; outlined text fields with a floating label; filled primary buttons and tonal secondary buttons,
  fully rounded; tables inside rounded cards with a tonal header row and subtle row hover; the index
  verdict as a tonal chip; dark surfaces use tonal elevation (higher means lighter) rather than heavy shadows.
- **Material 3 default density.**

## Calls made by the design author (not discussed with the author; flagged for review)

- **Seed and tokens are static.** Pick one green seed (starting point: near MongoDB's green, `#13AA52`;
  the mockups used a slightly darker `#2C6B30` primary for light mode) and generate both modes' tonal
  palettes with Material's Theme Builder, then check the resulting token values into the theme module as
  constants. No runtime color science and no new color dependency. Changing the accent later means
  re-generating the tables.
- **Mode follows the system** (egui's system theme preference; the builder verifies that eframe on macOS
  picks up appearance changes while the app runs). The author did not discuss how the mode is chosen.
  Whether to add a manual Light / Dark / System switch is an open question for the author (see the end).
- **The mockups are the reference for the look.** They are local and git-ignored
  (`.superpowers/brainstorm/*/content/theme-direction.html`), so the values are copied here. Light (green):
  primary `#2C6B30`, on-primary `#FFFFFF`, primary container `#B0F2AC`, secondary container `#D4E8CF`,
  background `#FBFDF6`, surface-low `#F0F5EA`, surface-high `#E4E9DE`, outline `#72796F`, outline-variant
  `#C2C9BD`. Dark (green): primary `#8BD88A`, on-primary `#00390F`, primary container `#1E5127`, secondary
  container `#3A4B39`, background `#10140F`, surface-low `#181D17`, surface-high `#262B24`, outline
  `#8B9388`, outline-variant `#414940`. These were hand-picked approximations, not generated output.
- **Seed.** Choose the seed so that Theme Builder's generated primaries land close to the mockup values
  above (light `#2C6B30`, dark `#8BD88A`); the builder tunes the seed by eye and records it. Generation is a
  manual, one-off step in the web tool; the resulting tables are pasted into the theme module and the
  seed and tool version are noted in a comment.
- **The drift test** forbids any `Color32` constructor that takes channel values (`from_rgb`, `from_gray`,
  `from_rgba_*`, hex) and any literal font size (`RichText::size`, `FontId::new`) outside the theme module.
  `Color32::TRANSPARENT` is allowed. The user-chosen connection tag color in `views/connection_badge.rs` is
  the one allow-listed exception; the allow-list names files, not patterns.
- **Fonts keep egui's default stack as a fallback** behind Roboto, so CJK, emoji and symbols in MongoDB data
  still render. Budget: the bundled fonts add at most about 1.5 MB to the app binary; above that the
  builder drops a weight or the icon subset and reports. The glyph subset is produced by a documented
  one-off script (for example `pyftsubset`) and the subset font is checked in; the list of glyphs is the set
  the views actually use.
- **Verdict and chart colors.** The seven verdict kinds map onto four tones (good, warning, bad, neutral),
  each with a container and an on-container role. Chart series colors must reach 3:1 contrast against the
  chart surface and must also differ by something other than hue (the legend, labels or dash style the
  charts already carry); the builder adds extra roles only as the charts need them.
- **Table cards and tonal rows** are drawn around and behind the existing `egui_extras` tables (a rounded
  frame, a painted header row and hover fill) without changing the table's layout. If a table cannot be
  styled that way without relayout, the builder keeps the layout and restyles only colors and the frame.
- **Screenshots** are regenerated locally on macOS with CI's toolchain (1.87.0) and committed with the
  change; the same baselines passed on the CI runner after the last regeneration (#130). The author
  reviews the changed images in the PR.
- **Fonts.** Bundle Roboto (Apache 2.0) Regular and Medium for text, keep a monospace face for JSON and
  query text, and a subset of Material Symbols (Outlined) limited to the icons actually used. Binary-size
  impact is checked and reported; if it is large, fall back to fewer weights or system fonts.
- **Semantic colors beyond Material's roles.** Material 3 has no success/warning roles. Add *success*,
  *warning* and their containers as extra roles, harmonised to the seed, for the verdict chip, status rows
  and the usage chart. Error uses Material's own error roles.
- **JSON syntax colors and chart fills move onto roles** (or onto extra roles derived from them), so they
  adapt to the mode. Connection tag colors are user data and are not touched.
- **State layers by overlay.** Hover, pressed and focus are Material's translucent overlay of the "on"
  color, applied through egui's widget visuals (`hovered`, `active`, `open`) and a shared helper for custom
  widgets.
- **Theme gallery test.** A screenshot of every custom widget in every state (enabled, hover, pressed,
  focused, disabled, error) in both modes, in addition to the full-screen baselines, so a token change is
  reviewed in one place.
- **One branch, one merge.** The work is planned as foundation first (tokens, startup, fonts, widgets,
  gallery), then screens in groups, but it ships as one feature (one PR) so the app never exists half in one
  style and half in another on `main`.
- **Guard against drift.** A test fails if a view or the app crate contains a `Color32` literal or a
  hand-sized text style outside the theme module (an allow-list for user-chosen tag colors).

## Testing

- Unit tests for the theme: every role pair used for text or component boundaries meets the AA contrast
  ratios in both modes (computed in the test from the token values).
- Accessibility tests (`egui_kittest`): existing ones stay green; add checks that custom widgets (segmented
  control, chip, outlined field) expose role, label and value to the accessibility tree.
- Screenshot baselines: all regenerated once, in both modes where the screen carries the new styling
  (browse, indexes, compare, dialogs) and the gallery. The PR shows the images for review.
- The drift test above.

## Left to the builder

- Module layout of the theme (tokens, scheme, startup, widgets) and token names.
- The exact seed and generated tables; exact radii and spacing within Material's defaults.
- How custom widgets are built (egui `Widget` vs painter helpers) and how focus rings are drawn.
- Which Material Symbols glyphs are needed and how the subset font is produced and checked in.
- Order of screen groups in the plan, and how the screenshot set is split between light and dark.
- Whether egui's default text sizes map onto Material's type scale directly or need per-style overrides.

## Open question for the author

- **Manual mode switch.** With the app following the system, there is no control to force light or dark.
  A Light / Dark / System choice would add one control (in the settings dialog the tool paths already use),
  its i18n keys, and one stored setting. It is left out of the first version unless the author wants it.
