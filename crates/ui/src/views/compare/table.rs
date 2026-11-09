//! Fixed-height, fixed-width cells for the long lists (pair table, preflight rows, run overview),
//! so `ScrollArea::show_rows` can draw only the visible ones.
use crate::I18n;
use crate::theme::{CONTROL_HEIGHT, Tone, card, chip, current_ui, shape};
use crate::viewmodel::compare::{EntryStatus, StatusIcon};
use eframe::egui;

/// Height of one table row; `show_rows` needs it fixed. A theme button (40 dp) or an outlined field
/// (40 dp plus its floating label's overhang) fits in it.
pub const ROW_HEIGHT: f32 = CONTROL_HEIGHT + shape::SPACE_2;
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

/// `add` in a full-width card: the container of one block of related content.
pub fn panel<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
    .inner
}

/// The tone a status icon is shown in: done is good, a problem is bad, the rest is neutral.
pub fn tone_of(icon: StatusIcon) -> Tone {
    match icon {
        StatusIcon::Ok => Tone::Good,
        StatusIcon::Warning => Tone::Warning,
        StatusIcon::Failed => Tone::Bad,
        StatusIcon::Pending
        | StatusIcon::Running
        | StatusIcon::Cancelled
        | StatusIcon::Discarded
        | StatusIcon::Unchecked
        | StatusIcon::Customised => Tone::Neutral,
    }
}

/// A state as a chip: the icon and the label in one text (so the state never relies on colour),
/// on the tone of the icon. Returns the chip's response.
pub fn status_chip(ui: &mut egui::Ui, icon: StatusIcon, label: &str) -> egui::Response {
    chip(ui, tone_of(icon), &status_text(icon, label))
}

/// A pair's check verdict as a chip, the side named where it matters.
pub fn entry_status_chip(ui: &mut egui::Ui, status: &EntryStatus, i18n: &I18n) -> egui::Response {
    chip(
        ui,
        tone_of(status.icon()),
        &entry_status_label(status, i18n),
    )
}

/// Column header line; `widths` are those of the rows below it. A blank title leaves its column
/// blank. The line sits on a `surface_container_high` band, like the other tables' headers.
pub fn header(ui: &mut egui::Ui, widths: &[f32], titles: &[String]) {
    let band = egui::Frame::new()
        .fill(current_ui(ui).surface_container_high)
        .corner_radius(shape::radius(shape::SMALL));
    band.show(ui, |ui| {
        ui.horizontal(|ui| {
            for (width, title) in widths.iter().zip(titles) {
                cell(ui, *width, |ui| {
                    ui.strong(title);
                });
            }
        });
    });
}
