//! Export: the two entry buttons (with the reason when one is closed) and the options modal for
//! plans and reports. The modal is one `egui::Modal` (`compare_export`) for every state of the
//! export; the waiting, running and result states are in `export_result.rs`. The views only
//! report actions: the file dialog is opened by the app layer from the `Choosing` state, never
//! from here, and nothing here starts a write.
use super::export_result;
use super::pending::pair_text;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    CompareState, ExportBlock, ExportDraft, ExportKind, ExportUi, PendingView,
};
use curlybrace_core::compare::apply::{ApplyKind, IdPolicy};
use curlybrace_core::compare::protocol::ExportFormat;
use eframe::egui;

const MODAL_WIDTH: f32 = 560.0;
/// Most characters the label box takes (core sanitises what is typed again).
pub(super) const LABEL_MAX_CHARS: usize = 200;

/// The entry button of `kind` and, when it is closed, the reason next to it.
pub fn show_entry(
    ui: &mut egui::Ui,
    state: &CompareState,
    kind: ExportKind,
    i18n: &I18n,
) -> Option<CompareAction> {
    let block = state.export_block(kind);
    let key = match kind {
        ExportKind::Plan => "compare.export.open_plan",
        ExportKind::Report => "compare.export.open_report",
    };
    ui.horizontal(|ui| {
        let button = egui::Button::new(i18n.t(key));
        let clicked = ui.add_enabled(block.is_none(), button).clicked();
        if let Some(reason) = block {
            let color = ui.visuals().warn_fg_color;
            ui.colored_label(color, i18n.t(reason.i18n_key()));
        }
        clicked.then_some(CompareAction::OpenExport(kind))
    })
    .inner
}

/// The modal, while an export is open. Escape or a click outside closes it only while the options
/// are shown: waiting for the dialog and a running job are never closed from here.
pub fn show_export(
    ctx: &egui::Context,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    if state.export == ExportUi::Idle {
        return None;
    }
    let modal = egui::Modal::new(egui::Id::new("compare_export"));
    let response = modal.show(ctx, |ui| show_body(ui, state, i18n));
    let closes = response.should_close() && state.export.configuring().is_some();
    response
        .inner
        .or(closes.then_some(CompareAction::CloseExport))
}

fn show_body(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    ui.set_width(MODAL_WIDTH);
    match &state.export {
        ExportUi::Configuring(draft) => show_options(ui, state, draft, i18n),
        ExportUi::Choosing(draft) => {
            export_result::show_choosing(ui, draft.kind, i18n);
            None
        }
        other => export_result::show(ui, other, i18n),
    }
}

pub(super) fn title_key(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::Plan => "compare.export.title.plan",
        ExportKind::Report => "compare.export.title.report",
    }
}

fn show_options(
    ui: &mut egui::Ui,
    state: &CompareState,
    draft: &ExportDraft,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.heading(i18n.t(title_key(draft.kind)));
    let options = match draft.kind {
        ExportKind::Plan => show_plan_options(ui, state, draft, i18n),
        ExportKind::Report => show_report_options(ui, state, draft, i18n),
    };
    ui.add_space(4.0);
    let label = show_label(ui, draft, i18n);
    show_warnings(ui, draft.kind, i18n);
    let buttons = show_buttons(ui, state, draft, i18n);
    buttons.or(label).or(options)
}

fn show_plan_options(
    ui: &mut egui::Ui,
    state: &CompareState,
    draft: &ExportDraft,
    i18n: &I18n,
) -> Option<CompareAction> {
    let view = state.pending_view(&|_| String::new());
    let total = view.documents().to_string();
    ui.label(i18n.t_with("compare.apply.pending.total", &[("count", &total)]));
    let format = show_format(ui, state, draft, i18n);
    let policy = has_inserts(&view)
        .then(|| show_id_policy(ui, draft, i18n))
        .flatten();
    policy.or(format)
}

fn has_inserts(view: &PendingView) -> bool {
    view.groups
        .iter()
        .flat_map(|g| g.lines.iter())
        .any(|l| l.kind == ApplyKind::Insert)
}

/// The two formats. The script is closed (with the reason shown) when the queued writes land on
/// both sides.
fn show_format(
    ui: &mut egui::Ui,
    state: &CompareState,
    draft: &ExportDraft,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.separator();
    ui.strong(i18n.t("compare.export.format.label"));
    let mixed = state.pending_has_mixed_sides();
    let mut action = None;
    for (format, key, open) in [
        (
            ExportFormat::Mongosh,
            "compare.export.format.mongosh",
            !mixed,
        ),
        (ExportFormat::JsonPlan, "compare.export.format.json", true),
    ] {
        let chosen = draft.plan.format == format;
        let radio = egui::RadioButton::new(chosen, i18n.t(key));
        if ui.add_enabled(open, radio).clicked() && !chosen {
            action = Some(CompareAction::SetExportFormat(format));
        }
    }
    if mixed {
        let color = ui.visuals().warn_fg_color;
        ui.colored_label(color, i18n.t(ExportBlock::MixedSides.i18n_key()));
    }
    action
}

fn show_id_policy(ui: &mut egui::Ui, draft: &ExportDraft, i18n: &I18n) -> Option<CompareAction> {
    ui.separator();
    ui.strong(i18n.t("compare.apply.id_policy.label"));
    let mut action = None;
    for (policy, key) in [
        (IdPolicy::Keep, "compare.apply.id_policy.keep"),
        (IdPolicy::Generate, "compare.apply.id_policy.generate"),
    ] {
        let chosen = draft.plan.id_policy == policy;
        if ui.radio(chosen, i18n.t(key)).clicked() && !chosen {
            action = Some(CompareAction::SetExportIdPolicy(policy));
        }
    }
    action
}

/// A checkbox per pair with finished results (named by the pair) and the paths option.
fn show_report_options(
    ui: &mut egui::Ui,
    state: &CompareState,
    draft: &ExportDraft,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.separator();
    ui.strong(i18n.t("compare.export.pairs_title"));
    let mut action = None;
    for (entry, pair) in state.completed_entries() {
        let mut ticked = draft.report.pairs.contains(&entry);
        let name = pair_text(state, pair, i18n);
        let changed = ui
            .push_id(entry, |ui| ui.checkbox(&mut ticked, name).changed())
            .inner;
        if changed {
            action = Some(CompareAction::ToggleExportPair(entry));
        }
    }
    ui.separator();
    let mut paths = draft.report.include_paths;
    let text = i18n.t("compare.export.include_paths");
    if ui.checkbox(&mut paths, text).changed() {
        action = Some(CompareAction::SetExportIncludePaths(paths));
    }
    ui.label(egui::RichText::new(i18n.t("compare.export.include_paths_note")).weak());
    action
}

/// The single-line label box. It reports an edit only: Enter in it does nothing.
fn show_label(ui: &mut egui::Ui, draft: &ExportDraft, i18n: &I18n) -> Option<CompareAction> {
    ui.separator();
    let caption = ui.label(i18n.t("compare.export.label.caption"));
    let mut text = match draft.kind {
        ExportKind::Plan => draft.plan.label.clone(),
        ExportKind::Report => draft.report.label.clone(),
    };
    let edit = egui::TextEdit::singleline(&mut text)
        .char_limit(LABEL_MAX_CHARS)
        .desired_width(f32::INFINITY);
    let changed = ui.add(edit).labelled_by(caption.id).changed();
    changed.then_some(CompareAction::SetExportLabel(text))
}

fn show_warnings(ui: &mut egui::Ui, kind: ExportKind, i18n: &I18n) {
    ui.separator();
    ui.strong(i18n.t("compare.export.warnings_title"));
    let keys: &[&str] = match kind {
        ExportKind::Plan => &[
            "compare.export.warning.data",
            "compare.export.warning.stale",
            "compare.export.warning.skipped",
        ],
        ExportKind::Report => &["compare.export.warning.report_keys"],
    };
    let color = ui.visuals().warn_fg_color;
    for key in keys {
        ui.colored_label(color, i18n.t(key));
    }
}

/// Choose file (and Copy for a report) and Cancel. A closed output shows the reason below once;
/// the script's own reason is already shown next to the format.
fn show_buttons(
    ui: &mut egui::Ui,
    state: &CompareState,
    draft: &ExportDraft,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.separator();
    let file = state.file_block(draft);
    let copy = state.copy_block(draft);
    let mut action = None;
    ui.horizontal(|ui| {
        let choose = egui::Button::new(i18n.t("compare.export.choose_file"));
        if ui.add_enabled(file.is_none(), choose).clicked() {
            action = Some(CompareAction::ChooseExportFile);
        }
        if draft.kind == ExportKind::Report {
            let button = egui::Button::new(i18n.t("compare.export.copy"));
            if ui.add_enabled(copy.is_none(), button).clicked() {
                action = Some(CompareAction::CopyReport);
            }
        }
        if ui.button(i18n.t("compare.export.cancel")).clicked() {
            action = Some(CompareAction::CloseExport);
        }
    });
    let reason = file.or(match draft.kind {
        ExportKind::Report => copy,
        ExportKind::Plan => None,
    });
    if let Some(reason) = reason.filter(|r| *r != ExportBlock::MixedSides) {
        let color = ui.visuals().warn_fg_color;
        ui.colored_label(color, i18n.t(reason.i18n_key()));
    }
    action
}
