//! Material 3 shape scale and the 4 dp spacing grid (values in egui points).

/// Checkbox-sized corners (Material: 2-4 dp); what egui-native widgets use.
pub const EXTRA_SMALL: f32 = 4.0;
pub const SMALL: f32 = 8.0;
pub const MEDIUM: f32 = 12.0;
pub const LARGE: f32 = 16.0;
pub const EXTRA_LARGE: f32 = 28.0;
/// Fully rounded (buttons, chips): larger than any half-height, so it clamps to a pill.
pub const FULL: f32 = 999.0;

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 20.0;
pub const SPACE_6: f32 = 24.0;

/// A uniform egui corner radius for `points`; egui stores radii as `u8`, so [`FULL`] and any
/// other oversized value saturates (egui clamps to a pill anyway) and negatives become 0.
pub fn radius(points: f32) -> eframe::egui::CornerRadius {
    // `as` on f32 -> u8 saturates and maps NaN to 0.
    eframe::egui::CornerRadius::same(points.round() as u8)
}
