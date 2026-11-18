//! The cell one field of the find bar sits in. An outlined field's label floats above its box, so
//! a field needs a little more height than a control; giving every field of a wrapping row the
//! same fixed-height cell, allocated at its final width up front, keeps the fields aligned and
//! lets the row wrap between them.
use crate::theme::CONTROL_HEIGHT;
use eframe::egui::{self, Align, Layout, vec2};

/// The height of a row of fields: a control plus the floating label's overhang above and
/// mirrored below, so the box is centred in the row and so are the buttons beside it.
pub(super) fn field_row_height(ui: &egui::Ui) -> f32 {
    let overhang = ui.text_style_height(&egui::TextStyle::Small) / 2.0;
    CONTROL_HEIGHT + 2.0 * overhang
}

/// `add` in a cell `width` wide and `height` tall (the row height of [`field_row_height`]),
/// placed in the current row of a wrapping layout (it wraps when the cell does not fit; a
/// `ui.vertical` cannot, it takes the rest of the row).
pub(super) fn field_cell<R>(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
        ui.set_min_height(height);
        add(ui)
    })
    .inner
}
