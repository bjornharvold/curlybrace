//! The production write warning is drawn from `show_modals` for any parked write, whichever
//! session is active, and names the connection the write is aimed at.
use super::host_key::PromptState;
use super::panels::{UiEvent, show_modals};
use super::reducer::Input;
use super::state::{AppState, Pending};
use super::write_guard::{ParkedWrite, WriteKind, WriteWarningAction};
use crate::I18n;
use bson::Bson;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::Command;
use curlybrace_core::test_support::mock_objects::{mock_connection, mock_namespace};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use uuid::Uuid;

struct Run {
    state: AppState,
    events: Vec<UiEvent>,
}

fn named(name: &str, production: bool) -> ConnectionConfig {
    ConnectionConfig {
        name: name.into(),
        id: Uuid::new_v4(),
        production,
        ..mock_connection()
    }
}

/// A warning parked on `prod`, while `other` is the active session.
fn parked_elsewhere() -> AppState {
    let (prod, other) = (named("prod-db", true), named("scratch-db", false));
    let ns = mock_namespace();
    let write_warning = Some(ParkedWrite {
        command: Command::Delete {
            session: prod.id,
            ns: ns.clone(),
            id: Bson::Int32(1),
        },
        pending: Pending { ns, editor: None },
        kind: WriteKind::Delete,
        epoch: 1,
    });
    AppState {
        active: Some(other.id),
        write_warning,
        ..AppState::new(vec![prod, other])
    }
}

fn harness(state: AppState) -> Harness<'static, Run> {
    let mut h = Harness::new_state(
        |ctx, run: &mut Run| {
            let i18n = I18n::english();
            show_modals(
                ctx,
                &mut run.state,
                &PromptState::default(),
                &i18n,
                &mut run.events,
            );
        },
        Run {
            state,
            events: Vec::new(),
        },
    );
    h.run_steps(4);
    h
}

#[test]
fn the_warning_draws_for_a_write_parked_on_a_session_that_is_not_active() {
    let h = harness(parked_elsewhere());
    let text =
        crate::views::write_warning::warning_text(WriteKind::Delete, "prod-db", &I18n::english());
    h.get_by_label(&text);
}

#[test]
fn confirming_emits_the_write_warning_input() {
    let mut h = harness(parked_elsewhere());
    h.get_by_label(&I18n::english().t("write_warning.confirm"))
        .click();
    h.run_steps(4);
    assert!(matches!(
        h.state().events.as_slice(),
        [UiEvent::Input(Input::WriteWarning(
            WriteWarningAction::Confirm
        ))]
    ));
}

#[test]
fn nothing_is_drawn_without_a_parked_write() {
    let state = AppState {
        write_warning: None,
        ..parked_elsewhere()
    };
    let h = harness(state);
    assert!(
        h.query_by_label(&I18n::english().t("write_warning.confirm"))
            .is_none()
    );
}

fn parked_on(state: AppState, session: Uuid) -> AppState {
    let ns = mock_namespace();
    let write_warning = Some(ParkedWrite {
        command: Command::Delete {
            session,
            ns: ns.clone(),
            id: Bson::Int32(1),
        },
        pending: Pending { ns, editor: None },
        kind: WriteKind::Delete,
        epoch: 1,
    });
    AppState {
        write_warning,
        ..state
    }
}

fn opened(state: AppState, id: Uuid) -> AppState {
    use super::reducer::update;
    use curlybrace_core::runtime::Event;
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    let event = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    update(s, Input::Event(event)).0
}

#[test]
fn two_open_connections_sharing_a_name_are_told_apart_in_the_warning() {
    let mut first = ConnectionConfig::new("prod", "mongodb://alpha.example.com:27017");
    first.production = true;
    let mut second = ConnectionConfig::new("prod", "mongodb://beta.example.com:27017");
    second.production = true;
    let (a, b) = (first.id, second.id);
    let both = || {
        opened(
            opened(AppState::new(vec![first.clone(), second.clone()]), a),
            b,
        )
    };
    let i18n = I18n::english();
    let label_a = both().open_label(a, &i18n).unwrap();
    let label_b = both().open_label(b, &i18n).unwrap();
    assert_ne!(label_a, label_b);
    for (session, label, other) in [(a, &label_a, &label_b), (b, &label_b, &label_a)] {
        let h = harness(parked_on(both(), session));
        let text = |l: &str| crate::views::write_warning::warning_text(WriteKind::Delete, l, &i18n);
        h.get_by_label(&text(label));
        assert!(h.query_by_label(&text(other)).is_none());
    }
}

#[test]
fn a_write_parked_on_an_unknown_session_names_the_translated_fallback() {
    let state = parked_on(AppState::new(vec![named("prod-db", true)]), Uuid::new_v4());
    let i18n = I18n::english();
    let unknown = i18n.t("doc.connection_unknown");
    let h = harness(state);
    let text = crate::views::write_warning::warning_text(WriteKind::Delete, &unknown, &i18n);
    h.get_by_label(&text);
}
