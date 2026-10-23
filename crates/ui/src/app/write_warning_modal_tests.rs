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
