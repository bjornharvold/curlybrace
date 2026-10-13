//! Multi-pair Compare states for the screenshot tests: the pair list, a mixed run overview and
//! one pair's results. Reached through the reducer like a real session; no database.
use super::compare_states::{SESSION, act, event, key, names, open_session};
use super::shots::browsing;
use bson::Bson;
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareEvent, DuplicateReport, PairOutcome, Preflight, RowView, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{CollectionKind, DuplicateGroup, IndexStatus};
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::state::{AppState, Workspace};
use curlybrace_ui::viewmodel::ViewMode;
use curlybrace_ui::viewmodel::compare::{DuplicatesView, EntryId, PairOverrides, Pick};

const RUN: u64 = 1;
const COLLECTIONS: [&str; 6] = [
    "customers",
    "invoices",
    "orders",
    "payments",
    "refunds",
    "shipments",
];

fn entries(list: &[&str]) -> Vec<(String, CollectionKind)> {
    list.iter()
        .map(|n| (n.to_string(), CollectionKind::Collection))
        .collect()
}

/// Both sides on database `crm`; each side lists its own collections.
fn list_side(state: &mut AppState, side: Side, collections: &[&str]) {
    act(
        state,
        CompareAction::PickSide(side, Pick::Database("crm".into())),
    );
    let listing = CompareEvent::Collections {
        session: SESSION,
        side,
        db: "crm".into(),
        entries: entries(collections),
    };
    event(state, listing);
}

fn open_with(state: &mut AppState, source: &[&str], target: &[&str]) {
    browsing(ViewMode::Tree)(state);
    state.workspace = Workspace::Compare;
    open_session(state);
    let databases = names(&["billing", "crm"]);
    for side in [Side::Source, Side::Target] {
        let listing = CompareEvent::Databases {
            session: SESSION,
            side,
            names: databases.clone(),
        };
        event(state, listing);
    }
    list_side(state, Side::Source, source);
    list_side(state, Side::Target, target);
    act(state, CompareAction::AddAllSameNamed);
}

fn preflight(source: IndexStatus, target: IndexStatus, too_large: bool) -> Preflight {
    Preflight {
        source_count: 48_211,
        target_count: 48_090,
        source_index: source,
        target_index: target,
        unindexed_too_large: too_large,
    }
}

/// Answers the check in flight with `preflight`.
fn answer_check(state: &mut AppState, preflight: Preflight) {
    let (_, probe) = state.compare.checks.in_flight().expect("a check is out");
    let answer = CompareEvent::Checked {
        session: SESSION,
        pair: probe,
        preflight,
    };
    event(state, answer);
}

fn summary(equal: u64, only_source: u64, only_target: u64, different: u64) -> Summary {
    Summary {
        equal,
        only_in_source: only_source,
        only_in_target: only_target,
        different,
        scanned_source: equal + only_source + different,
        scanned_target: equal + only_target + different,
    }
}

/// The pair id the run gave `entry`.
fn pair_of(state: &AppState, entry: EntryId) -> u32 {
    state
        .compare
        .overview
        .row_for(entry)
        .expect("the entry runs")
        .pair
}

fn finish(state: &mut AppState, entry: EntryId, outcome: PairOutcome) {
    let done = CompareEvent::PairDone {
        session: SESSION,
        run: RUN,
        pair: pair_of(state, entry),
        outcome,
    };
    event(state, done);
}

/// Defaults, three pairs (one with its own key), the two collections that have no counterpart,
/// and the checks answered: ok, needs an index, ok.
pub fn pairs_setup(state: &mut AppState) {
    let source = ["customers", "invoices", "orders", "legacy_users"];
    let target = ["customers", "invoices", "orders", "archive"];
    open_with(state, &source, &target);
    act(state, CompareAction::SetKey("account_id".into()));
    let own_key = PairOverrides::default().with_key_text(Some("invoice_no".into()));
    act(state, CompareAction::SetPairOverrides(1, own_key));
    act(state, CompareAction::RunCheck);
    answer_check(
        state,
        preflight(IndexStatus::Indexed, IndexStatus::Indexed, false),
    );
    answer_check(
        state,
        preflight(IndexStatus::Indexed, IndexStatus::NotIndexed, true),
    );
    answer_check(
        state,
        preflight(IndexStatus::Indexed, IndexStatus::Indexed, false),
    );
}

/// All six pairs checked and started.
fn started(state: &mut AppState) {
    open_with(state, &COLLECTIONS, &COLLECTIONS);
    act(state, CompareAction::SetKey("_id".into()));
    act(state, CompareAction::RunCheck);
    for _ in COLLECTIONS {
        answer_check(
            state,
            preflight(IndexStatus::Indexed, IndexStatus::Indexed, false),
        );
    }
    act(state, CompareAction::Start);
}

fn group(email: &str, count: u64) -> DuplicateGroup {
    DuplicateGroup {
        key: KeyTuple(vec![Bson::String(email.into())]),
        count,
    }
}

/// A run part way: completed, discarded, failed on a duplicate key (with its panel), cancelled
/// and one still running. (A real run cannot offer the panel while running; it is installed
/// directly so one picture shows every state.)
pub fn overview_mixed(state: &mut AppState) {
    started(state);
    finish(state, 0, PairOutcome::Completed(summary(48_000, 0, 0, 211)));
    finish(state, 1, PairOutcome::Completed(summary(900, 3, 2, 0)));
    act(state, CompareAction::DiscardPair(1));
    finish(
        state,
        2,
        PairOutcome::Failed(CoreError::CompareDuplicateKey),
    );
    finish(state, 3, PairOutcome::Cancelled(summary(100, 0, 0, 1)));
    let progress = CompareEvent::Progress {
        session: SESSION,
        run: RUN,
        pair: pair_of(state, 4),
        scanned_source: 21_000,
        scanned_target: 20_500,
        differences: 4,
    };
    event(state, progress);
    state.compare.duplicates = DuplicatesView::Loaded {
        entry: 2,
        source: DuplicateReport {
            groups: vec![group("ada@example.com", 12), group("grace@example.com", 3)],
            more: false,
        },
        target: DuplicateReport {
            groups: vec![],
            more: false,
        },
    };
}

/// Three pairs done; the second is opened with its first page of differences.
pub fn pair_results(state: &mut AppState) {
    open_with(state, &COLLECTIONS[..3], &COLLECTIONS[..3]);
    act(state, CompareAction::SetKey("_id".into()));
    act(state, CompareAction::RunCheck);
    for _ in 0..3 {
        answer_check(
            state,
            preflight(IndexStatus::Indexed, IndexStatus::Indexed, false),
        );
    }
    act(state, CompareAction::Start);
    finish(state, 0, PairOutcome::Completed(summary(900, 0, 0, 0)));
    finish(state, 1, PairOutcome::Completed(summary(4_700, 3, 1, 2)));
    finish(state, 2, PairOutcome::Completed(summary(120, 0, 0, 0)));
    let done = CompareEvent::RunDone {
        session: SESSION,
        run: RUN,
    };
    event(state, done);
    act(state, CompareAction::OpenPair(1));
    let rows = CompareEvent::Rows {
        session: SESSION,
        pair: pair_of(state, 1),
        category: Category::Different,
        offset: 0,
        total: 2,
        rows: [17, 42]
            .map(|n| RowView {
                key: key(n),
                has_source: true,
                has_target: true,
            })
            .to_vec(),
    };
    event(state, rows);
}
