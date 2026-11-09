//! A tonal, fully rounded status chip (for example the index verdict). Not interactive.
use super::{medium_font, one_row, painter};
use crate::theme::shape::{self, radius};
use crate::theme::{Tone, current_ui};
use eframe::egui::{Response, Sense, TextStyle, Ui, Vec2, WidgetInfo, WidgetType};

/// Chip height: Material's 32 dp chip, compacted to sit inside a table row.
pub const CHIP_HEIGHT: f32 = 24.0;

/// `text` on the tone's container color, elided with an ellipsis when it does not fit the
/// available width (the full text is then the tooltip). Exposed as a label with the full text.
pub fn chip(ui: &mut Ui, tone: Tone, text: &str) -> Response {
    let colors = current_ui(ui).tone(tone);
    let padding = shape::SPACE_3;
    let size = TextStyle::Small.resolve(ui.style()).size;
    let galley = one_row(
        ui,
        text,
        medium_font(ui, size),
        ui.available_width() - 2.0 * padding,
    );
    let desired = Vec2::new(galley.size().x + 2.0 * padding, CHIP_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), text));
    if galley.elided {
        response = response.on_hover_text(text);
    }
    if ui.is_rect_visible(rect) {
        let painter = painter(ui);
        painter.rect_filled(rect, radius(shape::FULL), colors.container);
        painter.galley(
            rect.center() - galley.size() / 2.0,
            galley,
            colors.on_container,
        );
    }
    response
}
