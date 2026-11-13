//! The preview pane of the Aggregate tab: a heading row (Tree | Table | JSON, which stage it
//! shows, how many documents), the status lines (running, stages not run, why it failed), then
//! the documents in the Documents tab's own views. A preview is read-only: its documents are a
//! pipeline's output, not rows of the collection, so every write entry is off and Copy (which
//! copies by `_id` from the collection) is blocked.
use super::aggregate::pipeline_error_text;
use super::documents::show_mode_switcher;
use super::error_label::show_error;
use super::{json::show_json, table::show_table, tree::show_tree};
use crate::I18n;
use crate::theme::CONTROL_HEIGHT;
use crate::viewmodel::ViewMode;
use crate::viewmodel::aggregate::{AggregateState, Preview, PreviewError, PreviewState};
use crate::viewmodel::aggregate_view::{AggregateView, shown_preview};
use bson::Document;
use curlybrace_core::pipeline::PREVIEW_LIMIT;
use eframe::egui;

/// Draws the pane for `draft`'s preview; the view's tree and mode are the pane's own.
pub(super) fn show_preview(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    view: &mut AggregateView,
    i18n: &I18n,
) {
    let shown = shown_preview(&draft.preview);
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        view.mode = show_mode_switcher(ui, view.mode, i18n);
        if !draft.stages.is_empty() {
            let n = (draft.selected + 1).to_string();
            ui.strong(i18n.t_with("aggregate.preview.heading", &[("n", &n)]));
        }
        if let Some(preview) = shown.filter(|p| !p.docs.is_empty()) {
            ui.label(count_text(preview.docs.len(), i18n));
        }
    });
    status_lines(ui, draft, i18n);
    match shown {
        Some(preview) if !preview.docs.is_empty() => show_docs(ui, &preview.docs, view, i18n),
        Some(_) => {
            ui.label(i18n.t("aggregate.preview.empty"));
        }
        None => {}
    }
}

/// How many documents are shown; at the limit it says only the first ones are.
fn count_text(count: usize, i18n: &I18n) -> String {
    let n = count.to_string();
    match count {
        _ if i64::try_from(count).is_ok_and(|c| c >= PREVIEW_LIMIT) => i18n.t_with(
            "aggregate.preview.capped",
            &[("limit", &PREVIEW_LIMIT.to_string())],
        ),
        1 => i18n.t("aggregate.preview.count_one"),
        _ => i18n.t_with("aggregate.preview.count_many", &[("n", &n)]),
    }
}

/// What the pane says about the preview besides its documents.
fn status_lines(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) {
    match &draft.preview {
        PreviewState::Idle if draft.stages.is_empty() => {}
        PreviewState::Idle => {
            ui.label(i18n.t("aggregate.preview.idle"));
        }
        PreviewState::Loading { not_run, last } => {
            ui.label(i18n.t("aggregate.preview.loading"));
            not_run_line(ui, not_run, i18n);
            kept_line(ui, last.as_ref(), i18n);
        }
        PreviewState::Shown(preview) => not_run_line(ui, &preview.not_run, i18n),
        PreviewState::Failed { error, last } => {
            match error {
                PreviewError::Pipeline(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, pipeline_error_text(e, i18n));
                }
                PreviewError::Core(e) => show_error(ui, e, i18n),
            }
            kept_line(ui, last.as_ref(), i18n);
        }
    }
}

/// "N stages not run: $out" for the trailing write stages the preview left out.
fn not_run_line(ui: &mut egui::Ui, not_run: &[String], i18n: &I18n) {
    let names = not_run.join(", ");
    let text = match not_run.len() {
        0 => return,
        1 => i18n.t_with("aggregate.preview.not_run_one", &[("names", &names)]),
        count => i18n.t_with(
            "aggregate.preview.not_run_many",
            &[("count", &count.to_string()), ("names", &names)],
        ),
    };
    ui.label(text);
}

/// Says the documents below are the last preview that ran, while there are some.
fn kept_line(ui: &mut egui::Ui, last: Option<&Preview>, i18n: &I18n) {
    if last.is_some() {
        ui.weak(i18n.t("aggregate.preview.kept"));
    }
}

/// The documents in the chosen view, read-only. The views' answers are not used: with every write
/// off and Copy blocked, all they can report is a menu entry that does nothing here.
fn show_docs(ui: &mut egui::Ui, docs: &[Document], view: &mut AggregateView, i18n: &I18n) {
    let AggregateView {
        tree, mode, page, ..
    } = view;
    let (page, key) = page.of(docs);
    tree.synced_to(&key);
    tree.read_only = true;
    tree.projected = true;
    tree.copy_blocked = true;
    let _offers_nothing = match mode {
        ViewMode::Tree => show_tree(ui, page, &key, tree, i18n),
        ViewMode::Table => show_table(ui, page, &key, tree, i18n),
        ViewMode::Json => show_json(ui, page, &key, tree, i18n),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_says_when_only_the_first_documents_are_shown() {
        let i18n = I18n::english();
        let limit = usize::try_from(PREVIEW_LIMIT).unwrap();
        assert_eq!(count_text(1, &i18n), i18n.t("aggregate.preview.count_one"));
        assert!(count_text(3, &i18n).contains('3'));
        assert!(count_text(limit, &i18n).contains(&PREVIEW_LIMIT.to_string()));
        assert_ne!(count_text(limit, &i18n), count_text(limit - 1, &i18n));
    }
}
