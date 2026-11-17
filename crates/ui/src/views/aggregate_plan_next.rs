//! What the Plan tab offers under a plan: Try an index (a combo of the collection's indexes that
//! sets the hint and explains again) with Clear hint, and the suggested index with its command,
//! Copy command and Create index.... Draws what the view-model computed and returns what the
//! person did; the reducer decides what happens. Off controls give their reason as their tooltip
//! and their accessible description.
use super::aggregate_run::RunContext;
use crate::I18n;
use crate::app::{AggregateAction, ExplainAction, OptionsAction};
use crate::theme::{gate, text_button, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_hint_list::HintList;
use crate::viewmodel::aggregate_plan_next::{PlanNext, plan_next};
use eframe::egui::{self, WidgetInfo, WidgetType};

/// The follow-ups under the plan; the actions of this frame.
pub(super) fn show_plan_next(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    context: &RunContext,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let next = plan_next(draft, context.ns.as_ref(), !context.disconnected);
    let mut actions = Vec::new();
    if next.try_index {
        show_try_row(ui, draft, &next, &mut actions, i18n);
    }
    if let (Some(command), Some(_)) = (&next.command, &next.suggested) {
        show_suggestion(ui, command, context, &mut actions, i18n);
    }
    actions
}

/// Gives an off control its reason: the tooltip and the accessible description.
fn explained(ui: &egui::Ui, response: egui::Response, reason: Option<&str>) -> egui::Response {
    let Some(reason) = reason else {
        return response;
    };
    let why = reason.to_string();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(why));
    response.on_hover_text(reason)
}

/// Try an index (the combo) and Clear hint (only with a hint set).
fn show_try_row(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    next: &PlanNext,
    actions: &mut Vec<AggregateAction>,
    i18n: &I18n,
) {
    let blocked = next.blocked.map(|key| i18n.t(key));
    let hint = &draft.options_ui.hint;
    if hint.wants_list() {
        actions.push(AggregateAction::Options(OptionsAction::HintListWanted));
    }
    ui.horizontal_wrapped(|ui| {
        let label = i18n.t("aggregate.options.explain.try_index");
        ui.label(label.clone());
        match &hint.list {
            HintList::NotAsked | HintList::Loading { .. } => {
                ui.label(i18n.t("aggregate.options.hint.loading"));
            }
            HintList::Failed => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    i18n.t("aggregate.options.hint.list_failed"),
                );
            }
            HintList::Loaded(names) => {
                let shown = next
                    .current_hint
                    .clone()
                    .unwrap_or_else(|| i18n.t("aggregate.options.explain.no_hint"));
                let picked = try_combo(ui, &label, &shown, names, blocked.as_deref(), i18n);
                actions.extend(
                    picked.map(|name| AggregateAction::Explain(ExplainAction::TryIndex(name))),
                );
            }
        }
        if next.current_hint.is_some() {
            let clear = gate(ui, blocked.is_none(), |ui| {
                text_button(ui, i18n.t("aggregate.options.explain.clear_hint"))
            });
            let clear = explained(ui, clear, blocked.as_deref());
            if clear.clicked() && blocked.is_none() {
                actions.push(AggregateAction::Explain(ExplainAction::ClearHint));
            }
        }
    });
}

/// The indexes combo named `label`; the name picked this frame. Off, it opens nothing and says why.
fn try_combo(
    ui: &mut egui::Ui,
    label: &str,
    shown: &str,
    names: &[String],
    blocked: Option<&str>,
    i18n: &I18n,
) -> Option<String> {
    let mut picked = None;
    let combo = ui
        .add_enabled_ui(blocked.is_none(), |ui| {
            egui::ComboBox::from_id_salt("try_index")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for name in names {
                        if ui.selectable_label(shown == name, name).clicked() {
                            picked = Some(name.clone());
                        }
                    }
                })
        })
        .inner;
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, blocked.is_none(), label));
    let reason = blocked
        .map(str::to_string)
        .unwrap_or_else(|| i18n.t("aggregate.options.explain.try_index_hint"));
    explained(ui, combo.response, Some(&reason));
    picked
}

/// The suggested index: its command (selectable, read-only), Copy command and Create index....
fn show_suggestion(
    ui: &mut egui::Ui,
    command: &str,
    context: &RunContext,
    actions: &mut Vec<AggregateAction>,
    i18n: &I18n,
) {
    ui.add_space(ui.spacing().item_spacing.y);
    ui.strong(i18n.t("aggregate.options.explain.suggested"));
    ui.add(egui::Label::new(egui::RichText::new(command).monospace()).selectable(true));
    ui.horizontal_wrapped(|ui| {
        if tonal_button(ui, i18n.t("aggregate.options.explain.copy_create_index")).clicked() {
            ui.ctx().copy_text(command.to_string());
            actions.push(AggregateAction::Explain(ExplainAction::CopyCreateIndex));
        }
        let allowed = !context.read_only;
        let create = gate(ui, allowed, |ui| {
            tonal_button(ui, i18n.t("aggregate.options.explain.create_index"))
        });
        let reason = i18n.t("aggregate.options.explain.create_index_read_only");
        let create = explained(ui, create, (!allowed).then_some(reason.as_str()));
        if create.clicked() && allowed {
            actions.push(AggregateAction::Explain(ExplainAction::CreateIndex));
        }
    });
}

#[cfg(test)]
#[path = "aggregate_plan_next_tests.rs"]
mod tests;
