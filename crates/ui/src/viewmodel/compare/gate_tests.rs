use super::*;
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::protocol::{PairOutcome, Preflight, Side};
use curlybrace_core::store::{CollectionKind, IndexStatus, Namespace};

fn picked(db: &str) -> SideForm {
    SideForm::default()
        .with_database(db)
        .with_collections(db, vec![("c".into(), CollectionKind::Collection)])
        .with_collection("c")
}

/// The id of the first entry a list hands out.
const ENTRY: EntryId = 0;

/// The state with `setup` as its form and the one pair its pickers describe as the only entry.
fn with_setup(state: CompareState, setup: SetupForm) -> CompareState {
    let pairs = match setup.pick_pair() {
        Ok((source, target)) => PairList::default().added(source, target),
        Err(_) => PairList::default(),
    };
    CompareState {
        setup,
        pairs,
        ..state
    }
}

fn ready() -> CompareState {
    let setup = SetupForm::default()
        .with_side(Side::Source, picked("a"))
        .with_side(Side::Target, picked("b"));
    CompareState {
        session: Some(SessionPhase::Ready(1)),
        ..with_setup(CompareState::default(), setup)
    }
}

fn pre(count: u64, indexed: bool) -> Preflight {
    let index = if indexed {
        IndexStatus::Indexed
    } else {
        IndexStatus::NotIndexed
    };
    Preflight {
        source_count: count,
        target_count: count,
        source_index: index,
        target_index: index,
        unindexed_too_large: !indexed,
    }
}

/// The entry checked with `count` documents on each side.
fn checked(state: CompareState, count: u64, indexed: bool) -> CompareState {
    let checks = CheckQueue::queued(&[ENTRY])
        .sending(ENTRY, 5)
        .answered(5, pre(count, indexed))
        .0;
    CompareState { checks, ..state }
}

fn running(state: CompareState) -> CompareState {
    let overview = RunOverview::default().started(1, &[(ENTRY, 9)]).0;
    CompareState { overview, ..state }
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
        checks: CheckQueue::queued(&[ENTRY]).sending(ENTRY, 5),
        ..ready()
    };
    assert!(!in_flight.can_check());
    let incomplete = with_setup(ready(), SetupForm::default());
    assert!(!incomplete.can_check());
    assert!(!running(ready()).can_check());
}

#[test]
fn start_additionally_needs_an_ok_fresh_preflight() {
    assert!(!ready().can_start(), "unchecked");
    for count in [5, 0] {
        let state = checked(ready(), count, true);
        assert!(state.can_start());
        let edited = CompareState {
            checks: state.checks.clone().invalidated(),
            ..state.clone()
        };
        assert!(!edited.can_start());
        let in_flight = CompareState {
            checks: CheckQueue::queued(&[ENTRY]).sending(ENTRY, 6),
            ..state.clone()
        };
        assert!(!in_flight.can_start());
        assert!(!running(state).can_start());
    }
    assert!(!checked(ready(), 5, false).can_start(), "blocked");
}

#[test]
fn start_needs_an_included_entry_and_every_included_entry_to_pass() {
    let state = checked(ready(), 5, true);
    let none_included = CompareState {
        pairs: state.pairs.clone().included(ENTRY, false),
        ..state.clone()
    };
    assert!(!none_included.can_start() && !none_included.can_start_passing());
    let refused = checked(ready(), 5, false);
    assert!(!refused.can_start() && !refused.can_start_passing());
    assert!(state.can_start() && state.can_start_passing());
}

#[test]
fn run_passing_skips_refused_entries_but_still_waits_for_every_check() {
    let state = checked(ready(), 5, true);
    let second = Namespace::new("a", "other");
    let target = Namespace::new("b", "other");
    let pairs = state.pairs.clone().added(second, target);
    let second_id = pairs.entries[1].id;
    let unchecked = CompareState {
        pairs: pairs.clone(),
        ..state.clone()
    };
    assert!(!unchecked.can_start_passing(), "one entry is not settled");
    let (checks, _) = CheckQueue::queued(&[ENTRY, second_id])
        .sending(ENTRY, 5)
        .answered(5, pre(5, true))
        .0
        .sending(second_id, 6)
        .answered(6, pre(9, false));
    let mixed = CompareState {
        pairs,
        checks,
        ..state
    };
    assert!(!mixed.can_start(), "one entry is refused");
    assert!(mixed.can_start_passing());
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
    let overview = RunOverview::default()
        .started(1, &[(ENTRY, 9)])
        .0
        .pair_done(1, 9, PairOutcome::Completed(Summary::default()))
        .run_done(1);
    let finished = CompareState {
        overview,
        ..ready()
    };
    assert!(finished.can_discard());
    assert!(!running(finished.clone()).can_discard());
    let not_ready = CompareState {
        session: Some(SessionPhase::Opening(1)),
        ..finished
    };
    assert!(!not_ready.can_discard());
}

fn on_one_connection(state: CompareState) -> CompareState {
    let id = Uuid::new_v4();
    CompareState {
        connections: Some((id, id)),
        ..state
    }
}

fn same_collection() -> CompareState {
    let side = Side::Target;
    let state = ready();
    let setup = state.setup.clone().with_side(side, picked("a"));
    on_one_connection(with_setup(state, setup))
}

#[test]
fn the_same_collection_with_the_same_filter_on_both_sides_is_refused() {
    let state = same_collection();
    assert_eq!(state.issues(), vec![SetupError::SameCollection]);
    assert_eq!(
        state.spec_for(ENTRY, 1).and_then(Result::err),
        Some(SetupError::SameCollection)
    );
    assert!(!state.can_check() && !state.can_start());
    assert_eq!(
        SetupError::SameCollection.i18n_key(),
        "compare.setup.same_collection"
    );
}

#[test]
fn the_same_collection_differs_by_connection_or_filter() {
    let side = Side::Target;
    let state = same_collection();
    let two_servers = CompareState {
        connections: Some((Uuid::new_v4(), Uuid::new_v4())),
        ..state.clone()
    };
    assert!(two_servers.issues().is_empty() && two_servers.can_check());
    let filtered = state.setup.clone().with_side(
        side,
        state.setup.side(side).clone().with_filter("{\"x\": 1}"),
    );
    let filtered = CompareState {
        setup: filtered,
        ..state.clone()
    };
    assert!(filtered.issues().is_empty() && filtered.can_check());
    // Equal filters written differently are still the same filter.
    let spaced = state
        .setup
        .clone()
        .with_side(side, state.setup.side(side).clone().with_filter("  { }  "));
    let spaced = CompareState {
        setup: spaced,
        ..state.clone()
    };
    assert_eq!(spaced.issues(), vec![SetupError::SameCollection]);
    // Two databases on one connection are fine.
    assert!(on_one_connection(ready()).issues().is_empty());
}

#[test]
fn gates_agree_with_the_single_button_questions() {
    for state in [
        ready(),
        checked(ready(), 5, true),
        checked(ready(), 5, false),
        running(ready()),
    ] {
        let gates = state.gates(&state.digest());
        assert_eq!(gates.check, state.can_check());
        assert_eq!(gates.start, state.can_start());
        assert_eq!(gates.start_passing, state.can_start_passing());
        assert_eq!(gates.cancel, state.can_cancel());
    }
}

#[test]
fn some_not_passing_is_set_only_when_an_included_entry_was_checked_and_refused() {
    let gates = |s: &CompareState| s.gates(&s.digest()).some_not_passing;
    assert!(!gates(&ready()), "unchecked is not refused");
    assert!(!gates(&checked(ready(), 5, true)));
    let refused = checked(ready(), 5, false);
    assert!(gates(&refused));
    let excluded = CompareState {
        pairs: refused.pairs.clone().included(ENTRY, false),
        ..refused
    };
    assert!(
        !gates(&excluded),
        "an excluded entry is not part of the run"
    );
}

// ---- Run semantics: every VALID included entry must pass; the others are skipped ----

fn ns(db: &str, c: &str) -> Namespace {
    Namespace::new(db.to_string(), c.to_string())
}

/// Entries 0 and 2 build, entry 1 has an invalid key; all three included.
fn with_an_invalid_entry() -> CompareState {
    let base = ready();
    let pairs = PairList::default()
        .added(ns("a", "x"), ns("b", "x"))
        .added(ns("a", "y"), ns("b", "y"))
        .added(ns("a", "z"), ns("b", "z"))
        .with_overrides(
            1,
            PairOverrides {
                key_text: Some("a,,b".into()),
                ..PairOverrides::default()
            },
        );
    CompareState { pairs, ..base }
}

/// Answers the queued checks one by one: `outcomes[i]` is the preflight of the i-th checkable
/// entry, `None` for a failed check.
fn answered(state: CompareState, ids: &[EntryId], outcomes: &[Option<Preflight>]) -> CompareState {
    let mut checks = CheckQueue::queued(ids);
    for (i, (id, outcome)) in ids.iter().zip(outcomes).enumerate() {
        let probe = 100 + i as u32;
        checks = checks.sending(*id, probe);
        checks = match outcome {
            Some(p) => checks.answered(probe, p.clone()).0,
            None => checks.failed(probe),
        };
    }
    CompareState { checks, ..state }
}

#[test]
fn an_invalid_entry_does_not_block_run_for_the_valid_ones() {
    let state = answered(
        with_an_invalid_entry(),
        &[0, 2],
        &[Some(pre(5, true)), Some(pre(5, true))],
    );
    let gates = state.gates(&state.digest());
    assert!(gates.start, "Run covers the valid entries only");
    assert!(gates.start_passing);
    assert!(
        gates.some_not_passing,
        "the invalid one is skipped and said so"
    );
}

#[test]
fn a_failed_check_does_not_block_run_passing_and_is_offered_as_skipped() {
    let state = answered(
        with_an_invalid_entry(),
        &[0, 2],
        &[Some(pre(5, true)), None],
    );
    let gates = state.gates(&state.digest());
    assert!(
        !gates.start,
        "a valid entry whose check failed did not pass"
    );
    assert!(gates.start_passing);
    assert!(gates.some_not_passing);
}

#[test]
fn a_refused_entry_blocks_run_but_not_run_passing() {
    let state = answered(
        with_an_invalid_entry(),
        &[0, 2],
        &[Some(pre(5, true)), Some(pre(5, false))],
    );
    let gates = state.gates(&state.digest());
    assert!(!gates.start && gates.start_passing && gates.some_not_passing);
}

#[test]
fn nothing_is_not_passing_when_every_included_entry_passed() {
    let state = checked(ready(), 5, true);
    assert!(!state.gates(&state.digest()).some_not_passing);
}

#[test]
fn all_entries_invalid_enables_neither_run() {
    let defaults = SetupForm::default().with_key_text("a,,b");
    let state = CompareState {
        setup: defaults,
        ..with_an_invalid_entry()
    };
    let gates = state.gates(&state.digest());
    assert!(!gates.start && !gates.start_passing);
}

// ---- Remove ----

#[test]
fn remove_waits_for_runs_checks_and_live_rows() {
    let idle = checked(ready(), 5, true);
    assert!(idle.can_remove(ENTRY));
    assert!(!idle.can_remove(99), "unknown entry");
    assert!(!running(idle.clone()).can_remove(ENTRY));
    let queued = CompareState {
        checks: CheckQueue::queued(&[ENTRY]),
        ..idle.clone()
    };
    assert!(!queued.can_remove(ENTRY));
    let sent = CompareState {
        checks: CheckQueue::queued(&[ENTRY]).sending(ENTRY, 7),
        ..idle
    };
    assert!(!sent.can_remove(ENTRY));
}
