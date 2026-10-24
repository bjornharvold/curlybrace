//! Editing a copy, hiding, changing the TTL and dropping in the reducer: the flows, the guards
//! (`_id_`, read-only, production) and the lesson of the create flow, that an answer acts on what
//! was SENT and not on whatever is open or selected when it arrives.
use super::index_write_tests::{
    act, browsing, changed, edit, ns, plain, sent_spec, shown, with_flags,
};
use super::reducer::{Input, update};
use super::state::{AppState, Banner};
use super::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::{
    BuilderEdit, DropConfirm, DropOld, IndexWrite, IndexesAction, IndexesState, TtlPrompt,
};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::indexes::{IndexOptions, IndexSpec, KeyKind};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::CollectionIndex;
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use uuid::Uuid;

fn email() -> IndexSpec {
    IndexSpec {
        options: IndexOptions {
            unique: true,
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("email_1", &[("email", KeyKind::Asc)])
    }
}

fn status() -> IndexSpec {
    mock_index_spec_named("status_1", &[("status", KeyKind::Asc)])
}

fn expiring() -> IndexSpec {
    IndexSpec {
        options: IndexOptions {
            expire_after_seconds: Some(100),
            ..IndexOptions::default()
        },
        ..mock_index_spec_named("createdAt_1", &[("createdAt", KeyKind::Asc)])
    }
}

fn id_index() -> IndexSpec {
    mock_index_spec_named("_id_", &[("_id", KeyKind::Asc)])
}

fn listed(spec: IndexSpec) -> CollectionIndex {
    CollectionIndex {
        spec,
        size_bytes: 4096,
    }
}

/// `s` with `specs` as the listed indexes of `app.people`.
fn with_indexes(s: AppState, specs: Vec<IndexSpec>) -> AppState {
    let indexes = s
        .indexes
        .clone()
        .with_indexes(specs.into_iter().map(listed).collect());
    AppState { indexes, ..s }
}

fn on_a_normal_collection() -> (AppState, Uuid) {
    let (s, id) = plain();
    (
        with_indexes(s, vec![id_index(), email(), status(), expiring()]),
        id,
    )
}

fn failed(s: AppState, id: Uuid, context: &'static str) -> AppState {
    update(
        s,
        Input::Event(Event::Failed {
            session: id,
            context,
            error: CoreError::Database("nope".into()),
        }),
    )
    .0
}

fn only_drop(commands: &[Command]) -> String {
    match commands {
        [Command::DropIndex { name, .. }] => name.clone(),
        other => panic!("expected one DropIndex, got {other:?}"),
    }
}

/// `email_1` edited into `email_1_v2` and sent.
fn copy_sent() -> (AppState, Uuid, Vec<Command>) {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::EditCopy("email_1".into()));
    let s = edit(s, BuilderEdit::SetSparse(true));
    let (s, commands) = act(s, IndexesAction::Create);
    (s, id, commands)
}

#[test]
fn editing_a_copy_creates_first_then_asks_before_dropping_the_old_index() {
    let (s, id) = on_a_normal_collection();
    let (s, commands) = act(s, IndexesAction::EditCopy("email_1".into()));
    assert_eq!(commands.len(), 1, "only the document count is requested");
    let builder = s.indexes.builder.as_ref().expect("the builder opens");
    assert_eq!(builder.editing, Some(email()));
    assert_eq!(builder.replaces(), Some("email_1"));
    assert_eq!(builder.name, "email_1_v2", "the copy never collides");
    assert_eq!(builder.keys, email().keys);
    assert!(builder.options.unique, "options are pre-filled");

    let s = edit(s, BuilderEdit::SetSparse(true));
    let (s, commands) = act(s, IndexesAction::Create);
    // Create first: the old index is not touched by the send.
    let spec = sent_spec(&commands);
    assert_eq!(spec.name, "email_1_v2");
    assert!(spec.options.unique && spec.options.sparse);

    // The build succeeded: the person is asked, nothing is dropped yet.
    let (s, commands) = changed(s, id);
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, Command::DropIndex { .. })),
        "{commands:?}"
    );
    assert_eq!(
        s.indexes.drop_old,
        Some(DropOld {
            old: "email_1".into(),
            new: "email_1_v2".into()
        })
    );
    assert!(s.indexes.builder.is_none(), "the builder closes");
    assert_eq!(s.indexes.selected.as_deref(), Some("email_1_v2"));

    // Agreeing drops exactly the original, through the same guard.
    let (s, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert_eq!(only_drop(&commands), "email_1");
    assert_eq!(
        s.indexes.writing,
        Some(IndexWrite::Drop {
            name: "email_1".into(),
            then_select: Some("email_1_v2".into())
        })
    );
    let (s, commands) = changed(s, id);
    assert!(s.indexes.drop_old.is_none() && s.indexes.writing.is_none());
    assert_eq!(
        s.indexes.selected.as_deref(),
        Some("email_1_v2"),
        "the new index is selected"
    );
    assert_eq!(commands.len(), 2, "the lists are asked for again");
}

#[test]
fn a_failed_build_never_drops_the_original() {
    let (s, id, _) = copy_sent();
    let s = failed(s, id, "create_index");
    assert!(s.indexes.drop_old.is_none(), "no question after a failure");
    assert!(s.indexes.writing.is_none() && s.pending.is_none());
    let builder = s.indexes.builder.as_ref().expect("the builder is back");
    assert!(!builder.submitted, "and editable, to try again");
    // Nothing can drop the original any more.
    let (s, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert!(commands.is_empty());
    let (_, late) = changed(s, id);
    assert!(!late.iter().any(|c| matches!(c, Command::DropIndex { .. })));
}

#[test]
fn declining_leaves_both_indexes() {
    let (s, id, _) = copy_sent();
    let (s, _) = changed(s, id);
    assert!(s.indexes.drop_old.is_some());
    let (s, commands) = act(s, IndexesAction::KeepOld);
    assert!(commands.is_empty(), "keeping sends nothing");
    assert!(s.indexes.drop_old.is_none());
    assert!(s.pending.is_none() && s.indexes.writing.is_none());
    // The question does not come back and cannot be answered with a drop.
    let (_, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert!(commands.is_empty());
}

#[test]
fn the_original_name_is_the_one_dropped_even_if_the_selection_changed_meanwhile() {
    let (s, id, _) = copy_sent();
    // While the build runs the person looks at another index.
    let (s, _) = act(s, IndexesAction::Select("status_1".into()));
    let (s, _) = changed(s, id);
    let (_, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert_eq!(only_drop(&commands), "email_1", "not status_1");
}

#[test]
fn cancelling_the_copy_builder_in_flight_still_asks_about_the_original() {
    // Decision: the build cannot be cancelled by closing the modal, so its late success still
    // asks (it never drops by itself) about the original that was recorded when it was sent.
    let (s, id, _) = copy_sent();
    let (s, _) = act(s, IndexesAction::CancelBuilder);
    let (s, _) = act(s, IndexesAction::Select("status_1".into()));
    let (s, _) = changed(s, id);
    assert_eq!(
        s.indexes.drop_old,
        Some(DropOld {
            old: "email_1".into(),
            new: "email_1_v2".into()
        })
    );
    let (_, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert_eq!(only_drop(&commands), "email_1");
}

#[test]
fn the_production_confirm_records_the_original() {
    let (s, id) = browsing(with_flags(true, false));
    let s = with_indexes(s, vec![email(), status()]);
    let (s, _) = act(s, IndexesAction::EditCopy("email_1".into()));
    let (s, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty(), "parked");
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(commands.len(), 1);
    let (s, _) = act(s, IndexesAction::Select("status_1".into()));
    let (s, _) = changed(s, id);
    assert_eq!(s.indexes.drop_old.as_ref().unwrap().old, "email_1");
    // And the drop of the original is guarded again.
    let (s, commands) = act(s, IndexesAction::ConfirmDropOld);
    assert!(commands.is_empty());
    assert_eq!(s.write_warning.as_ref().unwrap().kind, WriteKind::DropIndex);
}

#[test]
fn a_new_builder_is_not_a_copy() {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::OpenBuilder);
    let s = edit(s, BuilderEdit::SetField(0, "city".into()));
    let (s, _) = act(s, IndexesAction::Create);
    let (s, _) = changed(s, id);
    assert!(s.indexes.drop_old.is_none(), "nothing to ask about");
}

#[test]
fn a_copy_cannot_open_while_something_else_is_open_or_in_flight() {
    let (s, _) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::OpenBuilder);
    let (s, commands) = act(s, IndexesAction::EditCopy("email_1".into()));
    assert!(commands.is_empty());
    assert_eq!(s.indexes.builder.as_ref().unwrap().editing, None);

    let (s, _, _) = copy_sent();
    let (s, _) = act(s, IndexesAction::CancelBuilder);
    let (s, commands) = act(s, IndexesAction::EditCopy("status_1".into()));
    assert!(commands.is_empty() && s.indexes.builder.is_none());
    // An unknown index has nothing to copy.
    let (s, _) = on_a_normal_collection();
    let (s, commands) = act(s, IndexesAction::EditCopy("nope".into()));
    assert!(commands.is_empty() && s.indexes.builder.is_none());
}

#[test]
fn a_ttl_only_change_uses_set_index_ttl_not_a_copy() {
    let (s, id) = on_a_normal_collection();
    let (s, commands) = act(s, IndexesAction::ChangeTtl("createdAt_1".into()));
    assert!(commands.is_empty());
    assert_eq!(
        s.indexes.ttl_prompt,
        Some(TtlPrompt::new("createdAt_1".into(), Some(100)))
    );
    let (s, _) = act(s, IndexesAction::TtlTyped("3600".into()));
    assert_eq!(s.indexes.ttl_prompt.as_ref().unwrap().text, "3600");

    // Below zero seconds: nothing is sent.
    for bad in [-1, -5] {
        let (fresh, _) = on_a_normal_collection();
        let (_, commands) = act(fresh, IndexesAction::SetTtl("createdAt_1".into(), bad));
        assert!(commands.is_empty(), "{bad}");
    }
    let (s, commands) = act(s, IndexesAction::SetTtl("createdAt_1".into(), 3600));
    assert_eq!(
        shown(&commands),
        shown(&[Command::SetIndexTtl {
            session: id,
            ns: ns(),
            name: "createdAt_1".into(),
            seconds: 3600
        }])
    );
    assert!(s.indexes.builder.is_none(), "no copy is made");
    assert_eq!(
        s.indexes.writing,
        Some(IndexWrite::Ttl {
            name: "createdAt_1".into()
        })
    );
    let (s, commands) = changed(s, id);
    assert!(s.indexes.ttl_prompt.is_none() && s.indexes.writing.is_none());
    assert_eq!(commands.len(), 2, "refreshed");

    // An index that does not expire cannot be given an expiry in place.
    let (s, _) = on_a_normal_collection();
    let (s, commands) = act(s, IndexesAction::ChangeTtl("email_1".into()));
    assert!(commands.is_empty() && s.indexes.ttl_prompt.is_none());
    let (_, commands) = act(s, IndexesAction::SetTtl("email_1".into(), 60));
    assert!(commands.is_empty());
}

#[test]
fn a_failed_ttl_change_keeps_the_prompt_for_another_try() {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::ChangeTtl("createdAt_1".into()));
    let (s, _) = act(s, IndexesAction::TtlTyped("60".into()));
    let (s, _) = act(s, IndexesAction::SetTtl("createdAt_1".into(), 60));
    let s = failed(s, id, "set_index_ttl");
    assert!(s.indexes.writing.is_none() && s.pending.is_none());
    assert_eq!(s.indexes.ttl_prompt.as_ref().unwrap().text, "60");
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "set_index_ttl",
            ..
        })
    ));
    assert!(s.banner.as_ref().unwrap().is_data_failure());
    let (s, commands) = act(s, IndexesAction::SetTtl("createdAt_1".into(), 60));
    assert_eq!(commands.len(), 1, "sent again");
    // While it is in flight the prompt holds still; the answer closes it.
    let (s, _) = act(s, IndexesAction::TtlCancelled);
    assert!(s.indexes.ttl_prompt.is_some());
    let (s, _) = changed(s, id);
    assert!(s.indexes.ttl_prompt.is_none());
    // After a failure it can be cancelled instead.
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::ChangeTtl("createdAt_1".into()));
    let (s, _) = act(s, IndexesAction::SetTtl("createdAt_1".into(), 60));
    let s = failed(s, id, "set_index_ttl");
    let (s, _) = act(s, IndexesAction::TtlCancelled);
    assert!(s.indexes.ttl_prompt.is_none());
}

#[test]
fn hiding_sends_set_index_hidden() {
    let (s, id) = on_a_normal_collection();
    let (s, commands) = act(s, IndexesAction::SetHidden("status_1".into(), true));
    assert_eq!(
        shown(&commands),
        shown(&[Command::SetIndexHidden {
            session: id,
            ns: ns(),
            name: "status_1".into(),
            hidden: true
        }])
    );
    assert_eq!(
        s.indexes.writing,
        Some(IndexWrite::Hide {
            name: "status_1".into()
        })
    );
    // A second write waits for the first.
    let (s, again) = act(s, IndexesAction::SetHidden("email_1".into(), true));
    assert!(again.is_empty());
    let (s, commands) = changed(s, id);
    assert!(s.indexes.writing.is_none());
    assert_eq!(commands.len(), 2, "the lists are asked for again");
}

#[test]
fn unhiding_sends_set_index_hidden_false() {
    let (s, id) = on_a_normal_collection();
    let (_, commands) = act(s, IndexesAction::SetHidden("status_1".into(), false));
    assert_eq!(
        shown(&commands),
        shown(&[Command::SetIndexHidden {
            session: id,
            ns: ns(),
            name: "status_1".into(),
            hidden: false
        }])
    );
}

#[test]
fn a_failed_hide_clears_the_marker_and_says_so() {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::SetHidden("status_1".into(), true));
    let s = failed(s, id, "set_index_hidden");
    assert!(s.indexes.writing.is_none() && s.pending.is_none());
    assert!(s.banner.as_ref().is_some_and(Banner::is_data_failure));
    let (_, commands) = act(s, IndexesAction::SetHidden("status_1".into(), true));
    assert_eq!(commands.len(), 1, "the person can try again");
}

#[test]
fn a_late_hide_answer_does_not_touch_an_open_builder() {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::SetHidden("status_1".into(), true));
    // A builder is on screen with the person's input when the answer arrives.
    let newer = crate::viewmodel::IndexBuilderState::new(ns())
        .with_field(0, "city")
        .with_unique(true);
    let s = AppState {
        indexes: s.indexes.with_builder(Some(newer.clone())),
        ..s
    };
    let (s, _) = changed(s, id);
    assert_eq!(s.indexes.builder, Some(newer), "the input survives");
    assert!(s.indexes.drop_old.is_none());
    assert!(s.indexes.writing.is_none());
}

#[test]
fn dropping_requires_typing_the_exact_index_name() {
    let name = "my.Índex v1";
    let odd = || {
        let (s, id) = plain();
        let odd = mock_index_spec_named(name, &[("a", KeyKind::Asc)]);
        (with_indexes(s, vec![id_index(), odd]), id)
    };
    let (s, id) = odd();
    let (s, commands) = act(s, IndexesAction::DropRequested(name.into()));
    assert!(commands.is_empty());
    assert_eq!(s.indexes.drop_confirm, Some(DropConfirm::new(name.into())));

    for wrong in [
        "",
        "my.índex v1",
        "my.Índex v",
        " my.Índex v1",
        "my.Índex v1 ",
        "MY.ÍNDEX V1",
    ] {
        let (s, _) = act(odd().0, IndexesAction::DropRequested(name.into()));
        let (s, _) = act(s, IndexesAction::DropTyped(wrong.into()));
        let (s, commands) = act(s, IndexesAction::DropConfirmed);
        assert!(commands.is_empty(), "{wrong:?} must not drop");
        assert!(s.pending.is_none() && s.indexes.writing.is_none());
    }

    let (s, _) = act(s, IndexesAction::DropTyped(name.into()));
    let (s, commands) = act(s, IndexesAction::DropConfirmed);
    assert_eq!(
        shown(&commands),
        shown(&[Command::DropIndex {
            session: id,
            ns: ns(),
            name: name.into()
        }])
    );
    assert!(s.indexes.drop_confirm.is_some(), "open until answered");
    let (s, commands) = changed(s, id);
    assert!(s.indexes.drop_confirm.is_none() && s.indexes.writing.is_none());
    assert_eq!(commands.len(), 2);
}

#[test]
fn a_failed_drop_keeps_the_modal_with_the_typed_name_to_retry_or_cancel() {
    let (s, id) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::DropRequested("status_1".into()));
    let (s, _) = act(s, IndexesAction::DropTyped("status_1".into()));
    let (s, _) = act(s, IndexesAction::DropConfirmed);
    let s = failed(s, id, "drop_index");
    assert!(s.indexes.writing.is_none() && s.pending.is_none());
    assert_eq!(s.indexes.drop_confirm.as_ref().unwrap().typed, "status_1");
    assert!(s.banner.as_ref().is_some_and(Banner::is_data_failure));
    let (s, commands) = act(s, IndexesAction::DropConfirmed);
    assert_eq!(only_drop(&commands), "status_1");
    let s = failed(s, id, "drop_index");
    let (s, commands) = act(s, IndexesAction::DropCancelled);
    assert!(commands.is_empty() && s.indexes.drop_confirm.is_none());
}

#[test]
fn id_index_cannot_be_dropped_or_hidden() {
    let fresh = || on_a_normal_collection().0;
    let id_name = || "_id_".to_string();
    let (after, commands) = act(fresh(), IndexesAction::DropRequested(id_name()));
    assert!(commands.is_empty() && after.indexes.drop_confirm.is_none());
    let (after, commands) = act(fresh(), IndexesAction::SetHidden(id_name(), true));
    assert!(commands.is_empty() && after.pending.is_none() && after.indexes.writing.is_none());
    let (after, commands) = act(fresh(), IndexesAction::EditCopy(id_name()));
    assert!(commands.is_empty() && after.indexes.builder.is_none());
    let (after, commands) = act(fresh(), IndexesAction::ChangeTtl(id_name()));
    assert!(commands.is_empty() && after.indexes.ttl_prompt.is_none());
    let (_, commands) = act(fresh(), IndexesAction::SetTtl(id_name(), 60));
    assert!(commands.is_empty());
    let s = fresh();

    // Even a confirmation forced onto `_id_` sends nothing.
    let forced = AppState {
        indexes: s.indexes.with_drop_confirm(Some(DropConfirm {
            name: id_name(),
            typed: id_name(),
        })),
        ..s
    };
    let (_, commands) = act(forced, IndexesAction::DropConfirmed);
    assert!(commands.is_empty());
}

#[test]
fn an_index_that_is_not_listed_is_not_hidden_or_dropped() {
    let (s, _) = on_a_normal_collection();
    let (_, commands) = act(s, IndexesAction::SetHidden("nope".into(), true));
    assert!(commands.is_empty());
    let (s, _) = on_a_normal_collection();
    let (after, commands) = act(s, IndexesAction::DropRequested("nope".into()));
    assert!(commands.is_empty() && after.indexes.drop_confirm.is_none());
}

#[test]
fn hashed_and_2d_keys_survive_editing_a_copy() {
    let hashed = IndexSpec {
        raw: doc! { "key": { "h": "hashed" }, "name": "h_hashed", "v": 2 },
        ..mock_index_spec_named("h_hashed", &[("h", KeyKind::Hashed)])
    };
    let geo = IndexSpec {
        raw: doc! {
            "key": { "loc": "2d", "kind": 1 }, "name": "loc_2d_kind_1", "v": 2,
            "bits": 26, "min": -180.0, "max": 180.0,
        },
        ..mock_index_spec_named(
            "loc_2d_kind_1",
            &[("loc", KeyKind::Geo2d), ("kind", KeyKind::Asc)],
        )
    };
    for original in [hashed, geo] {
        let (s, _) = plain();
        let s = with_indexes(s, vec![original.clone()]);
        let (s, _) = act(s, IndexesAction::EditCopy(original.name.clone()));
        let s = edit(s, BuilderEdit::SetSparse(true));
        // A kind the picker does not offer cannot be typed over.
        let before = s.indexes.builder.clone();
        let s = edit(s, BuilderEdit::SetKind(0, KeyKind::Asc));
        assert_eq!(s.indexes.builder, before);
        let (_, commands) = act(s, IndexesAction::Create);
        let sent = sent_spec(&commands);
        assert_eq!(sent.keys, original.keys);
        let command = sent.create_command("people");
        let index = command.get_array("indexes").unwrap()[0]
            .as_document()
            .unwrap()
            .clone();
        let key = index.get_document("key").unwrap();
        assert_eq!(
            key.keys().collect::<Vec<_>>(),
            original
                .raw
                .get_document("key")
                .unwrap()
                .keys()
                .collect::<Vec<_>>()
        );
        let expected = original.raw.get_document("key").unwrap();
        for (field, value) in expected {
            assert_eq!(key.get(field), Some(value), "{field}");
        }
        for extra in ["bits", "min", "max"] {
            assert_eq!(index.get(extra), original.raw.get(extra), "{extra}");
        }
    }
}

/// A production connection with the drop of `status_1` confirmed in the modal and parked.
fn parked_drop() -> (AppState, Uuid) {
    let (s, id) = browsing(with_flags(true, false));
    let s = with_indexes(s, vec![email(), status()]);
    let (s, _) = act(s, IndexesAction::DropRequested("status_1".into()));
    let (s, _) = act(s, IndexesAction::DropTyped("status_1".into()));
    let (s, commands) = act(s, IndexesAction::DropConfirmed);
    assert!(commands.is_empty(), "nothing is sent before the answer");
    (s, id)
}

#[test]
fn dropping_on_a_production_connection_parks_for_the_production_warning() {
    let (s, id) = parked_drop();
    assert!(s.pending.is_none() && s.indexes.writing.is_none());
    let parked = s.write_warning.as_ref().expect("the write is parked");
    assert_eq!(parked.kind, WriteKind::DropIndex);
    assert_eq!(parked.command.session(), Some(id));
    assert!(parked.command.is_write());
    assert!(s.indexes.drop_confirm.is_some());

    let (confirmed, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(only_drop(&commands), "status_1");
    assert!(matches!(
        confirmed.indexes.writing,
        Some(IndexWrite::Drop { .. })
    ));
    let (s, _) = parked_drop();
    let (cancelled, commands) = update(s, Input::WriteWarning(WriteWarningAction::Cancel));
    assert!(commands.is_empty() && cancelled.indexes.drop_confirm.is_some());
}

#[test]
fn hiding_on_a_production_connection_parks_for_the_production_warning() {
    let (s, _) = browsing(with_flags(true, false));
    let s = with_indexes(s, vec![email(), status()]);
    let (s, commands) = act(s, IndexesAction::SetHidden("status_1".into(), true));
    assert!(commands.is_empty());
    assert_eq!(s.write_warning.as_ref().unwrap().kind, WriteKind::HideIndex);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(commands.len(), 1);
    assert!(matches!(s.indexes.writing, Some(IndexWrite::Hide { .. })));
}

#[test]
fn a_ttl_change_on_a_production_connection_parks_for_the_production_warning() {
    let (s, _) = browsing(with_flags(true, false));
    let s = with_indexes(s, vec![expiring()]);
    let (s, commands) = act(s, IndexesAction::SetTtl("createdAt_1".into(), 60));
    assert!(commands.is_empty());
    assert_eq!(
        s.write_warning.as_ref().unwrap().kind,
        WriteKind::SetIndexTtl
    );
}

/// A read-only connection with the modals of the write flows open, as if it got locked after.
fn locked_with_modals() -> AppState {
    let (s, _) = browsing(with_flags(false, true));
    let s = with_indexes(s, vec![email(), expiring()]);
    let indexes = IndexesState {
        drop_confirm: Some(DropConfirm {
            name: "email_1".into(),
            typed: "email_1".into(),
        }),
        drop_old: Some(DropOld {
            old: "email_1".into(),
            new: "email_1_v2".into(),
        }),
        ..s.indexes.clone()
    };
    AppState { indexes, ..s }
}

/// Every write action of the tab, on a connection that refuses writes: the person is told, and
/// nothing is sent or opened.
pub(super) fn assert_read_only_refuses_every_write() {
    let actions = [
        IndexesAction::EditCopy("email_1".into()),
        IndexesAction::SetHidden("email_1".into(), true),
        IndexesAction::SetHidden("email_1".into(), false),
        IndexesAction::ChangeTtl("createdAt_1".into()),
        IndexesAction::SetTtl("createdAt_1".into(), 60),
        IndexesAction::DropRequested("email_1".into()),
        IndexesAction::DropConfirmed,
        IndexesAction::ConfirmDropOld,
    ];
    for action in actions {
        let (after, commands) = act(locked_with_modals(), action.clone());
        assert!(commands.is_empty(), "{action:?}");
        assert_eq!(after.banner, Some(Banner::ReadOnly), "{action:?}");
        assert!(
            after.pending.is_none() && after.write_warning.is_none(),
            "{action:?}"
        );
        assert!(
            after.indexes.builder.is_none() && after.indexes.ttl_prompt.is_none(),
            "{action:?}"
        );
        assert_eq!(after.indexes.writing, None, "{action:?}");
    }
}

#[test]
fn a_dropped_index_modal_is_closed_by_cancel_and_cannot_be_forged() {
    let (s, _) = on_a_normal_collection();
    // Typing with no modal open does nothing; confirming with no modal sends nothing.
    let (after, commands) = act(s, IndexesAction::DropTyped("email_1".into()));
    assert!(commands.is_empty() && after.indexes.drop_confirm.is_none());
    let (s, _) = on_a_normal_collection();
    let (_, commands) = act(s, IndexesAction::DropConfirmed);
    assert!(commands.is_empty());
    let (s, _) = on_a_normal_collection();
    let (s, _) = act(s, IndexesAction::DropRequested("email_1".into()));
    let (s, _) = act(s, IndexesAction::DropCancelled);
    assert!(s.indexes.drop_confirm.is_none());
}

#[test]
fn copy_as_json_puts_the_definition_on_the_clipboard_action() {
    let geo = IndexSpec {
        raw: doc! { "key": { "loc": "2d" }, "name": "loc_2d", "bits": 26, "v": 2 },
        ..mock_index_spec_named("loc_2d", &[("loc", KeyKind::Geo2d)])
    };
    let json = geo.to_json();
    assert!(json.contains("\"2d\"") && json.contains("bits"), "{json}");
    let action = IndexesAction::CopyJson(json.clone());
    // What the shell puts on the clipboard is exactly the definition, unchanged.
    assert_eq!(action.clipboard_text(), Some(json.as_str()));
    // Nothing else puts anything on the clipboard.
    for other in [
        IndexesAction::Refresh,
        IndexesAction::Select("loc_2d".into()),
        IndexesAction::EditCopy("loc_2d".into()),
        IndexesAction::DropTyped(json.clone()),
        IndexesAction::TtlTyped(json),
    ] {
        assert_eq!(other.clipboard_text(), None, "{other:?}");
    }
    // The reducer neither records it nor sends anything.
    let (s, _) = plain();
    let s = with_indexes(s, vec![geo]);
    let before = s.indexes.clone();
    let (s, commands) = act(s, action);
    assert!(commands.is_empty());
    assert_eq!(s.indexes, before);
}

/// `email_1` copied as `email_1_v2` and parked behind the production warning.
fn parked_copy() -> (AppState, Uuid) {
    let (s, id) = browsing(with_flags(true, false));
    let s = with_indexes(s, vec![email(), status()]);
    let (s, _) = act(s, IndexesAction::EditCopy("email_1".into()));
    let (s, commands) = act(s, IndexesAction::Create);
    assert!(commands.is_empty() && s.write_warning.is_some());
    (s, id)
}

#[test]
fn the_production_confirm_records_the_original_that_was_parked() {
    let (s, id) = parked_copy();
    // Whatever is tried while the warning is up, the parked copy is still the one of email_1.
    let (s, _) = act(s, IndexesAction::CancelBuilder);
    let (s, _) = act(s, IndexesAction::EditCopy("status_1".into()));
    let s = edit(s, BuilderEdit::SetName("email_1_v2".into()));
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(sent_spec(&commands).name, "email_1_v2");
    let (s, _) = changed(s, id);
    assert_eq!(
        s.indexes.drop_old.as_ref().map(|d| d.old.as_str()),
        Some("email_1"),
        "never status_1"
    );
}

#[test]
fn the_builder_cannot_change_while_a_production_warning_is_up() {
    let (s, _) = parked_copy();
    let before = s.indexes.builder.clone();
    for action in [
        IndexesAction::CancelBuilder,
        IndexesAction::EditCopy("status_1".into()),
        IndexesAction::OpenBuilder,
        IndexesAction::Builder(BuilderEdit::SetName("other".into())),
    ] {
        let (after, commands) = act(parked_copy().0, action.clone());
        assert!(commands.is_empty(), "{action:?}");
        assert_eq!(after.indexes.builder, before, "{action:?}");
    }
    assert!(s.write_warning.is_some());
}
