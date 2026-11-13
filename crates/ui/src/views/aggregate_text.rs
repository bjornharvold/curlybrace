//! Text mode of the Aggregate tab: the whole pipeline as one JSON array in a monospace editor,
//! with the reason it cannot go back to stage cards (line and column for a syntax error) under it,
//! and a warning while a stage is turned off (edited text comes back with every stage on).
use super::aggregate::{NO_DRAG_SCROLL, pipeline_error_text};
use crate::I18n;
use crate::app::AggregateAction;
use crate::viewmodel::aggregate::AggregateState;
use eframe::egui;

/// The editor grows with the text between these many lines.
const EDITOR_ROWS: std::ops::RangeInclusive<usize> = 6..=30;

/// The pipeline editor in a scroll area at most `max_height` tall; an `EditText` when typed in.
pub(super) fn show_text(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    max_height: f32,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let mut buffer = draft.text.clone();
    let rows = draft
        .text
        .lines()
        .count()
        .clamp(*EDITOR_ROWS.start(), *EDITOR_ROWS.end());
    let changed = egui::ScrollArea::vertical()
        .id_salt("aggregate_text")
        .scroll_source(NO_DRAG_SCROLL)
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut buffer)
                    .id_salt("aggregate_pipeline_text")
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY),
            );
            // Named on its accesskit node: `widget_info` would announce a one-line field.
            let name = i18n.t("aggregate.text.editor");
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label(name));
            response.changed()
        })
        .inner;
    match &draft.text_error {
        Some(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                pipeline_error_text(error, i18n),
            );
        }
        None => {
            ui.weak(i18n.t("aggregate.text.hint"));
        }
    }
    if draft.has_disabled() {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            i18n.t("aggregate.text.disabled_lost"),
        );
    }
    changed
        .then_some(AggregateAction::EditText(buffer))
        .into_iter()
        .collect()
}
