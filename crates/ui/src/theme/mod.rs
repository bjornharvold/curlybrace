//! Material 3 design tokens: color schemes, shapes and a WCAG contrast check, plus the
//! installation of that theme into egui.
pub mod contrast;
mod fonts;
pub mod icons;
mod install;
pub mod scheme;
pub mod shape;
pub mod typography;
pub mod widgets;

#[cfg(test)]
mod fonts_tests;
#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod scheme_tests;

pub use fonts::{MEDIUM_FAMILY, SYMBOLS_FAMILY, font_definitions};
pub use install::{StateLayer, current, current_ui, install, state_layer, visuals_for};
pub use scheme::{Mode, Scheme, Tone, ToneColors, scheme};
pub use widgets::{
    card, chip, filled_button, outlined_field, outlined_field_with, pill, segmented, text_button,
    tonal_button,
};
