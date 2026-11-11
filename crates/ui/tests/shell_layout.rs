//! The Shell tab's vertical layout with the History panel open or closed: the output pane keeps
//! a usable height and the editor stays usable, at a small (800x500) and a common (1280x800)
//! window. Measured from the accesskit bounds of the editor and the output region. Fast, offline,
//! no GPU.
use curlybrace_core::history::{ShellHistoryEntry, ShellRunStatus};
use curlybrace_core::shell::Stream;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::ShellState;
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
    let h = harness(size, shell(open));
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
