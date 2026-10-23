//! The review modal: what will be written where, the warnings, the id policy and, for deletes,
//! the typed confirmation. It only reports actions; whether Apply may fire is decided by the
//! same check the reducer repeats (`Review::confirmation_passes`).
use super::pending::{destination_text, pair_text};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::compare::{CompareState, Review, ReviewPair};
use crate::views::connection_badge::show_badge;
use crate::views::write_warning::focus_cancel_unless_focused;
use curlybrace_core::compare::apply::IdPolicy;
use eframe::egui;
use uuid::Uuid;

pub use super::apply_result::show_progress;

const MODAL_WIDTH: f32 = 600.0;
const TABLE_MAX_HEIGHT: f32 = 280.0;
const DESTINATION_WIDTH: f32 = 200.0;

/// The dialog, while a review is open. Escape or a click outside it cancels the review.
pub fn show_review(
    ctx: &egui::Context,
    state: &CompareState,
    production: &dyn Fn(Uuid) -> bool,
    read_only: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let review = state.apply.review()?;
    let notice = review.production_destinations(production);
    let modal = egui::Modal::new(egui::Id::new("compare_apply_review"));
    let response = modal.show(ctx, |ui| {
        show_body(ui, state, review, &notice, read_only, i18n)
    });
    let closed = response.should_close();
    response
        .inner
        .or(closed.then_some(CompareAction::CancelReview))
}

fn show_body(
    ui: &mut egui::Ui,
    state: &CompareState,
    review: &Review,
    production: &[String],
    read_only: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.set_width(MODAL_WIDTH);
    show_production_heading(ui, !production.is_empty(), i18n);
    ui.heading(i18n.t("compare.apply.review.title"));
    show_pairs(ui, state, review, i18n);
    let total = review.op_total().to_string();
    ui.strong(i18n.t_with("compare.apply.review.total", &[("count", &total)]));
    show_field_line(ui, review, i18n);
    let policy = show_id_policy(ui, review, i18n);
    show_production_notice(ui, production, i18n);
    show_warnings(ui, review, i18n);
    let (typed, typing) = show_confirmation(ui, review, i18n);
    let buttons = show_buttons(ui, review, !production.is_empty(), typing, read_only, i18n);
    buttons.or(typed).or(policy)
}

/// The production warning heading with the production badge, like the write warning's, above
/// the review when a written connection is production.
fn show_production_heading(ui: &mut egui::Ui, production: bool, i18n: &I18n) {
    if !production {
        return;
    }
    ui.horizontal(|ui| {
        show_badge(ui, &ConnectionBadge::new(None, true), i18n);
        ui.heading(i18n.t("write_warning.title"));
    });
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
    let documents = review.field_documents();
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

/// One line per written connection that is production, so the person sees where the writes land.
fn show_production_notice(ui: &mut egui::Ui, production: &[String], i18n: &I18n) {
    if production.is_empty() {
        return;
    }
    ui.separator();
    let color = ui.visuals().error_fg_color;
    for name in production {
        let line = i18n.t_with(
            "compare.apply.review.production_line",
            &[("connection", name)],
        );
        ui.label(egui::RichText::new(line).strong().color(color));
    }
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
/// Also reports whether the box holds the keyboard focus (the person is typing).
fn show_confirmation(
    ui: &mut egui::Ui,
    review: &Review,
    i18n: &I18n,
) -> (Option<CompareAction>, bool) {
    let Some(required) = review.required_text(&review.word) else {
        return (None, false);
    };
    ui.separator();
    let caption = ui.label(i18n.t("compare.apply.review.confirm_prompt"));
    ui.label(egui::RichText::new(required).strong().monospace());
    let mut text = review.confirm_text.clone();
    let edit = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
    let response = ui.add(edit).labelled_by(caption.id);
    let typing = response.has_focus();
    (
        response
            .changed()
            .then_some(CompareAction::SetConfirmText(text)),
        typing,
    )
}

/// Apply (only when the confirmation passes) and Cancel. A plain review takes no focus on its
/// own and Enter confirms nothing. With a production destination Cancel comes first and holds
/// the initial focus (as in the write warning), so Enter can never apply; it leaves the focus
/// alone while the person types the confirmation.
fn show_buttons(
    ui: &mut egui::Ui,
    review: &Review,
    production: bool,
    typing: bool,
    read_only: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.separator();
    if read_only {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            i18n.t("compare.apply.reason.read_only_target"),
        );
    }
    let may_apply = review.confirmation_passes() && !read_only;
    ui.horizontal(|ui| {
        let apply = egui::Button::new(i18n.t("compare.apply.review.apply"));
        let cancel_label = i18n.t("compare.apply.review.cancel");
        if !production {
            let confirm = ui
                .add_enabled(may_apply, apply)
                .clicked()
                .then_some(CompareAction::ConfirmApply);
            let cancel = ui.button(cancel_label);
            return confirm.or(cancel.clicked().then_some(CompareAction::CancelReview));
        }
        let cancel = ui.button(cancel_label);
        let apply = ui.add_enabled(may_apply, apply);
        if !typing {
            focus_cancel_unless_focused(&cancel, &apply);
        }
        apply
            .clicked()
            .then_some(CompareAction::ConfirmApply)
            .or(cancel.clicked().then_some(CompareAction::CancelReview))
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn notice_lines(production: Vec<String>) -> Vec<String> {
        let mut harness = Harness::new_ui_state(
            |ui, names: &mut Vec<String>| show_production_notice(ui, names, &I18n::english()),
            production.clone(),
        );
        harness.run_steps(2);
        production
            .iter()
            .map(|name| {
                I18n::english().t_with(
                    "compare.apply.review.production_line",
                    &[("connection", name)],
                )
            })
            .filter(|line| harness.query_by_label(line).is_some())
            .collect()
    }

    #[test]
    fn notice_names_each_production_destination() {
        let names = vec!["prod-a".to_string(), "prod-b".to_string()];
        assert_eq!(notice_lines(names).len(), 2);
    }

    #[test]
    fn notice_is_absent_without_production_destinations() {
        let mut harness = Harness::new_ui_state(
            |ui, names: &mut Vec<String>| show_production_notice(ui, names, &I18n::english()),
            Vec::<String>::new(),
        );
        harness.run_steps(2);
        assert!(
            harness
                .query_by_label_contains("production connection")
                .is_none()
        );
    }

    #[test]
    fn notice_key_takes_the_connection_name() {
        let line = I18n::english().t_with(
            "compare.apply.review.production_line",
            &[("connection", "orders-prod")],
        );
        assert!(line.contains("orders-prod"), "{line}");
    }
}
