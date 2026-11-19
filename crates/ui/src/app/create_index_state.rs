//! Whether Create index... works in Browse and the Queries view, read from the app state: not on
//! a read-only connection, not while an index write is out, not while a document write waits for
//! the production warning, not while another index or profiler dialog is open.
use super::state::AppState;
use crate::views::create_index::{CreateBlockers, CreateIndex};

pub(super) fn create_index_state(state: &AppState) -> CreateIndex {
    CreateIndex::of(CreateBlockers {
        read_only: state.active_is_read_only(),
        index_write: state.indexes.writing.is_some(),
        prompt: state.indexes.has_prompt(),
        write_warning: state.write_warning.is_some(),
    })
}
