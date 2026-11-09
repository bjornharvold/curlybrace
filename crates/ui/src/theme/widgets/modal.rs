//! The Material 3 dialog container for `egui::Modal`: `surface_container_high`, the extra-large
//! (28 dp) radius, 24 dp padding and the installed light elevation shadow.
use crate::theme::current;
use crate::theme::shape::{self, radius};
use eframe::egui::{Context, Frame, Margin};

/// The frame to pass to `egui::Modal::frame`.
pub fn modal_frame(ctx: &Context) -> Frame {
    Frame::new()
        .fill(current(ctx).surface_container_high)
        .corner_radius(radius(shape::EXTRA_LARGE))
        .shadow(ctx.style().visuals.window_shadow)
        .inner_margin(Margin::same(shape::SPACE_6 as i8))
}
