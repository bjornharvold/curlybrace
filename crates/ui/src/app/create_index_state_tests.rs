//! What turns Browse's Create index... off, read from the app state.
use super::create_index_state::create_index_state;
use super::index_write_tests::{browsing, plain};
use super::queries_test_support::{consenting, flags};
use super::state::AppState;
use super::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::{IndexBuilderState, IndexWrite};
use crate::views::create_index::CreateIndex;
use curlybrace_core::runtime::Command;

fn warned(s: AppState) -> AppState {
    let session = s.session_id();
    let write_warning = Some(ParkedWrite {
        command: Command::SetProfilingLevel {
            session,
            db: "app".into(),
            level: 2,
        },
        pending: None,
        kind: WriteKind::SetProfilingLevel,
        epoch: s.latest_requested_epoch,
    });
    AppState { write_warning, ..s }
}

#[test]
fn a_writable_idle_connection_is_on() {
    let (s, _) = plain();
    assert_eq!(create_index_state(&s), CreateIndex::On);
}

#[test]
fn a_read_only_connection_is_read_only() {
    let (s, _) = browsing(flags(false, true));
    assert_eq!(create_index_state(&s), CreateIndex::ReadOnly);
}

#[test]
fn an_index_write_in_flight_is_index_write() {
    let (mut s, _) = plain();
    s.indexes.writing = Some(IndexWrite::Create {
        name: "a_1".into(),
        replaces: None,
    });
    assert_eq!(create_index_state(&s), CreateIndex::IndexWrite);
}

#[test]
fn an_open_dialog_is_prompt() {
    let (s, _) = plain();
    let builder = IndexBuilderState::new(s.selected_ns().unwrap().clone());
    let indexes = s.indexes.clone().with_builder(Some(builder));
    let with_builder = AppState { indexes, ..s };
    assert_eq!(create_index_state(&with_builder), CreateIndex::Prompt);
    let (s, _) = plain();
    assert_eq!(
        create_index_state(&consenting(s, "app", 2)),
        CreateIndex::Prompt
    );
}

#[test]
fn a_parked_write_warning_is_write_warning() {
    let (s, _) = plain();
    assert_eq!(create_index_state(&warned(s)), CreateIndex::WriteWarning);
}
