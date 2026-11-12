//! The Shell tab's vertical layout with the History panel open or closed: the output pane keeps
//! a usable height and the editor stays usable, at a small (800x500) and a common (1280x800)
//! window. Measured from the accesskit bounds of the editor and the output region. Fast, offline,
//! no GPU.
use curlybrace_core::history::{ShellHistoryEntry, ShellRunStatus};
use curlybrace_core::shell::{DocKind, Outcome, ShellResult, Stream};
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::ShellState;
use curlybrace_ui::viewmodel::shell_result::OutputMode;
use curlybrace_ui::views::shell::{ShellViewContext, show_shell};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const SESSION: Uuid = Uuid::from_u128(11);
const NOW: i64 = 1_700_000_000;
/// The room the app takes from the window before the tab gets it. The tab fills the central
/// panel (the workspace switcher is in the sidebar), whose margins the harness has as well; this
/// is slack on top of those.
const APP_CHROME: f32 = 8.0;
/// The output pane is never shorter than this while the window leaves room for it (points).
const OUTPUT_MIN: f32 = 120.0;
/// The editor stays usable: about three lines (points).
const EDITOR_USABLE: f32 = 60.0;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// A full history (200 runs) and a full output pane.
fn shell(open: bool) -> ShellState {
    let entries = (0..200)
        .map(|id| ShellHistoryEntry {
            id,
            ran_at_secs: NOW - id * 60,
            script: format!("db.c{id}.find()"),
            status: ShellRunStatus::Succeeded,
        })
        .collect();
    let output = (0..500).map(|n| format!("line {n}\n")).collect::<String>();
    let loaded = ShellState::default()
        .edited("db.a.find()".into(), None)
        .run_requested()
        .0
        .output_arrived(1, Stream::Stdout, &output)
        .history_loaded(entries);
    match open {
        true => loaded.history_toggled(),
        false => loaded,
    }
}

fn harness(size: egui::Vec2, shell: ShellState) -> Harness<'static, ShellState> {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size).build_ui_state(
        move |ui, shell: &mut ShellState| {
            ui.allocate_space(egui::vec2(10.0, APP_CHROME));
            let ctx = ShellViewContext {
                session: SESSION,
                connection: "local",
                badge: ConnectionBadge::default(),
                connected: true,
                now_secs: NOW,
                collections: &[],
            };
            let _ = show_shell(ui, shell, ctx, &i18n);
        },
        shell,
    );
    theme::install(&harness.ctx);
    harness.run_steps(4);
    harness
}

/// The top of a node and how much of its height is inside a window `bottom` points tall: a
/// pane laid out past the window's edge is not visible there.
fn span(h: &Harness<'static, ShellState>, role: Role, label: &str, bottom: f32) -> (f32, f32) {
    let node = h.get_by_role_and_label(role, label);
    let rect = node.accesskit_node().bounding_box().expect("bounds");
    let (top, end) = (rect.y0 as f32, (rect.y1 as f32).min(bottom));
    (top, (end - top).max(0.0))
}

/// What was measured in one layout.
#[derive(Debug)]
struct Layout {
    editor_top: f32,
    editor: f32,
    output: f32,
}

fn measured(size: egui::Vec2, open: bool) -> Layout {
    measured_shell(size, shell(open))
}

fn measured_shell(size: egui::Vec2, shell: ShellState) -> Layout {
    let h = harness(size, shell);
    let editor_label = t("shell.editor_label");
    let (editor_top, editor) = span(&h, Role::MultilineTextInput, &editor_label, size.y);
    let (_, output) = span(&h, Role::Region, &t("shell.output.region"), size.y);
    Layout {
        editor_top,
        editor,
        output,
    }
}

const SMALL: egui::Vec2 = egui::vec2(800.0, 500.0);
const COMMON: egui::Vec2 = egui::vec2(1280.0, 800.0);

#[test]
fn with_the_panel_open_the_output_keeps_its_minimum_and_the_editor_stays_usable() {
    for size in [SMALL, COMMON] {
        let l = measured(size, true);
        assert!(l.output >= OUTPUT_MIN, "{size:?}: {l:?}");
        assert!(l.editor >= EDITOR_USABLE, "{size:?}: {l:?}");
    }
}

/// Closed, the layout is the one from before the panel existed: the editor takes 45% of the
/// height below the toolbar (at least 120 points), measured on the text box, which sits a few
/// points inside its scroll area.
#[test]
fn with_the_panel_closed_the_editor_takes_its_usual_share() {
    for size in [SMALL, COMMON] {
        let l = measured(size, false);
        let below_toolbar = size.y - PANEL_MARGIN - l.editor_top;
        let expected = (below_toolbar * 0.45).max(120.0);
        assert!(
            (l.editor - expected).abs() <= 10.0,
            "{size:?}: {l:?} vs {expected}"
        );
    }
}

/// The harness's bottom margin (points).
const PANEL_MARGIN: f32 = 8.0;

/// The run of [`shell`] returned a full table (200 documents of 40 fields, cell `r<i>c<f>`) and
/// FINISHED (so the finished-run notice is shown, as it is from the frame after the result came),
/// shown in `mode`.
fn with_table(open: bool, mode: OutputMode) -> ShellState {
    let docs = (0..200)
        .map(|i| {
            (0..40)
                .map(|f| {
                    (
                        format!("field_{f}"),
                        bson::Bson::String(format!("r{i}c{f}")),
                    )
                })
                .collect()
        })
        .collect();
    let result = ShellResult::Documents {
        kind: DocKind::Array,
        docs,
        more: true,
        total: Some(1000),
    };
    let finished = shell(open)
        .result_arrived(1, result)
        .finished(1, Outcome::Exited(0));
    assert!(finished.notice.is_some(), "the finished-run notice");
    finished.mode_set(mode)
}

/// How many `role` nodes whose label passes `wanted` are wholly inside the output region and
/// the window (rows cut by the pane's or the window's bottom do not count).
fn visible(
    h: &Harness<'static, ShellState>,
    role: Role,
    bottom: f32,
    wanted: impl Fn(&str) -> bool,
) -> usize {
    let name = t("shell.output.region");
    let region = h.get_by_role_and_label(Role::Region, &name);
    let pane = region.accesskit_node().bounding_box().expect("bounds");
    let floor = (pane.y1 as f32).min(bottom) + 0.5;
    h.get_all_by_role(role)
        .filter(|node| wanted(&node.accesskit_node().value().unwrap_or_default()))
        .filter_map(|node| node.accesskit_node().bounding_box())
        .filter(|rect| rect.y0 as f32 >= pane.y0 as f32 && rect.y1 as f32 <= floor)
        .count()
}

/// Table data rows: the visible cells of the first column (`r<i>c0`).
fn table_rows(h: &Harness<'static, ShellState>, bottom: f32) -> usize {
    visible(h, Role::Label, bottom, |label| {
        label.starts_with('r') && label.ends_with("c0")
    })
}

fn table_header(h: &Harness<'static, ShellState>, bottom: f32) -> usize {
    visible(h, Role::Label, bottom, |label| label == "field_0")
}

/// Tree rows: a document's heading or a field's key.
fn tree_rows(h: &Harness<'static, ShellState>, bottom: f32) -> usize {
    let heading = t("doc.header").replace("{n}", "");
    visible(h, Role::Label, bottom, |label| {
        label.starts_with("field_") || label.starts_with(heading.trim_end())
    })
}

/// The height from the first text line's top to the visible bottom of the output pane: the line
/// area Text mode gets (points).
fn text_area(h: &Harness<'static, ShellState>, bottom: f32) -> f32 {
    let name = t("shell.output.region");
    let region = h.get_by_role_and_label(Role::Region, &name);
    let pane = region.accesskit_node().bounding_box().expect("bounds");
    let first = h
        .get_all_by_role(Role::Label)
        .filter(|node| {
            node.accesskit_node()
                .value()
                .is_some_and(|v| v.starts_with("line "))
        })
        .filter_map(|node| node.accesskit_node().bounding_box())
        .map(|rect| rect.y0 as f32)
        .fold(f32::INFINITY, f32::min);
    ((pane.y1 as f32).min(bottom) - first).max(0.0)
}

/// Rows of the run's text lines (`line <n>`) in view.
fn text_rows(h: &Harness<'static, ShellState>, bottom: f32) -> usize {
    visible(h, Role::Label, bottom, |label| label.starts_with("line "))
}

/// At 800x500 with History open after a finished run (the notice shown), Table shows its header
/// and at least 3 data rows and Tree at least 3 rows; at 1280x800 comfortably more (8). The
/// output keeps its minimum and the editor stays usable throughout.
#[test]
fn after_a_finished_run_with_the_history_panel_open_the_result_shows_rows() {
    for (size, rows) in [(SMALL, 3), (COMMON, 8)] {
        let table = harness(size, with_table(true, OutputMode::Table));
        let shown = table_rows(&table, size.y);
        assert_eq!(table_header(&table, size.y), 1, "{size:?}: the header");
        assert!(shown >= rows, "{size:?}: {shown} table rows");
        let tree = harness(size, with_table(true, OutputMode::Tree));
        let shown = tree_rows(&tree, size.y);
        assert!(shown >= rows, "{size:?}: {shown} tree rows");
        for mode in [OutputMode::Text, OutputMode::Table, OutputMode::Tree] {
            let l = measured_shell(size, with_table(true, mode));
            assert!(l.output >= OUTPUT_MIN, "{mode:?} {size:?}: {l:?}");
            assert!(l.editor >= EDITOR_USABLE, "{mode:?} {size:?}: {l:?}");
        }
    }
}

/// Text mode's line area (points) after a finished run with History open, before this slice:
/// measured on 4fd12ff with the mode switch taken out of the heading row (800x500, 1280x800).
const TEXT_AREA_BEFORE: [(egui::Vec2, f32); 2] = [(SMALL, 62.5), (COMMON, 97.0)];
/// How much less than that Text mode may get.
const TEXT_AREA_SLACK: f32 = 4.0;

#[test]
fn after_a_finished_run_text_mode_gets_no_less_room_than_before_the_mode_switch() {
    for (size, before) in TEXT_AREA_BEFORE {
        let h = harness(size, with_table(true, OutputMode::Text));
        let area = text_area(&h, size.y);
        assert!(area >= before - TEXT_AREA_SLACK, "{size:?}: {area}");
        assert!(text_rows(&h, size.y) >= 2, "{size:?}");
    }
}

/// One small text row, with its item spacing (points).
const LINE_ROW: f32 = 24.0;

/// A script with a syntax problem adds the problem line under the editor. It is chrome, measured
/// like the rest: with the panel open the output keeps its minimum and the editor stays usable;
/// with it closed the editor keeps its usual share and the line costs the output one small row at
/// most. Under the notice of a stopped Run the line is not drawn for the same problem, so the
/// notice's layout is the one from before the line existed.
#[test]
fn a_problem_line_keeps_the_planned_layout() {
    for size in [SMALL, COMMON] {
        for open in [false, true] {
            let good = with_table(open, OutputMode::Table);
            let broken = good.edited("db.a.find(".into(), None);
            let before = measured_shell(size, good);
            let l = measured_shell(size, broken);
            let at = format!("{size:?} open={open}: {l:?} vs {before:?}");
            match open {
                true => {
                    assert!(l.output >= OUTPUT_MIN, "{at}");
                    assert!(l.editor >= EDITOR_USABLE, "{at}");
                }
                false => {
                    assert!((l.editor - before.editor).abs() <= 1.0, "{at}");
                    assert!(before.output - l.output <= LINE_ROW, "{at}");
                }
            }
        }
    }
}

/// Two flagged queries and one call that could not be checked.
fn index_miss_report() -> curlybrace_core::guard::GuardReport {
    use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
    use curlybrace_core::guard::{GuardFinding, GuardReport};
    use curlybrace_core::store::{Namespace, Query};
    let finding = |collection: &str, kind| GuardFinding {
        ns: Namespace::new("shop", collection),
        method: "find".into(),
        query: Query::default(),
        kind,
        reasons: vec![Reason::new("collscan")],
        suggestion: Some(SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
    };
    GuardReport {
        findings: vec![
            finding("orders", VerdictKind::CollectionScan),
            finding("users", VerdictKind::SortNotCovered),
        ],
        unchecked: 1,
    }
}

/// The index notice (two findings and an unchecked line) after a finished run, History open:
/// the editor stays usable and the output is no shorter than under the syntax notice, the
/// tallest notice the tab had before (at 800x500 that notice already leaves less than the usual
/// minimum; the index notice must not take more). At 1280x800 the findings area is taller and
/// the output keeps its minimum.
#[test]
fn the_index_notice_keeps_the_output_as_tall_as_the_syntax_notice_does() {
    for size in [SMALL, COMMON] {
        let idle = shell(true).finished(1, Outcome::Exited(0));
        let (syntax, _) = idle.edited("db.a.find(".into(), None).run_requested();
        let missed = idle.checking(2, None, 0).index_missed(index_miss_report());
        let before = measured_shell(size, syntax);
        let l = measured_shell(size, missed);
        let at = format!("{size:?}: {l:?} vs {before:?}");
        assert!(l.editor >= EDITOR_USABLE, "{at}");
        match size == SMALL {
            true => assert!(l.output >= before.output - 1.0, "{at}"),
            false => assert!(l.output >= OUTPUT_MIN, "{at}"),
        }
    }
}
