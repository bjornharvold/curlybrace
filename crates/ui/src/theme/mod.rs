//! Material 3 design tokens: color schemes, shapes and a WCAG contrast check, plus the
//! installation of that theme into egui.
pub mod contrast;
mod fonts;
pub mod icons;
mod install;
pub mod scheme;
mod semantic;
pub mod shape;
pub mod typography;
pub mod widgets;

#[cfg(test)]
mod fonts_tests;
#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod scheme_tests;
#[cfg(test)]
mod semantic_tests;

pub use fonts::{MEDIUM_FAMILY, SYMBOLS_FAMILY, font_definitions};
pub use install::{StateLayer, current, current_ui, install, state_layer, visuals_for};
pub use scheme::{Mode, Scheme, Tone, ToneColors, scheme};
pub use semantic::{ChartColors, JsColors, JsonColors, chart_colors, js_colors, json_colors};
pub(crate) use widgets::CONTROL_HEIGHT;
pub use widgets::{
    CHIP_HEIGHT, FieldOptions, PILL_HEIGHT, card, chip, compact_text_button, filled_button, gate,
    modal_frame, outlined_field, outlined_field_opts, outlined_field_with, pill, pill_row,
    segmented, segmented_hinted, segmented_hinted_compact, segmented_where, tabs, text_button,
    text_button_with_icon, tonal_button,
};
