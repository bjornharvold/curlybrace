//! The bulk update / delete dialog: one modal for the whole flow (Define, Preview, Confirm,
//! Running, Report), so the production notice rides inside it and no second warning stacks on top.
//! It draws the flow it is given and reports what the person did; the reducer decides what
//! happens. Cancel is first and owns the initial focus of every step, so a stray Enter never
//! confirms; Escape and a click outside close the dialog, except while the write is running.
use super::bulk_dialog_steps::{
    confirm_body, define_body, preview_body, report_body, running_body,
};
use super::connection_badge::show_badge;
use super::index_dialogs::focus_once;
use crate::I18n;
use crate::app::BulkAction;
use crate::theme::{filled_button, gate, modal_frame, text_button, typography};
use crate::viewmodel::{BulkFlow, BulkKind, BulkStep, ConnectionBadge};
use eframe::egui;

const MODAL_WIDTH: f32 = 640.0;

/// Draws the dialog for `flow` on the connection named `connection`; returns what the person
/// did this frame, if anything.
pub fn show_bulk_dialog(
    ctx: &egui::Context,
    flow: &BulkFlow,
    connection: &str,
    badge: &ConnectionBadge,
    i18n: &I18n,
) -> Option<BulkAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("bulk_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            heading(ui, flow, badge, i18n);
            let edit = body(ui, flow, connection, i18n);
            let navigation = footer(ui, flow, i18n);
            action = navigation.or(edit);
        });
    // A running write is only ever stopped by its Cancel button.
    let dismissed = modal.should_close() && flow.step != BulkStep::Running;
    action.or(dismissed.then_some(BulkAction::Close))
}

fn heading(ui: &mut egui::Ui, flow: &BulkFlow, badge: &ConnectionBadge, i18n: &I18n) {
    let title = match flow.kind {
        BulkKind::Update => "bulk.title_update",
        BulkKind::Delete => "bulk.title_delete",
    };
    ui.horizontal(|ui| {
        show_badge(ui, badge, i18n);
        ui.label(typography::dialog_title(ui, i18n.t(title)));
    });
    if let Some(step) = step_key(flow.step) {
        ui.weak(i18n.t(step));
    }
}

fn step_key(step: BulkStep) -> Option<&'static str> {
    match step {
        BulkStep::Define => Some("bulk.step_define"),
        BulkStep::Preview => Some("bulk.step_preview"),
        BulkStep::Confirm => Some("bulk.step_confirm"),
        BulkStep::Running | BulkStep::Report => None,
    }
}

fn body(ui: &mut egui::Ui, flow: &BulkFlow, connection: &str, i18n: &I18n) -> Option<BulkAction> {
    match flow.step {
        BulkStep::Define => define_body(ui, flow, i18n),
        BulkStep::Preview => {
            preview_body(ui, flow, i18n);
            None
        }
        BulkStep::Confirm => confirm_body(ui, flow, connection, i18n),
        BulkStep::Running => {
            running_body(ui, flow, i18n);
            None
        }
        BulkStep::Report => {
            report_body(ui, flow, i18n);
            None
        }
    }
}

fn footer(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    ui.horizontal(|ui| match flow.step {
        BulkStep::Define => wizard_footer(ui, flow, i18n, false),
        BulkStep::Preview => wizard_footer(ui, flow, i18n, true),
        BulkStep::Confirm => confirm_footer(ui, flow, i18n),
        BulkStep::Running => single_footer(ui, flow, i18n, "common.cancel", BulkAction::Cancel),
        BulkStep::Report => single_footer(ui, flow, i18n, "common.close", BulkAction::Close),
    })
    .inner
}

/// Gives Cancel (or the only button) the first focus of the step it is drawn for.
fn focus_first(ui: &egui::Ui, first: &egui::Response, flow: &BulkFlow) {
    focus_once(ui, first, ("bulk_focus", &format!("{:?}", flow.step)));
}

/// Cancel, Back (from Preview on) and Next. Next waits for what the view-model says the step
/// needs: a valid update in Define, the count and the preview in Preview.
fn wizard_footer(
    ui: &mut egui::Ui,
    flow: &BulkFlow,
    i18n: &I18n,
    with_back: bool,
) -> Option<BulkAction> {
    let mut action = None;
    let cancel = text_button(ui, i18n.t("common.cancel"));
    focus_first(ui, &cancel, flow);
    if cancel.clicked() {
        action = Some(BulkAction::Close);
    }
    if with_back && text_button(ui, i18n.t("bulk.back")).clicked() {
        action = Some(BulkAction::Back);
    }
    let can_next = flow.clone().advanced().step != flow.step;
    let next = gate(ui, can_next, |ui| filled_button(ui, i18n.t("bulk.next")));
    if next.clicked() {
        action = Some(BulkAction::Next);
    }
    action
}

/// Cancel, Back and the destructive Run, which stays off until `can_run`.
fn confirm_footer(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let mut action = None;
    let cancel = text_button(ui, i18n.t("common.cancel"));
    focus_first(ui, &cancel, flow);
    if cancel.clicked() {
        action = Some(BulkAction::Close);
    }
    if text_button(ui, i18n.t("bulk.back")).clicked() {
        action = Some(BulkAction::Back);
    }
    let label = match flow.kind {
        BulkKind::Update => "bulk.run_update",
        BulkKind::Delete => "bulk.run_delete",
    };
    let run = gate(ui, flow.can_run(), |ui| filled_button(ui, i18n.t(label)));
    if run.clicked() {
        action = Some(BulkAction::Run);
    }
    action
}

fn single_footer(
    ui: &mut egui::Ui,
    flow: &BulkFlow,
    i18n: &I18n,
    label: &str,
    on_click: BulkAction,
) -> Option<BulkAction> {
    let button = text_button(ui, i18n.t(label));
    focus_first(ui, &button, flow);
    button.clicked().then_some(on_click)
}
