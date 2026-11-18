//! The index verdict as one fixed-height row above the documents: icon and title as one text in a tonal chip,
//! plus a "Why" button that opens the reasons, the suggested index (with a copy button) and the
//! estimate note in a popup under the row. The row is reserved even when there is no verdict
//! and the popup floats, so the documents never move.
use super::findbar::described;
use crate::I18n;
use crate::theme::{self, chip, compact_text_button, gate, tonal_button};
use crate::viewmodel::verdict::{BadgeModel, Tone, reason_text_key};
use eframe::egui;
use std::hash::{DefaultHasher, Hash, Hasher};

/// The narrowest the row gets: room for the longest English chip and the Why button.
const MIN_ROW_WIDTH: f32 = 320.0;

#[derive(Debug, Clone, PartialEq)]
pub enum VerdictAction {
    CopySuggestion(String),
    /// Create index... was pressed: open the index builder with the shown suggestion.
    CreateIndex,
}

impl VerdictAction {
    /// The command to put on the clipboard, for the copy action only (the guard notices offer no
    /// other).
    pub(super) fn copied(self) -> Option<String> {
        match self {
            Self::CopySuggestion(command) => Some(command),
            Self::CreateIndex => None,
        }
    }
}

/// Whether the suggestion offers Create index... and in which state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateIndex {
    /// No button (the guard notices: their context is "about to run", not the collection).
    Hidden,
    On,
    /// Shown off: the connection is read-only.
    ReadOnly,
    /// Shown off: an index write or prompt is open.
    Busy,
}

impl CreateIndex {
    /// Why the button is off (its `en.json` key); `None` while it works or is hidden.
    fn off_reason(self) -> Option<&'static str> {
        match self {
            Self::Hidden | Self::On => None,
            Self::ReadOnly => Some("query.verdict.create_index_read_only"),
            Self::Busy => Some("indexes.write_busy"),
        }
    }
}

/// The height of the verdict's row: tall enough for the chip and the Why button, so the documents
/// below never move when a verdict appears. A wrapping row that holds the verdict sets it as its
/// row height before its first widget, so the controls beside the verdict are centred on it.
pub fn verdict_row_height(ui: &egui::Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .max(theme::CHIP_HEIGHT)
        .max(theme::CONTROL_HEIGHT)
}

/// Whether the verdict's row fits on the current row of a wrapping layout beside `beside` points
/// of other controls (and the gap after them), judged from the row's width before anything is
/// drawn, so a row can be made tall enough for the verdict only when the verdict will sit on it.
pub fn fits_beside(ui: &egui::Ui, beside: f32) -> bool {
    ui.available_width() >= beside + ui.spacing().item_spacing.x + MIN_ROW_WIDTH
}

/// Draws the row for `model` (an empty row of the same size for `None`) in the rest of the
/// current row, or on the next line of a wrapping row when less than its minimum width is left.
pub fn show_verdict_row(
    ui: &mut egui::Ui,
    model: Option<&BadgeModel>,
    create: CreateIndex,
    i18n: &I18n,
) -> Option<VerdictAction> {
    let height = verdict_row_height(ui);
    // What is left of the current row (not the whole row's width, which a wrapping layout also
    // reports), at least a fixed width whatever the verdict says, so beside the checkboxes
    // whether the row wraps depends on the window, never on the verdict.
    let width = ui.available_size_before_wrap().x.max(MIN_ROW_WIDTH);
    let mut action = None;
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
        // The area occupies its whole width even when empty (an allocation only advances by what
        // is drawn), so what follows it does not move when a verdict appears.
        ui.set_min_size(egui::vec2(width, height));
        action = model.and_then(|model| show_row(ui, model, create, i18n));
    });
    action
}

fn show_row(
    ui: &mut egui::Ui,
    model: &BadgeModel,
    create: CreateIndex,
    i18n: &I18n,
) -> Option<VerdictAction> {
    let title = format!("{} {}", model.icon, i18n.t(model.title_key));
    chip(ui, chip_tone(model.tone), &title);
    if !has_details(model) {
        return None;
    }
    let toggle = compact_text_button(ui, i18n.t("query.verdict.why"));
    // Per verdict, so one verdict's open state does not carry over to an unrelated one.
    let open_id = ui.id().with(("verdict_why", salt(model)));
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let mut open = was_open != toggle.clicked();
    let mut action = None;
    if open {
        let popup = egui::Area::new(open_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(toggle.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| show_details(ui, model, create, i18n))
                    .inner
            });
        action = popup.inner;
        if popup.response.clicked_elsewhere() && !toggle.clicked() {
            open = false;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    action
}

fn has_details(model: &BadgeModel) -> bool {
    !model.reasons.is_empty() || model.plan_only || model.suggestion.is_some()
}

fn salt(model: &BadgeModel) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{model:?}").hash(&mut hasher);
    hasher.finish()
}

/// The theme tone a badge tone is drawn in. A check that is running or could not run says
/// nothing about the query, so it reads as neutral.
pub(super) fn chip_tone(tone: Tone) -> theme::Tone {
    match tone {
        Tone::Good => theme::Tone::Good,
        Tone::Warn => theme::Tone::Warning,
        Tone::Bad => theme::Tone::Bad,
        Tone::Neutral | Tone::Unavailable | Tone::Checking => theme::Tone::Neutral,
    }
}

fn show_details(
    ui: &mut egui::Ui,
    model: &BadgeModel,
    create: CreateIndex,
    i18n: &I18n,
) -> Option<VerdictAction> {
    ui.set_max_width(480.0);
    for reason in &model.reasons {
        ui.label(reason_line(reason.key, &reason.params, i18n));
    }
    if model.plan_only {
        ui.weak(i18n.t("query.verdict.plan_only"));
    }
    model
        .suggestion
        .as_deref()
        .and_then(|command| show_suggestion(ui, command, create, i18n))
}

/// A reason's text; one core emits that the badge has no string for still says something
/// visible (with its raw key) instead of vanishing.
pub(super) fn reason_line(key: &str, params: &[(&'static str, String)], i18n: &I18n) -> String {
    match reason_text_key(key) {
        Some(full) => {
            let params: Vec<(&str, &str)> = params.iter().map(|(n, v)| (*n, v.as_str())).collect();
            i18n.t_with(full, &params)
        }
        None => i18n.t_with("query.verdict.reason_unknown", &[("key", key)]),
    }
}

/// The suggested index with its Copy button and, unless `create` is hidden, Create index..., which
/// only opens the index builder.
pub(super) fn show_suggestion(
    ui: &mut egui::Ui,
    command: &str,
    create: CreateIndex,
    i18n: &I18n,
) -> Option<VerdictAction> {
    ui.label(i18n.t("query.verdict.suggestion"));
    ui.label(egui::RichText::new(command).monospace());
    ui.horizontal_wrapped(|ui| {
        let copy = compact_text_button(ui, i18n.t("query.verdict.copy")).clicked();
        let create_clicked = create != CreateIndex::Hidden && create_button(ui, create, i18n);
        match (copy, create_clicked) {
            (true, _) => Some(VerdictAction::CopySuggestion(command.to_string())),
            (false, true) => Some(VerdictAction::CreateIndex),
            (false, false) => None,
        }
    })
    .inner
}

/// Create index..., off with its reason as hover text and accessible description.
fn create_button(ui: &mut egui::Ui, create: CreateIndex, i18n: &I18n) -> bool {
    let off = create.off_reason();
    let response = gate(ui, off.is_none(), |ui| {
        tonal_button(ui, i18n.t("query.verdict.create_index"))
    });
    match off {
        Some(key) => {
            described(ui, response, i18n.t(key));
            false
        }
        None => response.clicked(),
    }
}

#[cfg(test)]
#[path = "verdict_badge_tests.rs"]
mod tests;
