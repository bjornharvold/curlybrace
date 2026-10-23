//! States for the apply-selection tests: one finished pair with rows in every category, scripted
//! through the reducer like a real session. No database, no network.
use bson::{Bson, doc};
use curlybrace_core::compare::diff::{ChangeKind, FieldChange, PathSegment};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareEvent, PairId, PairOutcome, Preflight, RowView, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{CollectionKind, IndexStatus};
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::reducer::{Input, update};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::{ActiveRun, ApplyUi, Pick, RowAction};

pub const PAIR: PairId = 2;
/// Rows per category in the fixture.
pub const DIFFERENT: usize = 3;
pub const ONLY_IN_SOURCE: usize = 2;
pub const ONLY_IN_TARGET: usize = 4;

pub fn key(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

pub fn step(state: AppState, input: Input) -> AppState {
    update(state, input).0
}

pub fn act(state: AppState, action: CompareAction) -> AppState {
    step(state, Input::Compare(action))
}

fn event(state: AppState, e: CompareEvent) -> AppState {
    step(state, Input::Event(Event::Compare(e)))
}

fn picked(state: AppState, side: Side) -> AppState {
    let state = act(
        state,
        CompareAction::PickSide(side, Pick::Database("shop".into())),
    );
    let listing = CompareEvent::Collections {
        session: 1,
        side,
        db: "shop".into(),
        entries: vec![("users".into(), CollectionKind::Collection)],
    };
    let state = event(state, listing);
    act(
        state,
        CompareAction::PickSide(side, Pick::Collection("users".into())),
    )
}

fn checked_and_started(state: AppState) -> AppState {
    let state = act(state, CompareAction::RunCheck);
    let preflight = Preflight {
        source_count: 9,
        target_count: 9,
        source_index: IndexStatus::Indexed,
        target_index: IndexStatus::Indexed,
        unindexed_too_large: false,
    };
    let answer = CompareEvent::Checked {
        session: 1,
        pair: 1,
        preflight,
    };
    act(event(state, answer), CompareAction::Start)
}

fn summary() -> Summary {
    Summary {
        different: DIFFERENT as u64,
        only_in_source: ONLY_IN_SOURCE as u64,
        only_in_target: ONLY_IN_TARGET as u64,
        ..Summary::default()
    }
}

/// Delivers the first page of `category` for the open pair.
pub fn rows_of(state: AppState, category: Category, count: usize) -> AppState {
    let (has_source, has_target) = match category {
        Category::OnlyInSource => (true, false),
        Category::OnlyInTarget => (false, true),
        Category::Different => (true, true),
    };
    let rows = (1..=count as i32)
        .map(|n| RowView {
            key: key(n),
            has_source,
            has_target,
        })
        .collect();
    let page = CompareEvent::Rows {
        session: 1,
        pair: PAIR,
        category,
        offset: 0,
        total: count,
        rows,
    };
    event(state, page)
}

/// A finished pair with its Different tab loaded.
pub fn finished() -> AppState {
    let (a, b) = (
        ConnectionConfig::new("one", "mongodb://a"),
        ConnectionConfig::new("two", "mongodb://b"),
    );
    let open = CompareAction::Open {
        source: a.id,
        target: b.id,
    };
    let state = act(AppState::new(vec![a, b]), open);
    let state = event(state, CompareEvent::Opened { session: 1 });
    let state = act(
        picked(picked(state, Side::Source), Side::Target),
        CompareAction::AddPair,
    );
    let state = checked_and_started(state);
    let done = CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: PAIR,
        outcome: PairOutcome::Completed(summary()),
    };
    let state = event(state, done);
    let state = event(state, CompareEvent::RunDone { session: 1, run: 1 });
    rows_of(state, Category::Different, DIFFERENT)
}

/// The finished pair showing `category`.
pub fn in_category(category: Category) -> AppState {
    let count = match category {
        Category::Different => DIFFERENT,
        Category::OnlyInSource => ONLY_IN_SOURCE,
        Category::OnlyInTarget => ONLY_IN_TARGET,
    };
    let state = act(finished(), CompareAction::SelectCategory(category));
    rows_of(state, category, count)
}

/// Key 1 of the open tab opened in the detail pane.
pub fn with_detail(state: AppState) -> AppState {
    let state = act(state, CompareAction::Select(key(1)));
    let change = FieldChange {
        path: "name".into(),
        segments: vec![PathSegment::Field("name".into())],
        kind: ChangeKind::Changed,
        source: Some(Bson::String("x".into())),
        target: Some(Bson::String("y".into())),
    };
    let detail = CompareEvent::Detail {
        session: 1,
        pair: PAIR,
        key: key(1),
        source: Some(doc! {"_id": 1, "name": "x"}),
        target: Some(doc! {"_id": 1, "name": "y"}),
        changes: vec![change],
    };
    event(state, detail)
}

/// An apply wrote to the pair since it ran.
pub fn applied(state: AppState) -> AppState {
    let overview = state.compare.overview.clone().marked_applied(PAIR);
    let compare = curlybrace_ui::viewmodel::compare::CompareState {
        overview,
        ..state.compare.clone()
    };
    AppState { compare, ..state }
}

/// Another compare run is in flight.
pub fn run_active(state: AppState) -> AppState {
    let active = ActiveRun {
        run: 5,
        cancel_requested: false,
        failure_noted: false,
        pairs: 1,
    };
    let mut compare = state.compare.clone();
    compare.overview.active = Some(active);
    AppState { compare, ..state }
}

/// A check is in flight.
pub fn check_out(state: AppState) -> AppState {
    let state = act(state, CompareAction::RunCheck);
    assert!(state.compare.check_in_flight(), "the fixture sent a check");
    state
}

/// An apply is running (the lists are read-only).
pub fn applying(state: AppState) -> AppState {
    let apply = ApplyUi::Running {
        run: 9,
        total: 1,
        rows: Vec::new(),
        cancel_requested: false,
    };
    let compare = curlybrace_ui::viewmodel::compare::CompareState {
        apply,
        ..state.compare.clone()
    };
    AppState { compare, ..state }
}

/// The localized confirmation word the tests open reviews with.
pub const WORD: &str = "delete";

/// Rows 1 and 2 of `category`'s tab ticked and queued with `action`.
pub fn queued(category: Category, action: RowAction) -> AppState {
    let state = in_category(category);
    let state = act(state, CompareAction::ToggleRow(key(1)));
    let state = act(state, CompareAction::ToggleRow(key(2)));
    let state = act(state, CompareAction::AddToPending(action));
    assert_eq!(state.compare.pending.items().len(), 1, "queued");
    state
}

/// Two Different rows queued to copy to the target (a replace).
pub fn with_replace() -> AppState {
    queued(Category::Different, RowAction::CopyToTarget)
}

/// Two only-in-source rows queued to copy to the target (an insert).
pub fn with_insert() -> AppState {
    queued(Category::OnlyInSource, RowAction::CopyToTarget)
}

/// Two only-in-target rows queued for deletion from the target (`two / shop.users`).
pub fn with_delete() -> AppState {
    queued(Category::OnlyInTarget, RowAction::DeleteFromTarget)
}

/// Deletes on both connections: the review asks for the localized word.
pub fn with_deletes_on_both_sides() -> AppState {
    let state = queued(Category::OnlyInSource, RowAction::DeleteFromSource);
    let state = act(state, CompareAction::SelectCategory(Category::OnlyInTarget));
    let state = rows_of(state, Category::OnlyInTarget, ONLY_IN_TARGET);
    let state = act(state, CompareAction::ToggleRow(key(1)));
    act(
        state,
        CompareAction::AddToPending(RowAction::DeleteFromTarget),
    )
}

/// The review dialog open for the pending list.
pub fn reviewing(state: AppState) -> AppState {
    let state = act(state, CompareAction::ReviewApply { word: WORD.into() });
    assert!(state.compare.apply.review().is_some(), "review is open");
    state
}

/// The apply stands in `apply`.
pub fn with_apply(state: AppState, apply: ApplyUi) -> AppState {
    let compare = curlybrace_ui::viewmodel::compare::CompareState {
        apply,
        ..state.compare.clone()
    };
    AppState { compare, ..state }
}

// ---------------------------------------------------------------------------------------------
// Screenshot states: three finished pairs with differences, scripted through the reducer. These
// follow the `&mut AppState` setup style of `compare_multi_states.rs`.
// ---------------------------------------------------------------------------------------------

/// Builders for the apply-flow screenshots.
pub mod shots {
    use super::{CompareEvent, RowView};
    use crate::support::compare_multi_states as multi;
    use crate::support::compare_states as cs;
    use curlybrace_core::compare::apply::{ApplyReport, IdPolicy, OpFailure};
    use curlybrace_core::compare::protocol::{PairOutcome, RunId};
    use curlybrace_core::compare::results::Category;
    use curlybrace_core::error::CoreError;
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::state::AppState;
    use curlybrace_ui::viewmodel::compare::{ApplyUi, RowAction};

    const SESSION: u64 = cs::SESSION;
    const RUN: u64 = 1;
    /// The confirmation word the screenshots review with.
    const WORD: &str = "delete";

    pub fn page(state: &mut AppState, entry: u32, category: Category, keys: &[i32]) {
        let rows = keys
            .iter()
            .map(|n| RowView {
                key: cs::key(*n),
                has_source: category != Category::OnlyInTarget,
                has_target: category != Category::OnlyInSource,
            })
            .collect();
        let event = CompareEvent::Rows {
            session: SESSION,
            pair: multi::pair_of(state, entry),
            category,
            offset: 0,
            total: keys.len(),
            rows,
        };
        cs::event(state, event);
    }

    /// Three pairs finished: `customers` has 3 different rows, `invoices` 4 only in the target and
    /// 1 only in the source, `orders` is equal.
    fn finished_run(state: &mut AppState) {
        multi::open_with(state, &multi::COLLECTIONS[..3], &multi::COLLECTIONS[..3]);
        cs::act(state, CompareAction::SetKey("_id".into()));
        cs::act(state, CompareAction::RunCheck);
        for _ in 0..3 {
            let ok = multi::preflight(
                curlybrace_core::store::IndexStatus::Indexed,
                curlybrace_core::store::IndexStatus::Indexed,
                false,
            );
            multi::answer_check(state, ok);
        }
        cs::act(state, CompareAction::Start);
        multi::finish(
            state,
            0,
            PairOutcome::Completed(multi::summary(900, 0, 0, 3)),
        );
        multi::finish(
            state,
            1,
            PairOutcome::Completed(multi::summary(4_700, 1, 4, 0)),
        );
        multi::finish(
            state,
            2,
            PairOutcome::Completed(multi::summary(120, 0, 0, 0)),
        );
        cs::event(
            state,
            CompareEvent::RunDone {
                session: SESSION,
                run: RUN,
            },
        );
    }

    /// The first pair's Different tab is open.
    pub fn customers_open(state: &mut AppState) {
        finished_run(state);
        cs::act(state, CompareAction::OpenPair(0));
        page(state, 0, Category::Different, &[17, 42, 108]);
    }

    /// Rows 17 and 42 of `customers` ticked, so the selection bar and its buttons show.
    pub fn selection_in_results(state: &mut AppState) {
        customers_open(state);
        cs::act(state, CompareAction::ToggleRow(cs::key(17)));
        cs::act(state, CompareAction::ToggleRow(cs::key(42)));
    }

    /// `customers`: two explicit keys copied to the target; `invoices`: the whole only-in-target
    /// tab deleted from the target. Back on the overview.
    pub fn pending_panel(state: &mut AppState) {
        selection_in_results(state);
        cs::act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
        cs::act(state, CompareAction::BackToOverview);
        cs::act(state, CompareAction::OpenPair(1));
        cs::act(state, CompareAction::SelectCategory(Category::OnlyInTarget));
        page(state, 1, Category::OnlyInTarget, &[1, 2, 3, 4]);
        cs::act(state, CompareAction::SelectAllInCategory);
        cs::act(
            state,
            CompareAction::AddToPending(RowAction::DeleteFromTarget),
        );
        // The one source-only invoice is inserted into the target: ids are kept or generated.
        cs::act(state, CompareAction::SelectCategory(Category::OnlyInSource));
        page(state, 1, Category::OnlyInSource, &[9]);
        cs::act(state, CompareAction::ToggleRow(cs::key(9)));
        cs::act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
        cs::act(state, CompareAction::BackToOverview);
    }

    /// The review dialog over the pending list, with a delete waiting for its typed name.
    pub fn review_with_delete(state: &mut AppState) {
        pending_panel(state);
        cs::act(state, CompareAction::ReviewApply { word: WORD.into() });
        cs::act(state, CompareAction::SetIdPolicy(IdPolicy::Generate));
        cs::act(state, CompareAction::SetConfirmText("invo".into()));
        assert!(state.compare.apply.review().is_some(), "the review is open");
    }

    /// [`review_with_delete`] with the destination connection flagged as production: the review
    /// shows the production heading and its badge.
    pub fn review_with_delete_production(state: &mut AppState) {
        pending_panel(state);
        state.compare.connections_production = (false, true);
        cs::act(state, CompareAction::ReviewApply { word: WORD.into() });
        cs::act(state, CompareAction::SetIdPolicy(IdPolicy::Generate));
        cs::act(state, CompareAction::SetConfirmText("invo".into()));
        assert!(state.compare.apply.review().is_some(), "the review is open");
    }

    fn confirmed(state: &mut AppState) -> RunId {
        pending_panel(state);
        confirm_pending(state)
    }

    /// Reviews and confirms the pending list (typing the required word, if any).
    pub fn confirm_pending(state: &mut AppState) -> RunId {
        cs::act(state, CompareAction::ReviewApply { word: WORD.into() });
        let review = state.compare.apply.review().expect("review").clone();
        let text = review.required_text(WORD).unwrap_or_default().to_string();
        cs::act(state, CompareAction::SetConfirmText(text));
        cs::act(state, CompareAction::ConfirmApply);
        match &state.compare.apply {
            ApplyUi::Running { run, .. } => *run,
            other => panic!("the apply did not start: {other:?}"),
        }
    }

    fn progress(state: &mut AppState, run: RunId, entry: u32, done: u64, total: u64) {
        let event = CompareEvent::ApplyProgress {
            session: SESSION,
            run,
            pair: multi::pair_of(state, entry),
            done,
            total,
        };
        cs::event(state, event);
    }

    /// Two pairs part way through their writes.
    pub fn apply_running(state: &mut AppState) {
        let run = confirmed(state);
        progress(state, run, 0, 1, 2);
        progress(state, run, 1, 2, 5);
    }

    fn failure(n: i32, error: CoreError) -> OpFailure {
        OpFailure {
            key: cs::key(n),
            error,
        }
    }

    pub fn report_of(state: &mut AppState, run: RunId, entry: u32, report: ApplyReport) {
        let event = CompareEvent::ApplyPairDone {
            session: SESSION,
            run,
            pair: multi::pair_of(state, entry),
            report,
        };
        cs::event(state, event);
    }

    /// Both pairs reported: 2 applied, 1 stale, 2 failed (with a failure list), 1 never ran.
    pub fn apply_finished_mixed(state: &mut AppState) {
        let run = confirmed(state);
        let first = ApplyReport {
            applied: 1,
            stale: 1,
            ..ApplyReport::default()
        };
        report_of(state, run, 0, first);
        let second = ApplyReport {
            applied: 1,
            failed: 2,
            failures: vec![
                failure(2, CoreError::NotFound),
                failure(3, CoreError::ApplyOutcomeUnknown),
            ],
            ..ApplyReport::default()
        };
        report_of(state, run, 1, second);
        cs::event(
            state,
            CompareEvent::ApplyDone {
                session: SESSION,
                run,
            },
        );
    }

    /// The apply result dismissed: the overview rows of the written pairs carry the marker.
    pub fn pair_marked_applied(state: &mut AppState) {
        apply_finished_mixed(state);
        cs::act(state, CompareAction::DismissApplyResult);
    }

    /// The same, with one written pair open: its header carries the marker.
    pub fn pair_header_marked_applied(state: &mut AppState) {
        pair_marked_applied(state);
        cs::act(state, CompareAction::OpenPair(0));
        page(state, 0, Category::Different, &[17, 42, 108]);
    }
}
