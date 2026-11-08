//! Material 3 design tokens: color schemes, shapes and a WCAG contrast check, plus the
//! installation of that theme into egui.
pub mod contrast;
mod install;
pub mod scheme;
pub mod shape;
pub mod typography;

#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod scheme_tests;

pub use install::{StateLayer, current, current_ui, install, state_layer, visuals_for};
pub use scheme::{Mode, Scheme, Tone, ToneColors, scheme};
