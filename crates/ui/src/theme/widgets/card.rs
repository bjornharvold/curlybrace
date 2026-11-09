//! A Material 3 card: a rounded container for related content.
use crate::theme::current_ui;
use crate::theme::shape::{self, radius};
use eframe::egui::{Frame, InnerResponse, Margin, Stroke, Ui};

/// `add` inside a `surface_container_low` frame with the medium (12 dp) radius and a hairline
/// `outline_variant` border (the panels share the card's fill, so the border keeps the card's
/// edge visible), with 16 dp padding and no shadow.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let s = current_ui(ui);
    Frame::new()
        .fill(s.surface_container_low)
        .stroke(Stroke::new(1.0, s.outline_variant))
        .corner_radius(radius(shape::MEDIUM))
        .inner_margin(Margin::same(shape::SPACE_4 as i8))
        .show(ui, add)
}
