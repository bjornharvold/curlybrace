//! Material 3 design tokens: color schemes, shapes and a WCAG contrast check.
pub mod contrast;
pub mod scheme;
pub mod shape;

#[cfg(test)]
mod scheme_tests;

pub use scheme::{Mode, Scheme, Tone, ToneColors, scheme};
