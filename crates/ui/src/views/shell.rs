//! The Shell tab: the connection it runs on with the Run / Cancel / Format toolbar on the same
//! row (with `Cmd/Ctrl+Enter` and `Cmd/Ctrl+Shift+F`, and the finished run's notice as its
//! status), why it may not run, the script editor with the first syntax problem under it, any other notice, the
//! History panel and the output pane (with its `Text | Table | Tree` switch). It only draws and
//! reports; every decision is the reducer's.
//!
//! `Esc` is not bound to Cancel: in the editor it hands keyboard focus back (the editor keeps
//! `Tab` for indenting), so binding it would cancel a run whenever someone leaves the editor.
use super::documents::show_connection_header_with;
use super::guard_notice::GuardNoticeAction;
use super::shell_editor::{editor_id, show_editor};
use super::shell_guard::{show_checking, show_index_miss};
use super::shell_history::show_history;
use super::shell_layout::{heights, last_chrome, remember_chrome};
use super::shell_notice::{notice_text, offers_problem_actions, offers_tool_paths};
use super::shell_output::show_output;
use super::shell_problem::show_problem_line;
use super::tone_notice::{tone_notice, tone_status};
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::{CONTROL_HEIGHT, Tone, filled_button, gate, text_button, tonal_button};
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::shell::{ShellNotice, ShellState};
use curlybrace_core::guard::GuardReport;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use uuid::Uuid;

/// Runs the selection, or the whole script.
pub const RUN_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
/// Re-indents the selection, or the whole script.
pub const FORMAT_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);

/// What the tab needs to know about the session it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct ShellViewContext<'a> {
    /// The session the tab belongs to; each session has its own editor state.
    pub session: Uuid,
    /// The connection's display name.
    pub connection: &'a str,
    /// Its color, production and read-only flags.
    pub badge: ConnectionBadge,
    /// Whether the session is connected right now.
    pub connected: bool,
    /// The frame's wall-clock time (Unix seconds), read once by the caller; history rows say
    /// how long ago they ran against it.
    pub now_secs: i64,
    /// The collection names the editor's completion offers: those of every database of the
    /// session whose collections are loaded, sorted and without duplicates.
    pub collections: &'a [String],
}

/// What the user asked for: a shell action, or the Tool paths dialog (offered when `mongosh`
/// is missing or too old).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellViewEvent {
    Action(ShellAction),
    OpenToolPaths,
}

/// Which buttons work in this frame.
#[derive(Clone, Copy)]
struct Gates {
    run: bool,
    cancel: bool,
    format: bool,
}

impl Gates {
    /// The gates for `script`, which is the script as edited in this frame (a shortcut pressed
    /// right after typing judges the typed text).
    fn of(state: &ShellState, script: &str, ctx: &ShellViewContext<'_>) -> Self {
        let has_text = !script.trim().is_empty();
        Self {
            run: ctx.connected && !ctx.badge.read_only && !state.is_busy() && has_text,
            // A running shell can always be stopped, and a check before a run dropped.
            cancel: state.is_busy(),
            // Formatting edits the draft only, so it works offline and on read-only connections.
            format: has_text,
        }
    }
}

/// The shortcuts pressed in this frame (before their gates are checked).
#[derive(Clone, Copy, Default)]
struct Pressed {
    run: bool,
    format: bool,
}

/// Draws the tab and returns every event of the frame in the order the reducer must apply them:
/// the editor's `Edit` first, so a Run or Format in the same frame acts on the text as typed,
/// then the shortcut or button, then the notice's own button (Tool paths, or Go to problem and
/// Run anyway).
pub fn show_shell(
    ui: &mut egui::Ui,
    state: &ShellState,
    ctx: ShellViewContext<'_>,
    i18n: &I18n,
) -> Vec<ShellViewEvent> {
    let pressed = shortcuts(ui, ctx.session);
    // The connection and the toolbar share a row; why runs are blocked comes right under it.
    let gates = Gates::of(state, &state.script, &ctx);
    let clicked = show_connection_header_with(ui, ctx.connection, &ctx.badge, i18n, |ui| {
        toolbar(ui, state, &ctx, gates, i18n)
    });
    show_conditions(ui, &ctx, i18n);
    let open = state.history.open;
    let top = ui.cursor().min.y;
    let planned = heights(ui.available_height(), open, last_chrome(ui, ctx.session));
    let edited = show_editor(
        ui,
        state,
        ctx.session,
        planned.editor,
        ctx.collections,
        i18n,
    );
    // The text as typed in this frame, so the problem line never lags a key behind.
    let typed = match &edited {
        Some(ShellAction::Edit { script, .. }) => script.as_str(),
        _ => state.script.as_str(),
    };
    let editor_used = ui.cursor().min.y - top;
    // Part of the chrome (measured, see `shell_layout`): it takes a row only while there is a
    // problem, and the next frame plans the heights with it.
    let in_notice = match &state.notice {
        Some(ShellNotice::SyntaxProblem { problem, .. }) => Some(problem),
        _ => None,
    };
    show_problem_line(ui, editor_id(ctx.session), typed, in_notice, i18n);
    // One wrapped label: two would cost a line break and a gap of the output pane's height.
    let (editor_hint, script_hint) = (i18n.t("shell.editor_hint"), i18n.t("shell.script_hint"));
    let hints = i18n.t_with(
        "shell.hints",
        &[("editor", &editor_hint), ("script", &script_hint)],
    );
    ui.label(egui::RichText::new(hints).small());
    // A finished run's notice is the toolbar's status (see `toolbar`); the others are read here.
    let notice = state
        .notice
        .as_ref()
        .filter(|notice| !matches!(notice, ShellNotice::Finished(_)))
        .and_then(|notice| show_notice(ui, notice, i18n));
    let panel = show_history(
        ui,
        &state.history,
        ctx.session,
        ctx.now_secs,
        planned.list,
        i18n,
    );
    ui.separator();
    if open {
        let used = ui.cursor().min.y - top;
        remember_chrome(ui, ctx.session, used - editor_used - panel.list_height);
    }
    let history = panel.action;
    let output = show_output(ui, state, ctx.session, i18n);
    let shortcut = gated(pressed, Gates::of(state, typed, &ctx));
    edited
        .into_iter()
        .chain(shortcut.or(clicked))
        .chain(history)
        .chain(output)
        .map(ShellViewEvent::Action)
        .chain(notice)
        .collect()
}

/// The keyboard shortcuts, read before the editor so `Enter` and `F` are not typed into it.
///
/// Rule: they belong to the Shell tab only while the keyboard is in its editor, or nowhere at
/// all (nothing focused). With focus on any other widget (a sidebar row, a text field, a button)
/// the keys are left alone for that widget, so one key press never does two things. Under a
/// modal dialog they are ignored too. Inside the rule they are consumed even when their button
/// is disabled, so the keys are never typed into the script.
fn shortcuts(ui: &mut egui::Ui, session: Uuid) -> Pressed {
    let editor = editor_id(session);
    let ours = ui.ctx().memory(|m| {
        m.top_modal_layer().is_none() && (m.has_focus(editor) || m.focused().is_none())
    });
    if !ours {
        return Pressed::default();
    }
    ui.input_mut(|i| Pressed {
        run: i.consume_shortcut(&RUN_SHORTCUT),
        format: i.consume_shortcut(&FORMAT_SHORTCUT),
    })
}

fn gated(pressed: Pressed, gates: Gates) -> Option<ShellAction> {
    match (pressed.run && gates.run, pressed.format && gates.format) {
        (true, _) => Some(ShellAction::Run),
        (false, true) => Some(ShellAction::Format),
        _ => None,
    }
}

/// Why runs are blocked or need confirming, said up front rather than on a click.
fn show_conditions(ui: &mut egui::Ui, ctx: &ShellViewContext<'_>, i18n: &I18n) {
    if !ctx.connected {
        tone_notice(
            ui,
            Tone::Warning,
            &i18n.t("shell.not_connected_note"),
            false,
        );
    }
    if ctx.badge.read_only {
        tone_notice(ui, Tone::Warning, &i18n.t("shell.read_only_note"), false);
    }
    if ctx.badge.production {
        tone_notice(ui, Tone::Warning, &i18n.t("shell.production_note"), false);
    }
}

fn toolbar(
    ui: &mut egui::Ui,
    state: &ShellState,
    ctx: &ShellViewContext<'_>,
    gates: Gates,
    i18n: &I18n,
) -> Option<ShellAction> {
    let run_shortcut = ui.ctx().format_shortcut(&RUN_SHORTCUT);
    let format_shortcut = ui.ctx().format_shortcut(&FORMAT_SHORTCUT);
    // Its own wrapping area beside the connection, as tall as a control from the start so the
    // buttons line up with the name.
    let size = egui::vec2(ui.available_width(), CONTROL_HEIGHT);
    let wrapped = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
    ui.allocate_ui_with_layout(size, wrapped, |ui| {
        let run = gate(ui, gates.run, |ui| filled_button(ui, i18n.t("shell.run")))
            .on_hover_text(run_hint(state, ctx, &run_shortcut, i18n));
        let cancel = gate(ui, gates.cancel, |ui| {
            tonal_button(ui, i18n.t("shell.cancel"))
        })
        .on_hover_text(i18n.t("shell.cancel_tooltip"));
        let format = gate(ui, gates.format, |ui| {
            text_button(ui, i18n.t("shell.format"))
        })
        .on_hover_text(i18n.t_with("shell.format_tooltip", &[("shortcut", &format_shortcut)]));
        if state.is_running() {
            ui.label(i18n.t("shell.status_running"));
        } else if state.is_busy() {
            // Busy and not running: the script's queries are being checked.
            show_checking(ui, i18n);
        }
        if let Some(notice @ ShellNotice::Finished(_)) = &state.notice {
            let (severity, text) = notice_text(notice, i18n);
            tone_status(ui, severity.tone(), &text);
        }
        match (run.clicked(), cancel.clicked(), format.clicked()) {
            (true, _, _) => Some(ShellAction::Run),
            (_, true, _) => Some(ShellAction::Cancel),
            (_, _, true) => Some(ShellAction::Format),
            _ => None,
        }
    })
    .inner
}

/// The Run tooltip: what it does, or why it cannot right now.
fn run_hint(state: &ShellState, ctx: &ShellViewContext<'_>, shortcut: &str, i18n: &I18n) -> String {
    let key = if !ctx.connected {
        "shell.run_disabled_not_connected"
    } else if ctx.badge.read_only {
        "shell.run_disabled_read_only"
    } else if state.is_running() {
        "shell.run_disabled_running"
    } else if state.is_busy() {
        // Busy and not running: the script's queries are being checked.
        "shell.run_disabled_checking"
    } else if state.script.trim().is_empty() {
        "shell.run_disabled_empty"
    } else {
        return i18n.t_with("shell.run_tooltip", &[("shortcut", shortcut)]);
    };
    i18n.t(key)
}

/// The notice of the last run or refusal; a missing or too old `mongosh` offers the Tool paths
/// dialog, a syntax problem offers Go to problem and Run anyway.
fn show_notice(ui: &mut egui::Ui, notice: &ShellNotice, i18n: &I18n) -> Option<ShellViewEvent> {
    if let ShellNotice::IndexMiss {
        report, unchecked, ..
    } = notice
    {
        return index_miss_event(ui, report, *unchecked, i18n);
    }
    let (severity, text) = notice_text(notice, i18n);
    tone_notice(ui, severity.tone(), &text, false);
    if offers_problem_actions(notice) {
        return problem_actions(ui, i18n).map(ShellViewEvent::Action);
    }
    let open =
        offers_tool_paths(notice) && text_button(ui, i18n.t("shell.open_tool_paths")).clicked();
    open.then_some(ShellViewEvent::OpenToolPaths)
}

/// The index notice: Run anyway and Cancel are shell actions, Copy goes to the clipboard.
fn index_miss_event(
    ui: &mut egui::Ui,
    report: &GuardReport,
    unchecked: usize,
    i18n: &I18n,
) -> Option<ShellViewEvent> {
    match show_index_miss(ui, report, unchecked, i18n)? {
        GuardNoticeAction::RunAnyway => Some(ShellViewEvent::Action(ShellAction::RunAnyway)),
        GuardNoticeAction::Cancel => Some(ShellViewEvent::Action(ShellAction::Cancel)),
        GuardNoticeAction::Copy(command) => {
            ui.ctx().copy_text(command);
            None
        }
    }
}

/// Go to problem (selects it in the editor) and Run anyway (the check can be wrong).
fn problem_actions(ui: &mut egui::Ui, i18n: &I18n) -> Option<ShellAction> {
    ui.horizontal_wrapped(|ui| {
        let go_to = text_button(ui, i18n.t("shell.check.go_to")).clicked();
        let anyway = text_button(ui, i18n.t("shell.check.run_anyway")).clicked();
        match (go_to, anyway) {
            (true, _) => Some(ShellAction::GoToProblem),
            (_, true) => Some(ShellAction::RunAnyway),
            _ => None,
        }
    })
    .inner
}
