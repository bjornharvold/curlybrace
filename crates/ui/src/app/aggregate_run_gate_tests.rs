//! The Run button's reasons that come from the session around the draft (another write in
//! flight), the count that failed to load, a target whose name has a space at its end, and the
//! availability that is remembered between repaints.
use super::*;
use crate::app::state::Pending;
use crate::app::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::aggregate::Mode;

#[test]
fn another_write_in_flight_turns_run_off_with_its_reason() {
    let mut s = ready();
    s.pending = Some(Pending {
        ns: people(),
        editor: None,
    });
    assert_eq!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::OtherWrite)
    );
    let (after, commands) = act(s, A::RunClicked);
    assert!(commands.is_empty());
    assert!(after.aggregate.run.dialog.is_none());
}

#[test]
fn a_write_warning_waiting_for_its_answer_turns_run_off_with_its_reason() {
    let mut s = ready();
    s.write_warning = Some(ParkedWrite {
        command: Command::EstimatedCount {
            session: EMBEDDED_SESSION,
            ns: people(),
        },
        pending: None,
        kind: WriteKind::AggregateRun,
        epoch: 0,
    });
    assert_eq!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::OtherWrite)
    );
}

#[test]
fn run_is_on_again_once_the_other_write_is_over() {
    let mut s = ready();
    s.pending = Some(Pending {
        ns: people(),
        editor: None,
    });
    s.pending = None;
    assert_eq!(s.run_availability(), RunAvailability::Ready);
}

#[test]
fn a_failed_count_says_it_is_not_available_instead_of_loading() {
    let (s, _) = act(ready(), A::RunClicked);
    assert_eq!(
        s.aggregate.run.dialog.clone().unwrap().count,
        CountState::Loading
    );
    let (s, commands) = ev(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "estimated_count",
            error: CoreError::Unsupported("this store cannot count documents".into()),
        },
    );
    assert!(commands.is_empty());
    assert_eq!(s.banner, None);
    assert_eq!(
        s.aggregate.run.dialog.unwrap().count,
        CountState::Unavailable
    );
}

#[test]
fn a_failed_count_does_not_overwrite_a_known_one_and_needs_no_dialog() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, _) = ev(
        s,
        Event::EstimatedCount {
            session: EMBEDDED_SESSION,
            ns: dest(),
            count: 5,
        },
    );
    let failed = Event::Failed {
        session: EMBEDDED_SESSION,
        context: "estimated_count",
        error: CoreError::Database("late".into()),
    };
    let (s, _) = ev(s, failed.clone());
    assert_eq!(s.aggregate.run.dialog.unwrap().count, CountState::Known(5));
    let (idle, _) = ev(ready(), failed);
    assert!(idle.aggregate.run.dialog.is_none());
}

#[test]
fn run_stays_confirmable_while_the_count_loads() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    assert_eq!(
        s.aggregate.run.dialog.clone().unwrap().count,
        CountState::Loading
    );
    let (s, commands) = act(s, A::RunConfirmed);
    assert_eq!(run_commands(&commands).len(), 1);
    assert!(s.aggregate.run.is_running());
}

#[test]
fn a_target_with_a_space_at_its_end_can_be_confirmed_by_typing_it_either_way() {
    for typed in ["reports ", "reports", " reports  "] {
        let s = drafted(&[MATCH, r#"{ "$out": "reports " }"#]);
        let (s, _) = act(s, A::RunClicked);
        let (s, _) = act(s, A::RunTyped(typed.into()));
        let (_, commands) = act(s, A::RunConfirmed);
        assert_eq!(run_commands(&commands).len(), 1, "{typed:?}");
    }
}

#[test]
fn availability_is_remembered_and_follows_every_input() {
    let s = ready();
    for _ in 0..4 {
        assert_eq!(s.run_availability(), RunAvailability::Ready);
    }
    assert_eq!(s.aggregate.run_shape_evaluations(), 1);

    // An edit changes the pipeline: asked again.
    let (edited, _) = act(ready(), A::Edit(1, r#"{ "$out": 5 }"#.into()));
    assert!(matches!(
        edited.run_availability(),
        RunAvailability::Disabled(RunDisabled::Invalid(_))
    ));
    // A toggle: the write stage no longer ends the pipeline.
    let (toggled, _) = act(ready(), A::Toggle(1));
    assert_eq!(toggled.run_availability(), RunAvailability::Hidden);
    let (back, _) = act(toggled, A::Toggle(1));
    assert_eq!(back.run_availability(), RunAvailability::Ready);
    // Text mode: an edited buffer is not applied yet.
    let (text, _) = act(ready(), A::SetMode(Mode::Text));
    assert_eq!(text.run_availability(), RunAvailability::Ready);
    let (edited_text, _) = act(text, A::EditText("[]".into()));
    assert_eq!(
        edited_text.run_availability(),
        RunAvailability::Disabled(RunDisabled::TextNotApplied)
    );
}

#[test]
fn the_session_around_the_draft_changes_the_answer_without_asking_again() {
    let mut s = ready();
    assert_eq!(s.run_availability(), RunAvailability::Ready);
    s.status = ConnStatus::Disconnected;
    assert_eq!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::NoConnection)
    );
    s.status = ConnStatus::Connected;
    // The bare target follows the source database: in a server database it is refused.
    s.sidebar = s.sidebar.selecting(Namespace::new("admin", "src"));
    assert!(matches!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::Invalid(_))
    ));
    s.sidebar = s.sidebar.selecting(people());
    assert_eq!(s.run_availability(), RunAvailability::Ready);
    assert_eq!(s.aggregate.run_shape_evaluations(), 1);
}
