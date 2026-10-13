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
use curlybrace_ui::viewmodel::compare::{ActiveRun, ApplyUi, Pick};

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
