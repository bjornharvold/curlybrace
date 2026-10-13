//! Fixed-height, fixed-width cells for the long lists (pair table, preflight rows, run overview),
//! so `ScrollArea::show_rows` can draw only the visible ones.
use crate::I18n;
use crate::viewmodel::compare::{EntryStatus, StatusIcon};
use eframe::egui;

/// Height of one table row; `show_rows` needs it fixed.
pub const ROW_HEIGHT: f32 = 24.0;
/// Most rows a list shows before it scrolls.
pub const MAX_ROWS_HEIGHT: f32 = 260.0;

/// One column of a table: a width of its own, or a share of what the rest leaves.
#[derive(Clone, Copy, Debug)]
pub enum Col {
    Fixed(f32),
    Flex(f32),
}

/// Narrowest a flexible column gets, however narrow the window.
const MIN_FLEX_WIDTH: f32 = 60.0;

/// Widths that make `cols` fill the width `ui` offers (less the gaps between cells and the
/// scrollbar of the row list), so no row runs past the window. Computed once per table and
/// shared by its header and rows, which keeps them aligned.
pub fn widths(ui: &egui::Ui, cols: &[Col]) -> Vec<f32> {
    let gaps = ui.spacing().item_spacing.x * cols.len().saturating_sub(1) as f32;
    let free = ui.available_width() - gaps - ui.spacing().scroll.allocated_width();
    let fixed: f32 = cols
        .iter()
        .map(|c| if let Col::Fixed(w) = c { *w } else { 0.0 })
        .sum();
    let weights: f32 = cols
        .iter()
        .map(|c| if let Col::Flex(w) = c { *w } else { 0.0 })
        .sum();
    let flexible = (free - fixed).max(0.0);
    cols.iter()
        .map(|c| match c {
            Col::Fixed(w) => *w,
            Col::Flex(w) => (flexible * w / weights).max(MIN_FLEX_WIDTH),
        })
        .collect()
}

/// A cell of exactly `width`; whatever `add` draws is clipped to it.
pub fn cell<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), layout, |ui| {
        ui.set_min_size(egui::vec2(width, ROW_HEIGHT));
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// A single-line label cut off with an ellipsis; the hover text carries the whole text.
pub fn truncated(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(text).truncate())
        .on_hover_text(text);
}

/// Icon and label in one label, so the status never relies on colour and reads as one phrase.
pub fn status_text(icon: StatusIcon, label: &str) -> String {
    format!("{} {label}", icon.glyph())
}

/// The label of a check verdict, with the side named where it matters.
pub fn entry_status_label(status: &EntryStatus, i18n: &I18n) -> String {
    let text = match status.side_key() {
        Some(side) => i18n.t_with(status.i18n_key(), &[("side", &i18n.t(side))]),
        None => i18n.t(status.i18n_key()),
    };
    status_text(status.icon(), &text)
}

/// Column header line; `widths` are those of the rows below it. A blank title leaves its column blank.
pub fn header(ui: &mut egui::Ui, widths: &[f32], titles: &[String]) {
    ui.horizontal(|ui| {
        for (width, title) in widths.iter().zip(titles) {
            cell(ui, *width, |ui| {
                ui.strong(title);
            });
        }
    });
}
