//! The index builder in the reducer: opening, editing, creating through the write guard, and what
//! the runtime answers.
use super::reducer::{Input, update};
use super::state::{AppState, Banner};
use super::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::index_builder::BUILD_WARNING_THRESHOLD;
use crate::viewmodel::{BuildWarning, BuilderEdit, CollectionTab, IndexesAction, IndexesState};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::{mock_connection, mock_index_spec_named};
use uuid::Uuid;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

/// Opens `cfg` as the active, connected session with `app.people` selected on the Indexes tab.
fn browsing(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let id = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(id));
    let epoch = s.epoch_counter;
    let (mut s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    );
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .selecting(ns());
    s.tab = CollectionTab::Indexes;
    s.indexes = IndexesState::default().for_collection(ns());
    (s, id)
}

fn plain() -> (AppState, Uuid) {
    browsing(mock_connection())
}

fn with_flags(production: bool, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        production,
        read_only,
        ..mock_connection()
    }
}

fn act(s: AppState, action: IndexesAction) -> (AppState, Vec<Command>) {
    update(s, Input::Indexes(action))
}

fn edit(s: AppState, edit: BuilderEdit) -> AppState {
    act(s, IndexesAction::Builder(edit)).0
}

/// The builder open with one valid key on `field`.
fn building(cfg: ConnectionConfig, field: &str) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let (s, _) = act(s, IndexesAction::OpenBuilder);
    (edit(s, BuilderEdit::SetField(0, field.into())), id)
}

/// `Command` is not `PartialEq`; its `Debug` text is a faithful stand-in.
fn shown(commands: &[Command]) -> String {
    format!("{commands:?}")
}

fn sent_spec(commands: &[Command]) -> IndexSpec {
    match commands {
        [Command::CreateIndex { spec, .. }] => spec.clone(),
        other => panic!("expected one CreateIndex, got {other:?}"),
    }
}

#[test]
fn opening_the_builder_creates_it_for_the_selected_collection_and_asks_for_the_count() {
    let (s, id) = plain();
    let (s, commands) = act(s, IndexesAction::OpenBuilder);
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    assert_eq!(builder.ns, ns());
    assert!(builder.is_untouched());
    assert_eq!(
        shown(&commands),
        shown(&[Command::EstimatedCount {
            session: id,
            ns: ns()
        }])
    );
    // Opening it again changes nothing and asks nothing.
    let (again, commands) = act(s, IndexesAction::OpenBuilder);
    assert!(commands.is_empty());
    assert!(again.indexes.builder.is_some());
}

#[test]
fn edits_reach_the_open_builder_and_cancel_closes_it() {
    let (s, _) = building(mock_connection(), "email");
    let builder = s.indexes.builder.clone().unwrap();
    assert_eq!(builder.keys[0].field, "email");
    assert_eq!(builder.name, "email_1");
    let s = edit(s, BuilderEdit::SetUnique(true));
    assert!(s.indexes.builder.as_ref().unwrap().options.unique);
    let (s, commands) = act(s, IndexesAction::CancelBuilder);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
    // An edit with no builder open is dropped.
    let s = edit(s, BuilderEdit::AddKey);
    assert!(s.indexes.builder.is_none());
}

#[test]
fn creating_sends_create_index_for_the_selected_collection() {
    let (s, id) = building(mock_connection(), "email");
    let s = edit(s, BuilderEdit::SetUnique(true));
    let expected = s.indexes.builder.as_ref().unwrap().spec().unwrap();
    let (s, commands) = act(s, IndexesAction::Create);
    assert_eq!(
        shown(&commands),
        shown(&[Command::CreateIndex {
            session: id,
            ns: ns(),
            spec: expected.clone()
        }])
    );
    assert_eq!(sent_spec(&commands), expected);
    assert_eq!(s.pending.as_ref().map(|p| p.ns.clone()), Some(ns()));
    // The builder stays open until the server has answered; a second Create is ignored.
    assert!(s.indexes.builder.is_some());
    let (s, again) = act(s, IndexesAction::Create);
    assert!(again.is_empty());
    assert!(s.pending.is_some());
}

#[test]
fn an_invalid_builder_sends_nothing() {
    let (s, _) = browsing(mock_connection());
    let (s, _) = act(s, IndexesAction::OpenBuilder);
    // The one key row is still empty.
    let (s, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty());
    assert!(s.pending.is_none() && s.write_warning.is_none());
    // With no builder open nothing is sent either.
    let (s, _) = act(s, IndexesAction::CancelBuilder);
    let (_, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty());
}

#[test]
fn a_failed_create_keeps_the_builder_open_with_the_users_input() {
    let (s, id) = building(mock_connection(), "email");
    let s = edit(s, BuilderEdit::SetName("people_email".into()));
    let s = edit(s, BuilderEdit::SetPartialFilterText(r#"{ "a": 1 }"#.into()));
    let (s, _) = act(s, IndexesAction::Create);
    let before = s.indexes.builder.clone();
    let (s, commands) = update(
        s,
        Input::Event(Event::Failed {
            session: id,
            context: "create_index",
            error: CoreError::Database("index already exists with a different name".into()),
        }),
    );
    assert!(commands.is_empty());
    assert_eq!(s.indexes.builder, before, "the input survives");
    assert!(s.pending.is_none(), "the person can try again");
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "create_index",
            ..
        })
    ));
    assert!(s.banner.as_ref().unwrap().is_data_failure());
    let (_, retry) = act(s, IndexesAction::Create);
    assert_eq!(retry.len(), 1);
}

#[test]
fn index_changed_closes_the_builder_and_selects_the_new_index() {
    let (s, id) = building(mock_connection(), "email");
    let (s, _) = act(s, IndexesAction::Create);
    let (s, commands) = update(
        s,
        Input::Event(Event::IndexChanged {
            session: id,
            ns: ns(),
        }),
    );
    assert!(s.indexes.builder.is_none());
    assert!(s.pending.is_none());
    assert_eq!(s.indexes.selected.as_deref(), Some("email_1"));
    // The lists are asked for again, as after every index write.
    assert_eq!(
        shown(&commands),
        shown(&[
            Command::ListIndexes {
                session: id,
                ns: ns()
            },
            Command::IndexHistory {
                session: id,
                ns: ns(),
                range: s.indexes.range
            },
        ])
    );
    // The new index comes back in the list and stays selected.
    let listed = CollectionIndex {
        spec: mock_index_spec_named("email_1", &[("email", KeyKind::Asc)]),
        size_bytes: 4096,
    };
    let (s, _) = update(
        s,
        Input::Event(Event::Indexes {
            session: id,
            ns: ns(),
            indexes: vec![listed],
        }),
    );
    assert_eq!(s.indexes.selected.as_deref(), Some("email_1"));
}

#[test]
fn index_changed_for_another_collection_leaves_the_builder_alone() {
    let (s, id) = building(mock_connection(), "email");
    let (s, commands) = update(
        s,
        Input::Event(Event::IndexChanged {
            session: id,
            ns: Namespace::new("app", "pets"),
        }),
    );
    assert!(s.indexes.builder.is_some());
    assert!(commands.is_empty());
}

#[test]
fn a_read_only_connection_disables_every_write_control_and_shows_why() {
    // The reducer side: the builder does not open, a builder that was open when the connection
    // got locked cannot create, and nothing is sent either way.
    let (s, _) = browsing(with_flags(false, true));
    let (s, commands) = act(s, IndexesAction::OpenBuilder);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));

    let (s, id) = building(mock_connection(), "email");
    let mut s = s;
    s.conn.connections.iter_mut().for_each(|c| {
        if c.id == id {
            c.read_only = true;
        }
    });
    let (s, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty());
    assert!(s.pending.is_none() && s.write_warning.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(s.indexes.builder.is_some(), "the input is not lost");
}

#[test]
fn creating_on_a_production_connection_parks_for_the_production_warning() {
    let (s, id) = building(with_flags(true, false), "email");
    let (s, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty(), "nothing is sent before the answer");
    assert!(s.pending.is_none());
    let parked = s.write_warning.as_ref().expect("the write is parked");
    assert_eq!(parked.kind, WriteKind::CreateIndex);
    assert!(parked.command.is_write());
    assert_eq!(parked.command.session(), Some(id));
    assert_eq!(parked.pending.ns, ns());
    assert!(s.indexes.builder.is_some());

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(sent_spec(&commands).name, "email_1");
    assert!(s.pending.is_some() && s.write_warning.is_none());

    // Cancelling sends nothing and keeps the builder.
    let (s, _) = building(with_flags(true, false), "email");
    let (s, _) = act(s, IndexesAction::Create);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Cancel));
    assert!(commands.is_empty() && s.indexes.builder.is_some());
}

#[test]
fn a_collection_over_one_million_documents_shows_the_build_warning() {
    let (s, id) = building(mock_connection(), "email");
    let count = BUILD_WARNING_THRESHOLD + 1;
    let (s, commands) = update(
        s,
        Input::Event(Event::EstimatedCount {
            session: id,
            ns: ns(),
            count,
        }),
    );
    assert!(commands.is_empty());
    let builder = s.indexes.builder.as_ref().unwrap();
    assert_eq!(
        builder.warning,
        Some(BuildWarning::LargeCollection { documents: count })
    );
    assert_eq!(builder.estimated_documents, Some(count));
    // The warning does not stand in the way of Create.
    let (_, commands) = act(s, IndexesAction::Create);
    assert_eq!(commands.len(), 1);
}

#[test]
fn no_warning_at_or_below_the_threshold() {
    for count in [0, 5_000, BUILD_WARNING_THRESHOLD] {
        let (s, id) = building(mock_connection(), "email");
        let (s, _) = update(
            s,
            Input::Event(Event::EstimatedCount {
                session: id,
                ns: ns(),
                count,
            }),
        );
        let builder = s.indexes.builder.as_ref().unwrap();
        assert_eq!(builder.warning, None, "{count}");
        assert_eq!(builder.estimated_documents, Some(count));
    }
}

#[test]
fn an_estimate_for_another_collection_or_no_builder_is_dropped() {
    let (s, id) = building(mock_connection(), "email");
    let (s, _) = update(
        s,
        Input::Event(Event::EstimatedCount {
            session: id,
            ns: Namespace::new("app", "pets"),
            count: 9_000_000,
        }),
    );
    assert_eq!(s.indexes.builder.as_ref().unwrap().warning, None);
    let (s, _) = act(s, IndexesAction::CancelBuilder);
    let (s, _) = update(
        s,
        Input::Event(Event::EstimatedCount {
            session: id,
            ns: ns(),
            count: 9_000_000,
        }),
    );
    assert!(s.indexes.builder.is_none());
}

#[test]
fn a_missing_estimate_never_blocks_create() {
    let (s, id) = building(mock_connection(), "email");
    let (s, _) = update(
        s,
        Input::Event(Event::Failed {
            session: id,
            context: "estimated_count",
            error: CoreError::Unsupported("this store cannot count documents".into()),
        }),
    );
    let builder = s.indexes.builder.as_ref().unwrap();
    assert_eq!(
        (builder.warning.clone(), builder.estimated_documents),
        (None, None)
    );
    assert_eq!(s.banner, None, "an estimate is advice, not worth a banner");
    assert!(builder.problem().is_none());
    let (_, commands) = act(s, IndexesAction::Create);
    assert_eq!(commands.len(), 1);
}

#[test]
fn switching_collection_closes_the_builder() {
    let (s, _) = building(mock_connection(), "email");
    let (s, _) = update(
        s,
        Input::Sidebar(crate::viewmodel::SidebarAction::Open(Namespace::new(
            "app", "pets",
        ))),
    );
    assert!(s.indexes.builder.is_none());
}

#[test]
fn the_embedded_session_creates_without_a_warning() {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = super::state::ConnStatus::Connected;
    s.sidebar = s.sidebar.selecting(ns());
    s.indexes = IndexesState::default().for_collection(ns());
    let (s, _) = act(s, IndexesAction::OpenBuilder);
    let s = edit(s, BuilderEdit::SetField(0, "a".into()));
    let (_, commands) = act(s, IndexesAction::Create);
    assert_eq!(sent_spec(&commands).keys.len(), 1);
}
