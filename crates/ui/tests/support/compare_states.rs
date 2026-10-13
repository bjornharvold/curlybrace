//! Fixed Compare workspace states for the screenshot tests, reached by feeding the reducer the
//! same actions and runtime events a real session would produce. No database, no network.
use super::shots::browsing;
use bson::{Bson, doc};
use curlybrace_core::compare::diff::{ChangeKind, FieldChange};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareEvent, DuplicateReport, PairOutcome, Preflight, RowView, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{CollectionKind, DuplicateGroup, IndexStatus};
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::reducer::{Input, update};
use curlybrace_ui::app::state::{AppState, Workspace};
use curlybrace_ui::viewmodel::ViewMode;
use curlybrace_ui::viewmodel::compare::Pick;

const SESSION: u64 = 1;
const RUN: u64 = 1;
const PAIR: u32 = 1;

/// How far the scripted session goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Through {
    Empty,
    Blocked,
    Running,
    Results,
    Detail,
    /// A run that failed on a duplicate key, then the Find duplicate keys answer.
    Duplicates,
}

fn step(state: &mut AppState, input: Input) {
    *state = update(std::mem::take(state), input).0;
}

fn act(state: &mut AppState, action: CompareAction) {
    step(state, Input::Compare(action));
}

fn event(state: &mut AppState, e: CompareEvent) {
    step(state, Input::Event(Event::Compare(e)));
}

fn key(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Both sides read `crm.customers` (source) and `crm.customers_copy` (target).
fn pick(state: &mut AppState, side: Side, collection: &str) {
    act(
        state,
        CompareAction::PickSide(side, Pick::Database("crm".into())),
    );
    let entries = vec![
        ("customers".to_string(), CollectionKind::Collection),
        ("customers_copy".to_string(), CollectionKind::Collection),
        ("invoices".to_string(), CollectionKind::Collection),
    ];
    let listing = CompareEvent::Collections {
        session: SESSION,
        side,
        db: "crm".into(),
        entries,
    };
    event(state, listing);
    act(
        state,
        CompareAction::PickSide(side, Pick::Collection(collection.into())),
    );
}

fn answer(source_index: IndexStatus, target_index: IndexStatus, too_large: bool) -> CompareEvent {
    CompareEvent::Checked {
        session: SESSION,
        pair: PAIR,
        preflight: Preflight {
            source_count: 482_113,
            target_count: 480_907,
            source_index,
            target_index,
            unindexed_too_large: too_large,
        },
    }
}

fn open_session(state: &mut AppState) {
    let (source, target) = (state.conn.connections[0].id, state.conn.connections[1].id);
    act(state, CompareAction::Open { source, target });
    event(state, CompareEvent::Opened { session: SESSION });
    let databases = names(&["billing", "crm", "sandbox"]);
    for side in [Side::Source, Side::Target] {
        let listing = CompareEvent::Databases {
            session: SESSION,
            side,
            names: databases.clone(),
        };
        event(state, listing);
    }
}

fn finish_run(state: &mut AppState) {
    let summary = Summary {
        equal: 481_890,
        only_in_source: 1,
        only_in_target: 0,
        different: 2,
        scanned_source: 482_113,
        scanned_target: 480_907,
    };
    let done = CompareEvent::PairDone {
        session: SESSION,
        run: RUN,
        pair: PAIR,
        outcome: PairOutcome::Completed(summary),
    };
    event(state, done);
    event(
        state,
        CompareEvent::RunDone {
            session: SESSION,
            run: RUN,
        },
    );
    let rows = CompareEvent::Rows {
        session: SESSION,
        pair: PAIR,
        category: Category::Different,
        offset: 0,
        total: 2,
        rows: [1, 2]
            .map(|n| RowView {
                key: key(n),
                has_source: true,
                has_target: true,
            })
            .to_vec(),
    };
    event(state, rows);
}

fn open_detail(state: &mut AppState) {
    act(state, CompareAction::Select(key(2)));
    let changes = vec![
        FieldChange {
            path: "plan".into(),
            kind: ChangeKind::Changed,
            source: Some(Bson::String("team".into())),
            target: Some(Bson::String("pro".into())),
        },
        FieldChange {
            path: "address.city".into(),
            kind: ChangeKind::Removed,
            source: Some(Bson::String("Arlington".into())),
            target: None,
        },
    ];
    let detail = CompareEvent::Detail {
        session: SESSION,
        pair: PAIR,
        key: key(2),
        source: Some(
            doc! {"_id": 2_i32, "name": "Grace Hopper", "plan": "team", "seats": 40_i32,
            "address": {"city": "Arlington"}},
        ),
        target: Some(
            doc! {"_id": 2_i32, "name": "Grace Hopper", "plan": "pro", "seats": 40_i32,
            "address": {}},
        ),
        changes,
    };
    event(state, detail);
}

/// The run fails on a duplicate key; Find duplicate keys is asked and answered: two groups on
/// the source (one "and more"), none on the target.
fn find_duplicates(state: &mut AppState) {
    let failed = CompareEvent::PairDone {
        session: SESSION,
        run: RUN,
        pair: PAIR,
        outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
    };
    event(state, failed);
    let done = CompareEvent::RunDone {
        session: SESSION,
        run: RUN,
    };
    event(state, done);
    act(state, CompareAction::FindDuplicates);
    let group = |email: &str, count: u64| DuplicateGroup {
        key: KeyTuple(vec![Bson::String(email.into())]),
        count,
    };
    let source = DuplicateReport {
        groups: vec![group("ada@example.com", 12), group("grace@example.com", 3)],
        more: true,
    };
    let target = DuplicateReport {
        groups: vec![],
        more: false,
    };
    let answer = CompareEvent::DuplicateKeys {
        session: SESSION,
        pair: PAIR,
        source,
        target,
    };
    event(state, answer);
}

/// A connected browser switched to the Compare workspace, scripted up to `through`.
pub fn compare_state(through: Through) -> impl FnOnce(&mut AppState) {
    move |state| {
        browsing(ViewMode::Tree)(state);
        state.workspace = Workspace::Compare;
        open_session(state);
        if through == Through::Empty {
            return;
        }
        pick(state, Side::Source, "customers");
        pick(state, Side::Target, "customers_copy");
        if through == Through::Duplicates {
            // The panel lists emails, so the failed key is `email`.
            act(state, CompareAction::SetKey("email".into()));
        }
        act(state, CompareAction::RunCheck);
        if through == Through::Blocked {
            let blocked = answer(IndexStatus::Indexed, IndexStatus::NotIndexed, true);
            event(state, blocked);
            return;
        }
        event(
            state,
            answer(IndexStatus::Indexed, IndexStatus::Indexed, false),
        );
        act(state, CompareAction::Start);
        if through == Through::Running {
            let progress = CompareEvent::Progress {
                session: SESSION,
                run: RUN,
                pair: PAIR,
                scanned_source: 211_000,
                scanned_target: 209_500,
                differences: 1,
            };
            event(state, progress);
            return;
        }
        if through == Through::Duplicates {
            find_duplicates(state);
            return;
        }
        finish_run(state);
        if through == Through::Detail {
            open_detail(state);
        }
    }
}
