//! egui views. Logic lives in `viewmodel`; these only render and collect input.
pub mod compare;
pub mod connections;
pub mod connections_edit;
pub mod connections_folder_prompt;
pub mod connections_list;
#[cfg(test)]
mod connections_list_tests;
pub mod context_menu;
pub mod documents;
pub mod editor;
pub mod error_label;
pub mod findbar;
pub mod json;
pub mod metrics;
pub mod sidebar;
pub mod table;
pub mod tree;
