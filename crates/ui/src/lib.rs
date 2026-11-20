//! Curlybrace UI crate.
pub mod app;
pub mod error_text;
pub mod i18n;
#[cfg(test)]
mod i18n_usage_tests;
#[cfg(test)]
mod log_capture;
pub mod theme;
pub mod viewmodel;
pub mod views;
pub use i18n::I18n;
