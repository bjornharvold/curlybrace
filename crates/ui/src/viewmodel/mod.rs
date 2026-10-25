//! Pure, egui-free view-model helpers.
pub mod cell;
pub mod compare;
pub mod connection_form;
pub mod connection_list;
pub mod connection_tag;
pub mod copy_clipboard;
pub mod copy_menu;
pub mod doc_actions;
pub mod doc_copy;
pub mod editor;
pub mod findbar;
pub mod index_builder;
pub mod index_chart;
pub mod index_detail;
pub mod index_write;
pub mod indexes;
pub mod json_cache;
pub mod page_key;
pub mod sidebar;
pub mod sidebar_selection;
pub mod table;
pub mod tree;
pub mod tree_state;
pub mod typed_confirmation;

pub use cell::cell_text;
pub use connection_form::{
    ConnectionForm, SecretEdit, StoredSecrets, auth_label_key, auth_options, save_connection,
    stored_secret_flags,
};
pub use connection_list::{
    Group, Security, Sort, SortColumn, format_utc, grouped, security, server_hosts, servers,
    visible,
};
pub use connection_tag::{ColorTag, ConnectionBadge, color_tag};
pub use copy_clipboard::{CopyClipboard, DocSelection, PasteBlock, PasteContext, PasteTarget};
pub use copy_menu::{
    PasteEntry, copy_entry, keyboard_paste_target, paste_block_key, paste_entry, paste_target_for,
};
pub use doc_actions::{
    DeleteConfirm, DocAction, MenuItem, choose, id_preview, is_projected, menu_items,
};
pub use doc_copy::{CopyEntry, DocCopy, DocPick, clipboard_for, copy_blocked};
pub use editor::{
    EditorAction, EditorIntent, EditorState, dismissal_action, resolve_editor, try_save,
};
pub use findbar::{
    FindAction, FindBarState, FindIntent, pick_intent, resolve_find_intent, submit_find,
};
pub use index_builder::{BuildWarning, BuilderEdit, BuilderProblem, IndexBuilderState};
pub use index_write::{DropConfirm, DropOld, IndexWrite, TtlPrompt};
pub use indexes::{CollectionTab, IndexesAction, IndexesState};
pub use json_cache::{JsonCache, view_json_cache};
pub use page_key::{PageKey, page_key};
pub use sidebar::{
    DbNode, SidebarAction, SidebarState, collection_click_action, compare_database_to_action,
    compare_to_action, db_click_action,
};
pub use sidebar_selection::{
    NodeRef, SelectMode, SidebarSelection, clipboard_from_selection, visible_nodes,
};
pub use table::table_columns;
pub use tree::{
    MAX_ROWS_PER_LEVEL, RowDetail, TreeRow, ViewMode, flatten_doc, row_value_text, type_label_key,
};
pub use tree_state::{PageRow, TreeState, flatten_page};
pub use typed_confirmation::typed_text_matches;
