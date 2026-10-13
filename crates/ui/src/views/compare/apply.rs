//! The review modal: what will be written where, the warnings, the id policy and, for deletes,
//! the typed confirmation. It only reports actions; whether Apply may fire is decided by the
//! same check the reducer repeats (`Review::confirmation_passes`).
use super::pending::{destination_text, pair_text};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, Review, ReviewPair};
use curlybrace_core::compare::apply::IdPolicy;
use eframe::egui;

pub use super::apply_result::show_progress;

const MODAL_WIDTH: f32 = 600.0;
const TABLE_MAX_HEIGHT: f32 = 280.0;
const DESTINATION_WIDTH: f32 = 200.0;

/// The dialog, while a review is open. Escape or a click outside it cancels the review.
pub fn show_review(
    ctx: &egui::Context,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let review = state.apply.review()?;
    let modal = egui::Modal::new(egui::Id::new("compare_apply_review"));
    let response = modal.show(ctx, |ui| show_body(ui, state, review, i18n));
    let closed = response.should_close();
    response
        .inner
        .or(closed.then_some(CompareAction::CancelReview))
}

fn show_body(
    ui: &mut egui::Ui,
    state: &CompareState,
    review: &Review,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.set_width(MODAL_WIDTH);
    ui.heading(i18n.t("compare.apply.review.title"));
    show_pairs(ui, state, review, i18n);
    let total = review.op_total().to_string();
    ui.strong(i18n.t_with("compare.apply.review.total", &[("count", &total)]));
    show_field_line(ui, review, i18n);
    let policy = show_id_policy(ui, review, i18n);
    show_warnings(ui, review, i18n);
    let typed = show_confirmation(ui, review, i18n);
    let buttons = show_buttons(ui, review, i18n);
    buttons.or(typed).or(policy)
}

fn show_pairs(ui: &mut egui::Ui, state: &CompareState, review: &Review, i18n: &I18n) {
    egui::ScrollArea::vertical()
        .id_salt("compare_review_pairs")
        .max_height(TABLE_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("compare_review_grid")
                .striped(true)
                .num_columns(6)
                .show(ui, |ui| {
                    for title in [
                        "compare.apply.review.column_pair",
                        "compare.apply.kind.insert",
                        "compare.apply.kind.replace",
                        "compare.apply.kind.delete",
                        "compare.apply.kind.fields",
                        "compare.apply.review.column_writes",
                    ] {
                        ui.strong(i18n.t(title));
                    }
                    ui.end_row();
                    for pair in &review.pairs {
                        show_pair_row(ui, state, pair, i18n);
                    }
                });
        });
}

fn show_pair_row(ui: &mut egui::Ui, state: &CompareState, pair: &ReviewPair, i18n: &I18n) {
    ui.label(pair_text(state, pair.pair, i18n));
    count_cell(
        ui,
        "compare.apply.kind.insert",
        pair.counts.insert,
        false,
        i18n,
    );
    count_cell(
        ui,
        "compare.apply.kind.replace",
        pair.counts.replace,
        false,
        i18n,
    );
    count_cell(
        ui,
        "compare.apply.kind.delete",
        pair.counts.delete,
        true,
        i18n,
    );
    count_cell(
        ui,
        "compare.apply.kind.fields",
        pair.counts.fields,
        false,
        i18n,
    );
    ui.vertical(|ui| {
        ui.set_max_width(DESTINATION_WIDTH);
        for (action, destination) in &pair.destinations {
            let line = i18n.t_with(
                "compare.apply.review.destination_line",
                &[
                    ("action", &i18n.t(action.i18n_key())),
                    ("destination", &destination_text(destination, i18n)),
                ],
            );
            ui.label(line);
        }
    });
    ui.end_row();
}

/// A count, named for screen readers by its kind. Deletes stand out when there are any.
fn count_cell(ui: &mut egui::Ui, kind_key: &str, count: u64, danger: bool, i18n: &I18n) {
    let mut text = egui::RichText::new(count.to_string());
    if danger && count > 0 {
        text = text.strong().color(ui.visuals().error_fg_color);
    }
    let name = i18n.t_with(
        "compare.apply.review.count_cell",
        &[("kind", &i18n.t(kind_key)), ("count", &count.to_string())],
    );
    let label = ui.label(text);
    label.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name.clone()));
}

/// "Fields: N, documents: M" when the review copies single fields.
fn show_field_line(ui: &mut egui::Ui, review: &Review, i18n: &I18n) {
    let documents: u64 = review.pairs.iter().map(|p| p.counts.fields).sum();
    if documents == 0 {
        return;
    }
    let (fields, documents) = (review.field_total().to_string(), documents.to_string());
    ui.label(i18n.t_with(
        "compare.apply.review.fields_line",
        &[("fields", &fields), ("documents", &documents)],
    ));
}

/// The toggle for the identifiers of inserted documents; only when something is inserted.
fn show_id_policy(ui: &mut egui::Ui, review: &Review, i18n: &I18n) -> Option<CompareAction> {
    if !review.pairs.iter().any(|p| p.counts.insert > 0) {
        return None;
    }
    ui.separator();
    ui.strong(i18n.t("compare.apply.id_policy.label"));
    let mut action = None;
    for (policy, key) in [
        (IdPolicy::Keep, "compare.apply.id_policy.keep"),
        (IdPolicy::Generate, "compare.apply.id_policy.generate"),
    ] {
        let chosen = review.id_policy == policy;
        if ui.radio(chosen, i18n.t(key)).clicked() && !chosen {
            action = Some(CompareAction::SetIdPolicy(policy));
        }
    }
    action
}

fn show_warnings(ui: &mut egui::Ui, review: &Review, i18n: &I18n) {
    if review.warnings.is_empty() {
        return;
    }
    ui.separator();
    ui.strong(i18n.t("compare.apply.review.warnings_title"));
    let color = ui.visuals().warn_fg_color;
    for warning in &review.warnings {
        ui.colored_label(color, format!("⚠ {}", i18n.t(warning.i18n_key())));
    }
}

/// The typed confirmation, when deletes need one: the prompt on its own wrapping line, the text
/// to type below it (bold, monospace) and a full-width box. The box shows what the review stores.
fn show_confirmation(ui: &mut egui::Ui, review: &Review, i18n: &I18n) -> Option<CompareAction> {
    let required = review.required_text(&review.word)?;
    ui.separator();
    let caption = ui.label(i18n.t("compare.apply.review.confirm_prompt"));
    ui.label(egui::RichText::new(required).strong().monospace());
    let mut text = review.confirm_text.clone();
    let edit = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
    let changed = ui.add(edit).labelled_by(caption.id).changed();
    changed.then_some(CompareAction::SetConfirmText(text))
}

/// Apply (only when the confirmation passes) and Cancel. Neither takes focus on its own and
/// Enter confirms nothing.
fn show_buttons(ui: &mut egui::Ui, review: &Review, i18n: &I18n) -> Option<CompareAction> {
    ui.separator();
    ui.horizontal(|ui| {
        let apply = egui::Button::new(i18n.t("compare.apply.review.apply"));
        let confirm = ui
            .add_enabled(review.confirmation_passes(), apply)
            .clicked()
            .then_some(CompareAction::ConfirmApply);
        let cancel = ui.button(i18n.t("compare.apply.review.cancel"));
        confirm.or(cancel.clicked().then_some(CompareAction::CancelReview))
    })
    .inner
}
