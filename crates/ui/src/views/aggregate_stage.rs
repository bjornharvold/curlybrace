//! The stage cards of the Aggregate tab. Each card: a header row (enable checkbox, the position
//! and stage name, which previews the stage when clicked, a "Previewed" marker on the selected
//! card, Move up, Move down, Remove), the stage's JSON in a monospace editor, and a one-line
//! status. Every control's accessible name carries the stage's position.
use super::aggregate::{NO_DRAG_SCROLL, named_button};
use super::aggregate_complete::completing_editor;
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme::shape::{self, radius};
use crate::theme::{CONTROL_HEIGHT, Tone, chip, current_ui};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_view::{AggregateView, StageStatus, stage_status};
use curlybrace_core::pipeline::Stage;
use eframe::egui::{self, WidgetInfo, WidgetType};

/// A stage editor grows with its text between these many lines.
const EDITOR_ROWS: std::ops::RangeInclusive<usize> = 2..=12;

/// One card to draw: the stage, its index and what the card needs to know about the list.
struct Card<'a> {
    index: usize,
    /// The stage's identity: what its editor's egui id is built from.
    key: u64,
    stage: &'a Stage,
    /// The stage whose output is previewed.
    selected: bool,
    last: usize,
}

/// The stage list in a scroll area at most `max_height` tall; the actions of the cards. Typing in
/// a stage goes to `view` (sent once it pauses), not to the actions.
pub(super) fn show_stages(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    view: &mut AggregateView,
    max_height: f32,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    if draft.stages.is_empty() {
        ui.label(i18n.t("aggregate.no_stages"));
        return Vec::new();
    }
    let now = ui.input(|i| i.time);
    let last = draft.stages.len() - 1;
    egui::ScrollArea::vertical()
        .id_salt("aggregate_stages")
        .scroll_source(NO_DRAG_SCROLL)
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut actions = Vec::new();
            for (index, stage) in draft.stages.iter().enumerate() {
                let card = Card {
                    index,
                    key: draft.keys.get(index).copied().unwrap_or(index as u64),
                    stage,
                    selected: index == draft.selected,
                    last,
                };
                let shown = view.shown_text(index, &stage.text).to_string();
                let (picked, typed) = show_card(ui, &card, &shown, i18n);
                actions.extend(picked);
                if let Some(text) = typed {
                    let (next, displaced) = std::mem::take(view).edited(index, text, now);
                    *view = next;
                    // The other stage's waiting edit goes out now; the new one is then taken at
                    // once too, as an action follows.
                    actions.extend(displaced.map(|(i, text)| AggregateAction::Edit(i, text)));
                }
            }
            actions
        })
        .inner
}

/// One card; returns its actions and the stage's new text when it was typed in.
fn show_card(
    ui: &mut egui::Ui,
    card: &Card<'_>,
    text: &str,
    i18n: &I18n,
) -> (Vec<AggregateAction>, Option<String>) {
    let status = stage_status(text);
    let n = (card.index + 1).to_string();
    card_frame(ui, card.selected)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let actions = card_header(ui, card, &status, &n, i18n);
            let typed = stage_editor(ui, card.key, text, &n, i18n);
            status_line(ui, card.stage.enabled, &status, i18n);
            (actions, typed)
        })
        .inner
}

/// The card's frame; the selected card has a thicker primary border (its "Previewed" marker says
/// the same in words).
fn card_frame(ui: &egui::Ui, selected: bool) -> egui::Frame {
    let s = current_ui(ui);
    let stroke = match selected {
        true => egui::Stroke::new(2.0, s.primary),
        false => egui::Stroke::new(1.0, s.outline_variant),
    };
    egui::Frame::new()
        .fill(s.surface_container_low)
        .stroke(stroke)
        .corner_radius(radius(shape::MEDIUM))
        .inner_margin(egui::Margin::same(shape::SPACE_2 as i8))
        .outer_margin(egui::Margin::symmetric(0, shape::SPACE_1 as i8))
}

/// Enable, the title (previews the stage when clicked), the marker, then Move up, Move down and
/// Remove, in one wrapping row.
fn card_header(
    ui: &mut egui::Ui,
    card: &Card<'_>,
    status: &StageStatus,
    n: &str,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let i = card.index;
    let params = [("n", n)];
    ui.horizontal_wrapped(|ui| {
        // One row height for every control, so the checkbox, the title and the 32 pt buttons
        // share a centre line instead of hanging from the top.
        ui.spacing_mut().interact_size.y = CONTROL_HEIGHT;
        let toggled = enable_box(
            ui,
            card.stage.enabled,
            i18n.t_with("aggregate.stage.enable_n", &params),
        );
        let picked = title(ui, card, status, n, i18n);
        if card.selected {
            chip(ui, Tone::Good, &i18n.t("aggregate.stage.previewed"));
        }
        let up = i18n.t("aggregate.stage.move_up");
        let moved_up = named_button(
            ui,
            i > 0,
            (&up, i18n.t_with("aggregate.stage.move_up_n", &params)),
        );
        let down = i18n.t("aggregate.stage.move_down");
        let moved_down = named_button(
            ui,
            i < card.last,
            (&down, i18n.t_with("aggregate.stage.move_down_n", &params)),
        );
        let remove = i18n.t("aggregate.stage.remove");
        let removed = named_button(
            ui,
            true,
            (&remove, i18n.t_with("aggregate.stage.remove_n", &params)),
        );
        [
            toggled.then_some(AggregateAction::Toggle(i)),
            picked.then_some(AggregateAction::Select(i)),
            moved_up.then_some(AggregateAction::MoveUp(i)),
            moved_down.then_some(AggregateAction::MoveDown(i)),
            removed.then_some(AggregateAction::Remove(i)),
        ]
        .into_iter()
        .flatten()
        .collect()
    })
    .inner
}

/// The enable checkbox, named `name`; whether it was toggled.
fn enable_box(ui: &mut egui::Ui, enabled: bool, name: String) -> bool {
    let mut on = enabled;
    let response = ui.checkbox(&mut on, "");
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, name.clone()));
    response.changed()
}

/// "2. $match" (or the invalid marker for a stage that does not parse) as a selectable label;
/// whether it was clicked on a card that is not the selected one.
fn title(ui: &mut egui::Ui, card: &Card<'_>, status: &StageStatus, n: &str, i18n: &I18n) -> bool {
    let name = match status {
        StageStatus::Valid(name) => name.clone(),
        _ => i18n.t("aggregate.stage.invalid"),
    };
    let text = i18n.t_with("aggregate.stage.title", &[("n", n), ("name", &name)]);
    let response = ui.selectable_label(card.selected, egui::RichText::new(text).strong());
    let accessible = i18n.t_with("aggregate.stage.select_n", &[("n", n)]);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            card.selected,
            accessible.clone(),
        )
    });
    response.clicked() && !card.selected
}

/// The stage's JSON in a monospace editor named after its position, with stage-name completion;
/// the new text when typed in.
fn stage_editor(ui: &mut egui::Ui, key: u64, text: &str, n: &str, i18n: &I18n) -> Option<String> {
    let mut buffer = text.to_string();
    let rows = text
        .lines()
        .count()
        .clamp(*EDITOR_ROWS.start(), *EDITOR_ROWS.end());
    let id = stage_text_id(key);
    let (response, changed) = completing_editor(ui, id, &mut buffer, rows, i18n);
    // Named on its accesskit node, not through `widget_info`, which would announce the
    // multi-line editor as a one-line text field.
    let name = i18n.t_with("aggregate.stage.editor_n", &[("n", n)]);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
    changed.then_some(buffer)
}

/// The egui id of the stage editor of identity `key`; it follows the stage when it moves.
pub(super) fn stage_text_id(key: u64) -> egui::Id {
    egui::Id::new(("aggregate_stage_text", key))
}

/// Valid, or what is wrong; a disabled stage also says the preview skips it.
fn status_line(ui: &mut egui::Ui, enabled: bool, status: &StageStatus, i18n: &I18n) {
    ui.horizontal_wrapped(|ui| {
        match status {
            StageStatus::Valid(name) => {
                ui.weak(i18n.t_with("aggregate.stage.valid", &[("name", name)]));
            }
            StageStatus::Syntax {
                line,
                column,
                message,
            } => {
                let (line, column) = (line.to_string(), column.to_string());
                let params = [
                    ("line", line.as_str()),
                    ("column", &column),
                    ("message", message),
                ];
                error_label(ui, i18n.t_with("aggregate.error.syntax", &params));
            }
            StageStatus::NotAStage => error_label(ui, i18n.t("aggregate.stage.not_a_stage")),
        }
        if !enabled {
            ui.weak(i18n.t("aggregate.stage.disabled"));
        }
    });
}

fn error_label(ui: &mut egui::Ui, text: String) {
    ui.colored_label(ui.visuals().error_fg_color, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_id_follows_the_stage_when_it_moves() {
        let draft = ["$match", "$sort"]
            .into_iter()
            .fold(AggregateState::default(), |d, n| d.add_stage(n));
        let before: Vec<_> = draft.keys.iter().map(|k| stage_text_id(*k)).collect();
        let moved = draft.move_up(1);
        let after: Vec<_> = moved.keys.iter().map(|k| stage_text_id(*k)).collect();
        assert_eq!(after, [before[1], before[0]]);
        assert_ne!(before[0], before[1]);
    }
}
