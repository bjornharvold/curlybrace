//! egui views. Logic lives in `viewmodel`; these only render and collect input.
pub mod compare;
pub mod connection_badge;
pub mod connections;
pub mod connections_edit;
pub mod connections_edit_dialog;
pub mod connections_edit_mode;
pub mod connections_folder_prompt;
pub mod connections_from_uri;
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
pub mod sidebar_roots;
pub mod table;
pub mod tree;
pub mod write_warning;
