//! The Aggregate tab's confirmed write run: the Run button of the header (hidden for a read
//! pipeline, off with its reason otherwise), the status line of the last run, and the dialog that
//! asks the person to type the target collection's name before `$out` / `$merge` writes. Draws
//! what it is given and returns what the person did; the reducer decides what happens.
use super::aggregate::pipeline_error_text;
use super::connection_badge::show_badge;
use super::index_dialogs::focus_once;
use super::tone_notice::tone_notice;
use super::typed_confirmation::{TypedConfirmation, show_typed_confirmation};
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme::{Tone, filled_button, gate, modal_frame, text_button, tonal_button, typography};
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::aggregate_run::{RunAvailability, RunDialog, RunDisabled, RunStatus};
use curlybrace_core::store::Namespace;
use eframe::egui;

const MODAL_WIDTH: f32 = 520.0;
/// What the dialog keeps for its frame, title, buttons and the window's edge.
const MODAL_CHROME: f32 = 220.0;
const MIN_BODY_HEIGHT: f32 = 120.0;
/// The room kept between the dialog and the window's side edges.
const SIDE_ROOM: f32 = 80.0;

/// What the run controls need from the session around the draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunContext {
    pub availability: RunAvailability,
    /// The active connection's tag; the production chip rides in the dialog.
    pub badge: ConnectionBadge,
}

/// The i18n key of the reason Run is off, for the reasons that have a fixed text.
fn disabled_key(reason: &RunDisabled) -> Option<&'static str> {
    match reason {
        RunDisabled::ReadOnly => Some("aggregate.run.disabled.read_only"),
        RunDisabled::NoConnection => Some("aggregate.run.disabled.no_connection"),
        RunDisabled::NoCollection => Some("aggregate.run.disabled.no_collection"),
        RunDisabled::Busy => Some("aggregate.run.disabled.busy"),
        RunDisabled::TextNotApplied => Some("aggregate.run.disabled.text_not_applied"),
        RunDisabled::Invalid(_) => None,
    }
}

fn disabled_text(reason: &RunDisabled, i18n: &I18n) -> String {
    match (disabled_key(reason), reason) {
        (Some(key), _) => i18n.t(key),
        (None, RunDisabled::Invalid(error)) => pipeline_error_text(error, i18n),
        (None, _) => String::new(),
    }
}

/// `db.collection`, as it is written everywhere in the tab.
fn dotted(ns: &Namespace) -> String {
    format!("{}.{}", ns.db, ns.collection)
}

/// The Run button, inside the header's wrapping row; nothing for a pipeline that does not write.
pub fn show_run_button(
    ui: &mut egui::Ui,
    availability: &RunAvailability,
    i18n: &I18n,
) -> Option<AggregateAction> {
    let label = i18n.t("aggregate.run.button");
    match availability {
        RunAvailability::Hidden => None,
        RunAvailability::Ready => tonal_button(ui, label)
            .clicked()
            .then_some(AggregateAction::RunClicked),
        RunAvailability::Disabled(reason) => {
            let why = disabled_text(reason, i18n);
            let off = gate(ui, false, |ui| tonal_button(ui, label)).on_hover_text(&why);
            ui.ctx()
                .accesskit_node_builder(off.id, |node| node.set_description(why));
            None
        }
    }
}

/// The status of the last run under the header; nothing before the first run.
pub fn show_run_status(ui: &mut egui::Ui, status: &RunStatus, i18n: &I18n) {
    match status {
        RunStatus::Idle => {}
        RunStatus::Running => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(i18n.t("aggregate.run.status.running"));
            });
        }
        RunStatus::Done { target, count } => {
            let name = dotted(target);
            let text = match count {
                Some(n) => i18n.t_with(
                    "aggregate.run.status.done",
                    &[("target", &name), ("count", &n.to_string())],
                ),
                None => i18n.t_with("aggregate.run.status.done_no_count", &[("target", &name)]),
            };
            tone_notice(ui, Tone::Good, &text, false);
        }
        RunStatus::Failed(error) => {
            let reason = i18n.error(error);
            let text = i18n.t_with("aggregate.run.status.failed", &[("error", &reason)]);
            tone_notice(ui, Tone::Bad, &text, false);
        }
        RunStatus::Unknown => {
            let text = i18n.t("aggregate.run.status.unknown");
            tone_notice(ui, Tone::Warning, &text, false);
        }
    }
}

/// The confirmation dialog of the draft's run, if one is open. Focus starts in the name field,
/// Enter there confirms a matching name, Escape and a click outside cancel.
pub fn show_run_dialog(
    ctx: &egui::Context,
    dialog: Option<&RunDialog>,
    badge: &ConnectionBadge,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let Some(dialog) = dialog else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    let screen = ctx.screen_rect();
    let width = MODAL_WIDTH.min(screen.width() - SIDE_ROOM);
    let modal = egui::Modal::new(egui::Id::new("aggregate_run_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                show_badge(ui, badge, i18n);
                ui.label(typography::dialog_title(ui, i18n.t("aggregate.run.title")));
            });
            let max_height = (screen.height() - MODAL_CHROME).max(MIN_BODY_HEIGHT);
            let (typed, entered) = egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    facts(ui, dialog, i18n);
                    confirmation(ui, dialog, i18n)
                })
                .inner;
            actions.extend(typed.map(AggregateAction::RunTyped));
            actions.extend(entered.then_some(AggregateAction::RunConfirmed));
            actions.extend(buttons(ui, dialog, i18n));
        });
    if actions.is_empty() && modal.should_close() {
        actions.push(AggregateAction::RunDismissed);
    }
    actions
}

/// What the run reads, writes and does, in plain words.
fn facts(ui: &mut egui::Ui, dialog: &RunDialog, i18n: &I18n) {
    let source = dotted(&dialog.source);
    let target = dotted(&dialog.target_ns());
    ui.label(i18n.t_with("aggregate.run.source", &[("source", &source)]));
    ui.label(i18n.t_with("aggregate.run.target", &[("target", &target)]));
    let stage_key = match dialog.stage.as_str() {
        "$out" => "aggregate.run.stage_out",
        _ => "aggregate.run.stage_merge",
    };
    ui.label(i18n.t(stage_key));
    let count = match dialog.target_count {
        Some(n) => i18n.t_with("aggregate.run.count", &[("count", &n.to_string())]),
        None => i18n.t("aggregate.run.count_unavailable"),
    };
    ui.label(count);
    let irreversible = i18n.t("aggregate.run.irreversible");
    tone_notice(ui, Tone::Warning, &irreversible, true);
}

/// The typed confirmation; the text typed this frame, and whether Enter confirmed a matching one.
fn confirmation(ui: &mut egui::Ui, dialog: &RunDialog, i18n: &I18n) -> (Option<String>, bool) {
    let name = dialog.target.collection.as_str();
    let accessible = i18n.t_with("aggregate.run.type_accessible", &[("name", name)]);
    let prompt = TypedConfirmation {
        instruction: &i18n.t("aggregate.run.type_instruction"),
        required: name,
        field_label: &i18n.t("aggregate.run.type_field"),
        accessible_name: &accessible,
    };
    let mut text = dialog.typed.clone();
    let field = show_typed_confirmation(ui, &prompt, &mut text);
    focus_once(ui, &field, ("aggregate_run_focus", "typed"));
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let matches = text.trim() == name;
    (field.changed().then_some(text), entered && matches)
}

/// Cancel and the Run button (off, with its reason, until the typed name matches).
fn buttons(ui: &mut egui::Ui, dialog: &RunDialog, i18n: &I18n) -> Option<AggregateAction> {
    ui.horizontal(|ui| {
        let cancel = text_button(ui, i18n.t("common.cancel")).clicked();
        let ready = dialog.is_confirmed();
        let run = gate(ui, ready, |ui| {
            filled_button(ui, i18n.t("aggregate.run.confirm"))
        });
        if !ready {
            let why = i18n.t("aggregate.run.confirm_blocked");
            let run = run.clone().on_hover_text(&why);
            ui.ctx()
                .accesskit_node_builder(run.id, |node| node.set_description(why));
        }
        match (cancel, ready && run.clicked()) {
            (_, true) => Some(AggregateAction::RunConfirmed),
            (true, false) => Some(AggregateAction::RunDismissed),
            _ => None,
        }
    })
    .inner
}

#[cfg(test)]
#[path = "aggregate_run_tests.rs"]
mod tests;
