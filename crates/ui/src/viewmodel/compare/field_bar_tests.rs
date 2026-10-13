use super::*;
use crate::viewmodel::compare::overview::{PairStatus, RunRow};
use crate::viewmodel::compare::{ApplyUi, Counts};
use bson::Bson;
use curlybrace_core::compare::diff::{ChangeKind, FieldChange};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::RowView;
use curlybrace_core::compare::results::Category;

const PAIR: PairId = 2;
const TO_T: Direction = Direction::SourceToTarget;
const TO_S: Direction = Direction::TargetToSource;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn field(name: &str) -> Vec<PathSegment> {
    vec![PathSegment::Field(name.into())]
}

fn change(segments: Vec<PathSegment>) -> FieldChange {
    FieldChange {
        path: String::new(),
        segments,
        kind: ChangeKind::Changed,
        source: Some(Bson::Int32(1)),
        target: Some(Bson::Int32(2)),
    }
}

/// `name`, `_id`, a dotted name, a `$` name and a field inside an unordered array.
fn detail() -> DetailView {
    let changes = [
        field("name"),
        field("_id"),
        field("a.b"),
        field("$x"),
        vec![PathSegment::Field("tags".into()), PathSegment::AnyElement],
    ]
    .map(change);
    DetailView::loading(k(1)).loaded(None, None, &changes)
}

fn state() -> CompareState {
    let mut state = CompareState::default();
    state.overview.rows = vec![RunRow {
        entry: 0,
        pair: PAIR,
        run: 1,
        status: PairStatus::Completed(Summary::default()),
        last_progress: (0, 0, 0),
        applied: false,
        facts: None,
    }];
    let rows = (1..=3)
        .map(|n| RowView {
            key: k(n),
            has_source: true,
            has_target: true,
        })
        .collect();
    state.results = [(
        PAIR,
        crate::viewmodel::compare::ResultsView {
            pair: PAIR,
            counts: Counts {
                only_in_source: 0,
                only_in_target: 0,
                different: 3,
                identical: 0,
            },
            category: Category::Different,
            offset: 0,
            rows,
            requested: None,
            load_failed: false,
        },
    )]
    .into();
    state.open_pair = Some(PAIR);
    state
}

fn ticking(state: CompareState, paths: &[Vec<PathSegment>]) -> CompareState {
    let field_selection = paths.iter().fold(state.field_selection.clone(), |s, p| {
        s.toggled(PAIR, &k(1), p)
    });
    CompareState {
        field_selection,
        ..state
    }
}

fn line(detail: &DetailView, name: &str) -> ChangeLine {
    detail
        .changes
        .iter()
        .find(|l| l.segments == field(name))
        .expect("line")
        .clone()
}

#[test]
fn an_addressable_field_can_be_ticked() {
    let (state, detail) = (state(), detail());
    assert_eq!(
        state.field_tick_block(&detail, &line(&detail, "name")),
        None
    );
}

#[test]
fn a_field_that_cannot_be_written_in_either_direction_says_why() {
    let (state, detail) = (state(), detail());
    let reason = |name: &str, why| {
        assert_eq!(
            state.field_tick_block(&detail, &line(&detail, name)),
            Some(FieldBlock::Unaddressable(why)),
            "{name}"
        );
    };
    reason("_id", Unaddressable::Id);
    reason("a.b", Unaddressable::BadName);
    reason("$x", Unaddressable::BadName);
    let any = detail.changes.last().expect("any-element line");
    assert_eq!(
        state.field_tick_block(&detail, any),
        Some(FieldBlock::Unaddressable(Unaddressable::AnyElement))
    );
}

#[test]
fn the_gate_closes_every_checkbox_with_the_standard_reason() {
    let detail = detail();
    let name = line(&detail, "name");
    let mut busy = state();
    busy.apply = ApplyUi::Running {
        run: 7,
        total: 0,
        rows: Vec::new(),
        cancel_requested: false,
    };
    let mut applied = state();
    applied.overview.rows[0].applied = true;
    let mut pending = state();
    pending.overview.rows[0].status = PairStatus::Pending;
    for (state, why) in [
        (busy, ActionBlock::Busy),
        (applied, ActionBlock::Applied),
        (pending, ActionBlock::NotCompleted),
    ] {
        assert_eq!(
            state.field_tick_block(&detail, &name),
            Some(FieldBlock::Gate(why))
        );
        // The gate wins over the field's own reason.
        assert_eq!(
            state.field_tick_block(&detail, &line(&detail, "_id")),
            Some(FieldBlock::Gate(why))
        );
    }
}

#[test]
fn a_key_off_the_page_and_a_closed_pair_close_the_controls() {
    let detail = DetailView::loading(k(99)).loaded(None, None, &[change(field("name"))]);
    let off_page = state();
    assert_eq!(
        off_page.field_tick_block(&detail, &detail.changes[0]),
        Some(FieldBlock::Gate(ActionBlock::NotOnPage))
    );
    let none_open = CompareState::default();
    assert_eq!(
        none_open.field_clear_block(&detail),
        Some(FieldBlock::Gate(ActionBlock::NotCompleted))
    );
}

#[test]
fn nothing_ticked_closes_the_queue_buttons_and_clear() {
    let (state, detail) = (state(), detail());
    for direction in [TO_T, TO_S] {
        assert_eq!(
            state.field_queue_block(&detail, direction),
            Some(FieldBlock::NoFields)
        );
    }
    assert_eq!(state.field_clear_block(&detail), Some(FieldBlock::NoFields));
}

#[test]
fn a_ticked_copyable_field_opens_both_buttons_and_clear() {
    let (state, detail) = (ticking(state(), &[field("name")]), detail());
    assert_eq!(state.field_queue_block(&detail, TO_T), None);
    assert_eq!(state.field_queue_block(&detail, TO_S), None);
    assert_eq!(state.field_clear_block(&detail), None);
}

#[test]
fn ticks_that_cannot_be_copied_here_name_that_reason() {
    // The reducer never ticks these; a stale tick must still not enable a button.
    let (state, detail) = (ticking(state(), &[field("_id")]), detail());
    for direction in [TO_T, TO_S] {
        assert_eq!(
            state.field_queue_block(&detail, direction),
            Some(FieldBlock::NotCopyableHere)
        );
    }
    // One copyable field among them is enough.
    let mixed = ticking(state, &[field("name")]);
    assert_eq!(mixed.field_queue_block(&detail, TO_T), None);
}

#[test]
fn a_run_or_a_check_closes_the_buttons_but_not_the_checkboxes() {
    let detail = detail();
    let mut running = ticking(state(), &[field("name")]);
    running.overview.active = Some(crate::viewmodel::compare::ActiveRun {
        run: 2,
        cancel_requested: false,
        failure_noted: false,
        pairs: 1,
    });
    assert_eq!(
        running.field_queue_block(&detail, TO_T),
        Some(FieldBlock::Gate(ActionBlock::Running))
    );
    assert_eq!(
        running.field_tick_block(&detail, &line(&detail, "name")),
        None
    );
}

#[test]
fn the_bar_lists_one_row_per_change_with_its_tick_and_the_counts() {
    let state = ticking(state(), &[field("name"), field("_id")]);
    let detail = detail();
    let bar = FieldBar::of(&state, &detail);
    assert_eq!(bar.rows.len(), detail.changes.len());
    assert_eq!(bar.selected, 2);
    assert!(bar.rows[0].ticked && bar.rows[1].ticked && !bar.rows[2].ticked);
    assert_eq!(bar.rows[0].block, None);
    assert_eq!(
        bar.rows[1].block,
        Some(FieldBlock::Unaddressable(Unaddressable::Id))
    );
    assert_eq!(bar.queue_block(RowAction::CopyFieldsToTarget), None);
    assert_eq!(bar.queue_block(RowAction::CopyFieldsToSource), None);
    assert_eq!(bar.clear, None);
}

#[test]
fn every_block_has_a_distinct_translated_key() {
    let keys: Vec<_> = [
        FieldBlock::Gate(ActionBlock::Busy),
        FieldBlock::Unaddressable(Unaddressable::Id),
        FieldBlock::NoFields,
        FieldBlock::NotCopyableHere,
    ]
    .iter()
    .map(|b| b.i18n_key())
    .collect();
    assert_eq!(
        keys,
        [
            "compare.apply.reason.busy",
            "compare.apply.field_reason.id",
            "compare.apply.reason.no_fields",
            "compare.apply.reason.not_copyable_here",
        ]
    );
}

#[test]
fn only_the_different_tab_offers_field_copies() {
    let mut state = ticking(state(), &[field("name")]);
    state.results.get_mut(&PAIR).expect("results").category = Category::OnlyInTarget;
    let detail = detail();
    let blocked = Some(FieldBlock::Gate(ActionBlock::NotDifferentTab));
    assert_eq!(
        state.field_tick_block(&detail, &line(&detail, "name")),
        blocked
    );
    assert_eq!(state.field_queue_block(&detail, TO_T), blocked);
    assert_eq!(state.field_queue_block(&detail, TO_S), blocked);
    assert_eq!(state.field_clear_block(&detail), blocked);
    assert_eq!(
        ActionBlock::NotDifferentTab.i18n_key(),
        "compare.apply.reason.not_different_tab"
    );
}
