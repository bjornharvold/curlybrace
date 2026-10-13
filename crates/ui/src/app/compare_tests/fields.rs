//! Field-level selection and queueing in the reducer: what may be ticked and queued, what clears
//! the ticks, how a fields item replaces and merges, and that it confirms like any other item.
use super::confirm::{applies, key, reviewing, summary_of, tab};
use super::*;
use crate::viewmodel::compare::{ApplyUi, FieldSelection, PairStatus, RowAction, Scope};
use curlybrace_core::compare::apply::{ApplyKind, Direction};
use curlybrace_core::compare::diff::{ChangeKind, FieldChange, PathSegment};
use curlybrace_core::compare::key::compare_tuples;
use std::cmp::Ordering;

const TO_T: RowAction = RowAction::CopyFieldsToTarget;
const TO_S: RowAction = RowAction::CopyFieldsToSource;

fn seg(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

fn pth(name: &str) -> Vec<PathSegment> {
    vec![seg(name)]
}

fn change(segments: Vec<PathSegment>, kind: ChangeKind) -> FieldChange {
    FieldChange {
        path: String::new(),
        segments,
        kind,
        source: Some(Bson::Int32(1)),
        target: Some(Bson::Int32(2)),
    }
}

/// The detail's changes: addressable `a`, `b`, `c`, a pair that overlaps (`n` and `n.m`) and
/// three that can be copied in neither direction (`_id`, a dotted name, an array element).
fn changes() -> Vec<FieldChange> {
    use ChangeKind::{Added, Changed, Removed};
    vec![
        change(pth("a"), Changed),
        change(pth("b"), Added),
        change(pth("c"), Removed),
        change(pth("n"), Changed),
        change(vec![seg("n"), seg("m")], Changed),
        change(pth("_id"), Changed),
        change(pth("x.y"), Changed),
        change(vec![seg("arr"), PathSegment::Index(0)], Added),
    ]
}

fn detail_event(k: KeyTuple) -> Input {
    ev(CompareEvent::Detail {
        session: 1,
        pair: RUN_PAIR,
        key: k,
        source: Some(bson::doc! {"a": 1}),
        target: Some(bson::doc! {"a": 2}),
        changes: changes(),
    })
}

/// `state` with the detail of `k` asked for and answered.
fn open_detail(state: AppState, k: KeyTuple) -> AppState {
    let (state, cmds) = act(state, CompareAction::Select(k.clone()));
    assert_eq!(cmds.len(), 1, "the detail is asked for");
    update(state, detail_event(k)).0
}

/// A finished run on the Different tab with the detail of row 1 loaded.
fn detail_open() -> AppState {
    open_detail(tab(Category::Different, 3, true), key(1))
}

fn toggle(state: AppState, paths: &[Vec<PathSegment>]) -> AppState {
    paths.iter().fold(state, |s, p| {
        act(s, CompareAction::ToggleField(p.clone())).0
    })
}

/// The detail with `a` and `b` ticked.
pub(super) fn ticked() -> AppState {
    toggle(detail_open(), &[pth("a"), pth("b")])
}

/// `a` and `b` queued for a copy to the target; the detail stays open.
pub(super) fn queued() -> AppState {
    act(ticked(), CompareAction::AddFieldsToPending(TO_T)).0
}

pub(super) fn reviewing_queued() -> AppState {
    reviewing(queued())
}

fn ticks(state: &AppState) -> Vec<Vec<PathSegment>> {
    state.compare.field_selection.paths(RUN_PAIR, &key(1))
}

fn queued_fields(state: &AppState) -> Vec<(RowAction, KeyTuple, Vec<Vec<PathSegment>>)> {
    state
        .compare
        .pending
        .items()
        .iter()
        .filter_map(|c| match &c.scope {
            Scope::Fields { key, paths } => Some((c.action, key.clone(), paths.clone())),
            _ => None,
        })
        .collect()
}

/// `state` with `paths` ticked directly, as if the detail had allowed it.
fn forced(state: AppState, paths: &[Vec<PathSegment>]) -> AppState {
    let selection = paths.iter().fold(FieldSelection::default(), |s, p| {
        s.toggled(RUN_PAIR, &key(1), p)
    });
    let mut state = state;
    state.compare.field_selection = selection;
    state
}

#[test]
fn toggle_clear_and_queue_produce_the_pending_item_and_its_command_parts() {
    let state = toggle(detail_open(), &[pth("b"), pth("a"), pth("c")]);
    assert_eq!(ticks(&state), vec![pth("a"), pth("b"), pth("c")]);
    let state = toggle(state, &[pth("c")]);
    assert_eq!(ticks(&state), vec![pth("a"), pth("b")], "untick");
    let (cleared, cmds) = act(copy_of(&state), CompareAction::ClearFieldSelection);
    assert!(cmds.is_empty() && ticks(&cleared).is_empty());

    let (state, cmds) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert!(cmds.is_empty(), "queueing writes nothing");
    assert_eq!(
        queued_fields(&state),
        vec![(TO_T, key(1), vec![pth("a"), pth("b")])]
    );
    assert!(
        ticks(&state).is_empty(),
        "the queued key's ticks are cleared"
    );
    assert!(state.compare.detail.as_ref().is_some_and(|d| !d.loading));
    assert!(state.banner.is_none());
    let (ops, bulk) = state.compare.pending.to_command_parts();
    assert!(bulk.is_empty());
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].kind, ApplyKind::Fields);
    assert_eq!(ops[0].direction, Direction::SourceToTarget);
    assert_eq!(ops[0].key, key(1));
    assert_eq!(ops[0].pair, RUN_PAIR);
    assert_eq!(ops[0].fields, vec![pth("a"), pth("b")]);
}

#[test]
fn queueing_to_the_source_runs_target_to_source() {
    let (state, _) = act(ticked(), CompareAction::AddFieldsToPending(TO_S));
    let (ops, _) = state.compare.pending.to_command_parts();
    assert_eq!(ops[0].direction, Direction::TargetToSource);
}

/// Each state is one in which neither ticking nor queueing may work.
fn gated() -> Vec<(&'static str, AppState)> {
    let mut not_completed = detail_open();
    not_completed.compare.overview.rows[0].status = PairStatus::Pending;
    let applied = {
        let mut s = detail_open();
        s.compare.overview = s.compare.overview.clone().marked_applied(RUN_PAIR);
        s
    };
    let loading = act(
        tab(Category::Different, 3, true),
        CompareAction::Select(key(1)),
    )
    .0;
    let other_tab = open_detail(tab(Category::OnlyInSource, 3, true), key(1));
    let (running, cmds) = act(reviewing_queued(), CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1);
    vec![
        ("no detail", tab(Category::Different, 3, true)),
        ("detail still loading", loading),
        ("pair not completed", not_completed),
        ("pair applied", applied),
        ("apply under review", reviewing_queued()),
        ("apply running", running),
        ("detail of another tab", other_tab),
    ]
}

#[test]
fn ticking_is_refused_unless_the_detail_is_loaded_and_the_tab_writable() {
    for (name, state) in gated() {
        let (state, cmds) = act(state, CompareAction::ToggleField(pth("a")));
        assert!(cmds.is_empty(), "{name}");
        assert!(ticks(&state).is_empty(), "{name}");
    }
}

#[test]
fn clearing_is_refused_while_the_lists_are_read_only() {
    for (name, state) in gated() {
        let state = forced(state, &[pth("a")]);
        let (state, cmds) = act(state, CompareAction::ClearFieldSelection);
        assert!(cmds.is_empty(), "{name}");
        assert_eq!(ticks(&state), vec![pth("a")], "{name}");
    }
}

#[test]
fn queueing_is_refused_unless_the_detail_is_loaded_and_the_tab_writable() {
    for (name, state) in gated() {
        let state = forced(state, &[pth("a"), pth("b")]);
        let before = state.compare.pending.clone();
        let (after, cmds) = act(state, CompareAction::AddFieldsToPending(TO_T));
        assert!(cmds.is_empty(), "{name}");
        assert_eq!(after.compare.pending, before, "{name}");
        assert_eq!(ticks(&after).len(), 2, "{name}: the ticks stay");
        assert!(after.banner.is_none(), "{name}");
    }
}

#[test]
fn a_path_that_is_unaddressable_both_ways_or_not_in_the_detail_cannot_be_ticked() {
    let refused = [
        pth("_id"),
        pth("x.y"),
        vec![seg("arr"), PathSegment::Index(0)],
        pth("not_changed"),
        vec![seg("a"), seg("deeper")],
        Vec::new(),
        vec![PathSegment::AnyElement],
    ];
    for path in refused {
        let (state, _) = act(detail_open(), CompareAction::ToggleField(path.clone()));
        assert!(ticks(&state).is_empty(), "{path:?}");
    }
}

#[test]
fn queueing_needs_a_copyable_selection_and_a_field_action() {
    let nothing = act(detail_open(), CompareAction::AddFieldsToPending(TO_T)).0;
    assert!(nothing.compare.pending.is_empty() && nothing.banner.is_none());

    let unaddressable = forced(detail_open(), &[pth("_id"), pth("x.y")]);
    let (state, cmds) = act(unaddressable, CompareAction::AddFieldsToPending(TO_S));
    assert!(cmds.is_empty() && state.compare.pending.is_empty());
    assert_eq!(ticks(&state).len(), 2, "refused: the ticks stay");

    for action in [
        RowAction::CopyToTarget,
        RowAction::CopyToSource,
        RowAction::DeleteFromTarget,
        RowAction::DeleteFromSource,
    ] {
        let (state, _) = act(ticked(), CompareAction::AddFieldsToPending(action));
        assert!(state.compare.pending.is_empty(), "{action:?}");
        assert_eq!(ticks(&state).len(), 2);
    }
}

#[test]
fn only_the_copyable_ticked_paths_are_queued() {
    let state = forced(detail_open(), &[pth("a"), pth("_id"), pth("not_changed")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(queued_fields(&state), vec![(TO_T, key(1), vec![pth("a")])]);
    assert!(ticks(&state).is_empty());
}

/// Every event that closes or replaces the detail, or removes the pair, forgets the ticks.
#[test]
fn the_ticks_are_cleared_wherever_the_detail_goes_away() {
    let failed = CompareEvent::Failed {
        session: 1,
        context: "compare_detail",
        error: CoreError::NotFound,
    };
    let table: Vec<(&str, Input)> = vec![
        ("Back", Input::Compare(CompareAction::Back)),
        (
            "Select another",
            Input::Compare(CompareAction::Select(key(2))),
        ),
        (
            "Select the same",
            Input::Compare(CompareAction::Select(key(1))),
        ),
        (
            "category",
            Input::Compare(CompareAction::SelectCategory(Category::OnlyInSource)),
        ),
        ("overview", Input::Compare(CompareAction::BackToOverview)),
        ("discard", Input::Compare(CompareAction::DiscardPair(ENTRY))),
        ("re-run", Input::Compare(CompareAction::Rerun(ENTRY))),
        (
            "remove pair",
            Input::Compare(CompareAction::RemovePair(ENTRY)),
        ),
        ("close session", Input::Compare(CompareAction::CloseSession)),
        ("detail failed", ev(failed)),
    ];
    for (name, input) in table {
        let (state, _) = update(ticked(), input);
        assert_eq!(
            state.compare.field_selection,
            FieldSelection::default(),
            "{name}"
        );
    }
}

#[test]
fn paging_clears_the_ticks_and_closes_the_detail() {
    let state = toggle(open_detail(with_rows(), key(1)), &[pth("a"), pth("b")]);
    assert_eq!(ticks(&state).len(), 2);
    let (state, _) = act(state, CompareAction::Page(1));
    assert!(state.compare.detail.is_none());
    assert_eq!(state.compare.field_selection, FieldSelection::default());
}

#[test]
fn unrelated_actions_keep_the_ticks() {
    let keep = [
        CompareAction::ToggleRow(key(2)),
        CompareAction::OpenPair(ENTRY),
        CompareAction::AddKeyToPending(key(2), RowAction::CopyToSource),
        CompareAction::SelectAllInCategory,
        CompareAction::ClearSelection,
        CompareAction::ClearPending,
    ];
    for action in keep {
        let (state, _) = act(ticked(), action.clone());
        assert_eq!(ticks(&state).len(), 2, "{action:?}");
        assert!(state.compare.detail.is_some(), "{action:?}");
    }
}

#[test]
fn a_late_detail_answer_attaches_no_ticks_to_another_key() {
    let (state, _) = act(ticked(), CompareAction::Select(key(2)));
    assert!(ticks(&state).is_empty());
    // The answer for the key that was left arrives late: dropped, the new detail still loads.
    let (state, _) = update(state, detail_event(key(1)));
    assert!(state.compare.detail.as_ref().is_some_and(|d| d.loading));
    let (state, _) = act(state, CompareAction::ToggleField(pth("a")));
    assert_eq!(state.compare.field_selection, FieldSelection::default());
    // The answer for key 2 loads it, still without any tick.
    let (state, _) = update(state, detail_event(key(2)));
    assert!(state.compare.detail.as_ref().is_some_and(|d| !d.loading));
    assert_eq!(state.compare.field_selection, FieldSelection::default());
    assert_eq!(state.compare.field_selection.count(RUN_PAIR, &key(1)), 0);
}

#[test]
fn queueing_more_fields_in_the_same_direction_merges_them() {
    let state = toggle(queued(), &[pth("c")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(
        queued_fields(&state),
        vec![(TO_T, key(1), vec![pth("a"), pth("b"), pth("c")])]
    );
    assert_eq!(state.compare.pending.items().len(), 1);
}

#[test]
fn queueing_in_the_other_direction_replaces_the_item() {
    let state = toggle(queued(), &[pth("c")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_S));
    assert_eq!(queued_fields(&state), vec![(TO_S, key(1), vec![pth("c")])]);
    assert_eq!(state.compare.pending.items().len(), 1);
}

#[test]
fn fields_and_a_whole_document_change_of_one_key_replace_each_other() {
    let whole = CompareAction::AddKeyToPending(key(1), RowAction::CopyToTarget);
    let elsewhere = CompareAction::AddKeyToPending(key(2), RowAction::CopyToSource);
    let (state, _) = act(detail_open(), whole.clone());
    let (state, _) = act(state, elsewhere);
    assert_eq!(state.compare.pending.items().len(), 2);

    let state = toggle(state, &[pth("a")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    let (ops, _) = state.compare.pending.to_command_parts();
    assert_eq!(ops.len(), 2, "the whole-document change of key 1 is gone");
    assert!(
        ops.iter()
            .any(|o| o.key == key(1) && o.kind == ApplyKind::Fields)
    );
    assert!(
        ops.iter()
            .any(|o| o.key == key(2) && o.kind == ApplyKind::Replace)
    );

    let (state, _) = act(state, whole);
    let (ops, _) = state.compare.pending.to_command_parts();
    assert_eq!(ops.len(), 2, "and the other way round");
    assert!(
        ops.iter()
            .any(|o| o.key == key(1) && o.kind == ApplyKind::Replace)
    );
    assert!(queued_fields(&state).is_empty());
}

#[test]
fn overlapping_paths_are_refused_with_a_banner_and_nothing_changes() {
    let overlap = vec![seg("n"), seg("m")];
    let state = toggle(detail_open(), &[pth("n"), overlap.clone()]);
    let (state, cmds) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert!(cmds.is_empty() && state.compare.pending.is_empty());
    assert_eq!(state.banner, Some(Banner::FieldsConflict));
    assert_eq!(ticks(&state).len(), 2, "the ticks stay to be fixed");

    // The union with what is queued for the key overlaps too: the queued item stays as it was.
    let state = toggle(
        act(ticked(), CompareAction::AddFieldsToPending(TO_T)).0,
        &[pth("n")],
    );
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    let before = state.compare.pending.clone();
    let state = toggle(state, &[overlap]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(state.compare.pending, before);
    assert_eq!(state.banner, Some(Banner::FieldsConflict));
}

#[test]
fn a_category_change_that_covers_the_tab_refuses_fields_with_a_banner() {
    let (state, _) = act(detail_open(), CompareAction::SelectAllInCategory);
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
    let before = state.compare.pending.clone();
    assert_eq!(before.items().len(), 1);
    let state = toggle(state, &[pth("a")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(state.compare.pending, before);
    assert_eq!(state.banner, Some(Banner::FieldsCovered));
    assert_eq!(ticks(&state), vec![pth("a")]);
}

fn nan_key() -> KeyTuple {
    KeyTuple(vec![Bson::Double(f64::NAN)])
}

/// A finished run whose only Different row has a NaN key, with its detail loaded.
fn nan_detail() -> AppState {
    let rows = ev(CompareEvent::Rows {
        session: 1,
        pair: RUN_PAIR,
        category: Category::Different,
        offset: 0,
        total: 1,
        rows: vec![RowView {
            key: nan_key(),
            has_source: true,
            has_target: true,
        }],
    });
    let done = pair_done(1, summary_of(Category::Different, 1));
    let (state, _) = update(running_state(), done);
    let (state, _) = update(state, rows);
    let (state, _) = update(state, run_done());
    assert!(nan_key() != nan_key(), "a NaN key is never == to itself");
    open_detail(state, nan_key())
}

#[test]
fn a_document_with_a_nan_key_ticks_queues_reviews_and_confirms() {
    let state = toggle(nan_detail(), &[pth("a")]);
    assert_eq!(state.compare.field_selection.count(RUN_PAIR, &nan_key()), 1);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(state.compare.field_selection.count(RUN_PAIR, &nan_key()), 0);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert_eq!(
        state.compare.pending.items().len(),
        1,
        "nothing ticked: no second item"
    );

    let state = toggle(state, &[pth("b")]);
    let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
    let fields = queued_fields(&state);
    assert_eq!(
        fields.len(),
        1,
        "merged into the NaN key's item, not a second one"
    );
    assert_eq!(fields[0].2, vec![pth("a"), pth("b")]);

    let (state, cmds) = act(reviewing(state), CompareAction::ConfirmApply);
    let [CompareCommand::Apply { ops, .. }] = applies(&cmds)[..] else {
        panic!("exactly one apply, got {cmds:?}")
    };
    assert_eq!(ops.len(), 1);
    assert_eq!(compare_tuples(&ops[0].key, &nan_key()), Ordering::Equal);
    assert_eq!(ops[0].fields, vec![pth("a"), pth("b")]);
    assert!(state.compare.apply.is_running());
}

#[test]
fn a_fields_item_reviews_confirms_and_sends_exactly_one_apply() {
    let reviewed = reviewing_queued();
    let (state, cmds) = act(reviewed, CompareAction::ConfirmApply);
    let [CompareCommand::Apply { ops, bulk, .. }] = applies(&cmds)[..] else {
        panic!("exactly one apply, got {cmds:?}")
    };
    assert!(bulk.is_empty());
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].pair, RUN_PAIR);
    assert_eq!(ops[0].key, key(1));
    assert_eq!(ops[0].kind, ApplyKind::Fields);
    assert_eq!(ops[0].direction, Direction::SourceToTarget);
    assert_eq!(ops[0].fields, vec![pth("a"), pth("b")]);
    assert!(state.compare.apply.is_running());
    let (_, again) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&again).is_empty(), "a double confirm sends nothing");
}

/// Swaps the reviewed fields item for `other` (same pair) and confirms: nothing may be sent.
fn confirm_after_swap(key_: KeyTuple, action: RowAction, paths: Vec<Vec<PathSegment>>) {
    let mut state = reviewing_queued();
    let (swapped, outcome) = state
        .compare
        .pending
        .cleared()
        .with_fields(RUN_PAIR, &key_, action, paths);
    assert_eq!(outcome, crate::viewmodel::compare::FieldsQueued::Queued);
    state.compare.pending = swapped;
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty());
    assert_eq!(state.compare.apply, ApplyUi::Idle);
}

#[test]
fn a_path_key_or_direction_swap_between_review_and_confirm_sends_nothing() {
    confirm_after_swap(key(1), TO_T, vec![pth("a"), pth("c")]);
    confirm_after_swap(key(1), TO_T, vec![pth("a")]);
    confirm_after_swap(key(2), TO_T, vec![pth("a"), pth("b")]);
    confirm_after_swap(key(1), TO_S, vec![pth("a"), pth("b")]);
}

#[test]
fn opening_another_pair_drops_the_ticks_of_the_pair_left() {
    use super::multirun::{finished_three, pair_of, rows_for};
    let state = finished_three();
    let (a, b) = (pair_of(&state, 0), pair_of(&state, 1));
    let state = update(state, rows_for(a, 3)).0;
    let (state, _) = act(state, CompareAction::OpenPair(0));
    // The pair holds one different document, key 0.
    let (state, cmds) = act(state, CompareAction::Select(key(0)));
    assert_eq!(cmds.len(), 1, "the detail is asked for");
    let answer = ev(CompareEvent::Detail {
        session: 1,
        pair: a,
        key: key(0),
        source: None,
        target: None,
        changes: changes(),
    });
    let state = update(state, answer).0;
    let state = toggle(state, &[pth("a")]);
    assert_eq!(state.compare.field_selection.count(a, &key(0)), 1);
    let (state, _) = act(state, CompareAction::OpenPair(1));
    assert_eq!(state.compare.open_pair, Some(b));
    assert!(state.compare.detail.is_none());
    assert_eq!(state.compare.field_selection, FieldSelection::default());
}

#[test]
fn fields_can_be_queued_while_a_run_is_active() {
    // Deliberate: the reducer gate is the whole-document one (it ignores an active run); the
    // views decide whether to offer the buttons.
    let running = open_detail(tab(Category::Different, 3, false), key(1));
    assert!(running.compare.is_running());
    let state = toggle(running, &[pth("a")]);
    let (state, cmds) = act(state, CompareAction::AddFieldsToPending(TO_T));
    assert!(cmds.is_empty());
    assert_eq!(queued_fields(&state), vec![(TO_T, key(1), vec![pth("a")])]);
}

#[test]
fn a_successful_queue_clears_only_the_fields_refusal_banners() {
    for (banner, cleared) in [
        (Banner::FieldsConflict, true),
        (Banner::FieldsCovered, true),
        (Banner::NotConnected, false),
    ] {
        let mut state = ticked();
        state.banner = Some(banner.clone());
        let (state, _) = act(state, CompareAction::AddFieldsToPending(TO_T));
        assert_eq!(state.compare.pending.items().len(), 1);
        assert_eq!(state.banner.is_none(), cleared, "{banner:?}");
    }
}

#[test]
fn no_field_action_ever_sends_a_command() {
    let actions = [
        CompareAction::ToggleField(pth("a")),
        CompareAction::ClearFieldSelection,
        CompareAction::AddFieldsToPending(TO_T),
        CompareAction::AddFieldsToPending(TO_S),
    ];
    for action in actions {
        let (_, cmds) = act(ticked(), action.clone());
        assert!(cmds.is_empty(), "{action:?}");
    }
}
