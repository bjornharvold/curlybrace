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
