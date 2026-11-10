//! The Shell tab: the connection it runs on and why it may not run, the Run / Cancel / Format
//! toolbar (with `Cmd/Ctrl+Enter` and `Cmd/Ctrl+Shift+F`), the script editor, the notice of the
//! last run and the output pane. It only draws and reports; every decision is the reducer's.
//!
//! `Esc` is not bound to Cancel: in the editor it hands keyboard focus back (the editor keeps
//! `Tab` for indenting), so binding it would cancel a run whenever someone leaves the editor.
use super::documents::show_connection_header;
use super::shell_editor::show_editor;
use super::shell_notice::{notice_text, offers_tool_paths};
use super::shell_output::show_output;
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::{Tone, filled_button, gate, text_button, tonal_button};
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::shell::{ShellNotice, ShellState};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

/// Runs the selection, or the whole script.
pub const RUN_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
/// Re-indents the selection, or the whole script.
pub const FORMAT_SHORTCUT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);

/// The share of the tab's height the editor takes; the output pane gets the rest.
const EDITOR_SHARE: f32 = 0.45;
/// The editor is never shorter than this (points).
const EDITOR_MIN_HEIGHT: f32 = 120.0;

/// What the tab needs to know about the session it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct ShellViewContext<'a> {
    /// The connection's display name.
    pub connection: &'a str,
    /// Its color, production and read-only flags.
    pub badge: ConnectionBadge,
    /// Whether the session is connected right now.
    pub connected: bool,
}

/// What the user asked for: a shell action, or the Tool paths dialog (offered when `mongosh`
/// is missing, too old or could not be started).
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
    fn of(state: &ShellState, ctx: &ShellViewContext<'_>) -> Self {
        Self {
            run: ctx.connected && !ctx.badge.read_only && state.can_run(),
            // A running shell can always be stopped.
            cancel: state.is_running(),
            // Formatting edits the draft only, so it works offline and on read-only connections.
            format: !state.script.trim().is_empty(),
        }
    }
}

/// Draws the tab. At most one event per frame: a shortcut wins over an edit made in the same
/// frame (both in one frame takes two key events within a few milliseconds).
pub fn show_shell(
    ui: &mut egui::Ui,
    state: &ShellState,
    ctx: ShellViewContext<'_>,
    i18n: &I18n,
) -> Option<ShellViewEvent> {
    let gates = Gates::of(state, &ctx);
    let shortcut = shortcuts(ui, gates);
    show_connection_header(ui, ctx.connection, &ctx.badge, i18n);
    show_conditions(ui, &ctx, i18n);
    let clicked = toolbar(ui, state, &ctx, gates, i18n);
    let height = (ui.available_height() * EDITOR_SHARE).max(EDITOR_MIN_HEIGHT);
    let edited = show_editor(ui, state, height, i18n);
    ui.label(egui::RichText::new(i18n.t("shell.editor_hint")).small());
    let notice = state
        .notice
        .as_ref()
        .and_then(|notice| show_notice(ui, notice, i18n));
    ui.separator();
    show_output(ui, state, i18n);
    shortcut
        .or(clicked)
        .map(ShellViewEvent::Action)
        .or(notice)
        .or(edited.map(ShellViewEvent::Action))
}

/// The keyboard shortcuts, read before the editor so `Enter` and `F` are not typed into it.
/// They are consumed even when their button is disabled, and ignored under a modal dialog.
fn shortcuts(ui: &mut egui::Ui, gates: Gates) -> Option<ShellAction> {
    if ui.ctx().memory(|m| m.top_modal_layer().is_some()) {
        return None;
    }
    let (run, format) = ui.input_mut(|i| {
        (
            i.consume_shortcut(&RUN_SHORTCUT),
            i.consume_shortcut(&FORMAT_SHORTCUT),
        )
    });
    match (run && gates.run, format && gates.format) {
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
    ui.horizontal_wrapped(|ui| {
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
    } else if state.script.trim().is_empty() {
        "shell.run_disabled_empty"
    } else {
        return i18n.t_with("shell.run_tooltip", &[("shortcut", shortcut)]);
    };
    i18n.t(key)
}

/// The notice of the last run or refusal; a `mongosh` problem offers the Tool paths dialog.
fn show_notice(ui: &mut egui::Ui, notice: &ShellNotice, i18n: &I18n) -> Option<ShellViewEvent> {
    let (severity, text) = notice_text(notice, i18n);
    tone_notice(ui, severity.tone(), &text, false);
    let open =
        offers_tool_paths(notice) && text_button(ui, i18n.t("shell.open_tool_paths")).clicked();
    open.then_some(ShellViewEvent::OpenToolPaths)
}
