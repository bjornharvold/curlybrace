use super::*;
use curlybrace_core::store::CollectionKind;

fn picked(db: &str) -> SideForm {
    SideForm::default()
        .with_database(db)
        .with_collections(db, vec![("c".into(), CollectionKind::Collection)])
        .with_collection("c")
}

fn ready() -> CompareState {
    CompareState {
        session: Some(SessionPhase::Ready(1)),
        setup: SetupForm::default()
            .with_side(
                curlybrace_core::compare::protocol::Side::Source,
                picked("a"),
            )
            .with_side(
                curlybrace_core::compare::protocol::Side::Target,
                picked("b"),
            ),
        ..CompareState::default()
    }
}

#[test]
fn check_needs_a_ready_session_a_ready_form_and_nothing_in_flight() {
    assert!(ready().can_check());
    assert!(!CompareState::default().can_check());
    let opening = CompareState {
        session: Some(SessionPhase::Opening(1)),
        ..ready()
    };
    assert!(!opening.can_check());
    let in_flight = CompareState {
        check_in_flight: true,
        ..ready()
    };
    assert!(!in_flight.can_check());
    let incomplete = CompareState {
        setup: SetupForm::default(),
        ..ready()
    };
    assert!(!incomplete.can_check());
    let running = CompareState {
        run: RunView::started(1),
        ..ready()
    };
    assert!(!running.can_check());
}

#[test]
fn start_additionally_needs_an_ok_fresh_preflight() {
    assert!(!ready().can_start(), "unchecked");
    for ok in [PreflightView::Ok, PreflightView::EmptyBoth] {
        let state = CompareState {
            preflight: ok,
            ..ready()
        };
        assert!(state.can_start());
        let stale = CompareState {
            check_stale: true,
            ..state.clone()
        };
        assert!(!stale.can_start());
        let in_flight = CompareState {
            check_in_flight: true,
            ..state.clone()
        };
        assert!(!in_flight.can_start());
        let running = CompareState {
            run: RunView::started(1),
            ..state
        };
        assert!(!running.can_start());
    }
    let blocked = CompareState {
        preflight: PreflightView::NeedsIndex {
            side: curlybrace_core::compare::protocol::Side::Source,
        },
        ..ready()
    };
    assert!(!blocked.can_start());
}

#[test]
fn open_needs_both_picks_a_change_and_no_open_in_progress() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let fresh = CompareState::default();
    assert!(fresh.can_open((Some(a), Some(b))));
    assert!(!fresh.can_open((Some(a), None)));
    assert!(!fresh.can_open((None, Some(b))));
    let open = CompareState {
        connections: Some((a, b)),
        ..ready()
    };
    assert!(!open.can_open((Some(a), Some(b))), "nothing changed");
    assert!(open.can_open((Some(b), Some(b))));
    let opening = CompareState {
        session: Some(SessionPhase::Opening(1)),
        connections: Some((a, b)),
        ..CompareState::default()
    };
    assert!(!opening.can_open((Some(b), Some(a))));
}

#[test]
fn close_and_discard_follow_the_session_and_the_run() {
    assert!(!CompareState::default().can_close());
    assert!(ready().can_close());
    assert!(!ready().can_discard(), "nothing to discard");
    let finished = CompareState {
        run: RunView::Finished {
            run: 1,
            cancelled: false,
            failure: None,
        },
        ..ready()
    };
    assert!(finished.can_discard());
    let running = CompareState {
        run: RunView::started(1),
        ..ready()
    };
    assert!(!running.can_discard());
    let not_ready = CompareState {
        session: Some(SessionPhase::Opening(1)),
        ..finished
    };
    assert!(!not_ready.can_discard());
}
