//! The pair list: adding, removing, including and editing entries, and the defaults they follow.
use super::*;
use crate::viewmodel::compare::{EntryId, PairOverrides, PairStatus, SetupError};

pub(super) fn listing(state: AppState, side: Side, db: &str, names: &[&str]) -> AppState {
    let entries = names
        .iter()
        .map(|n| ((*n).to_string(), CollectionKind::Collection))
        .collect();
    let event = CompareEvent::Collections {
        session: 1,
        side,
        db: db.into(),
        entries,
    };
    update(state, ev(event)).0
}

/// An open session with databases a and b picked and `names` listed on both sides; no pair yet.
pub(super) fn databases_state(names: &[&str]) -> AppState {
    let (mut state, _, _) = opened();
    for (side, db) in [(Side::Source, "a"), (Side::Target, "b")] {
        let pick = CompareAction::PickSide(side, Pick::Database(db.into()));
        state = listing(act(state, pick).0, side, db, names);
    }
    state
}

/// One entry per name, in list order (ids 0, 1, 2, ...).
pub(super) fn multi_state(names: &[&str]) -> AppState {
    act(databases_state(names), CompareAction::AddAllSameNamed).0
}

pub(super) fn sources(state: &AppState) -> Vec<String> {
    state
        .compare
        .pairs
        .entries
        .iter()
        .map(|e| e.source.collection.clone())
        .collect()
}

pub(super) fn status(state: &AppState, entry: EntryId) -> PairStatus {
    state
        .compare
        .overview
        .row_for(entry)
        .expect("the entry has a row")
        .status
        .clone()
}

pub(super) fn all_unchecked(state: &AppState) -> bool {
    use crate::viewmodel::compare::EntryCheck;
    state
        .compare
        .checks
        .entries
        .values()
        .all(|c| *c == EntryCheck::Unchecked)
}

fn pick_both(state: AppState, name: &str) -> AppState {
    [Side::Source, Side::Target]
        .into_iter()
        .fold(state, |s, side| {
            act(
                s,
                CompareAction::PickSide(side, Pick::Collection(name.into())),
            )
            .0
        })
}

#[test]
fn add_pair_appends_an_entry_and_invalidates_checks() {
    let state = listing(checked_state(), Side::Source, "a", &["users", "orders"]);
    let state = listing(state, Side::Target, "b", &["users", "orders"]);
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
    let (state, cmds) = act(pick_both(state, "orders"), CompareAction::AddPair);
    assert!(cmds.is_empty());
    assert_eq!(sources(&state), vec!["users", "orders"]);
    assert!(all_unchecked(&state), "{:?}", state.compare.checks);
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
}

#[test]
fn adding_the_same_pair_again_changes_nothing() {
    let state = checked_state();
    let (again, cmds) = act(state, CompareAction::AddPair);
    assert!(cmds.is_empty());
    assert_eq!(sources(&again), vec!["users"]);
    assert_eq!(again.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn add_pair_refuses_a_view() {
    let (state, _) = act(
        picked_form_state(),
        CompareAction::PickSide(Side::Target, Pick::Collection("v".into())),
    );
    let (state, cmds) = act(state, CompareAction::AddPair);
    assert!(cmds.is_empty() && state.compare.pairs.entries.is_empty());
    assert!(
        state
            .compare
            .issues()
            .contains(&SetupError::ViewSelected(Side::Target))
    );
}

#[test]
fn add_all_same_named_adds_every_matched_name_once() {
    let state = databases_state(&["alpha", "beta"]);
    let state = listing(state, Side::Source, "a", &["alpha", "beta", "only_s"]);
    let state = listing(state, Side::Target, "b", &["alpha", "beta", "only_t"]);
    let (state, cmds) = act(state, CompareAction::AddAllSameNamed);
    assert!(cmds.is_empty());
    assert_eq!(sources(&state), vec!["alpha", "beta"]);
    let before = state.compare.pairs.clone();
    let (again, _) = act(state, CompareAction::AddAllSameNamed);
    assert_eq!(again.compare.pairs, before, "running it twice adds nothing");
}

#[test]
fn add_all_same_named_keeps_overrides_and_adds_only_the_new_names() {
    let state = multi_state(&["alpha", "beta"]);
    let custom = PairOverrides {
        key_text: Some("sku".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(1, custom.clone()));
    let state = listing(state, Side::Source, "a", &["alpha", "beta", "gamma"]);
    let state = listing(state, Side::Target, "b", &["alpha", "beta", "gamma"]);
    let (state, _) = act(state, CompareAction::AddAllSameNamed);
    assert_eq!(sources(&state), vec!["alpha", "beta", "gamma"]);
    assert_eq!(state.compare.pairs.entries[1].overrides, custom);
}

#[test]
fn add_unmatched_pairs_with_a_typed_name() {
    let state = listing(
        databases_state(&["alpha"]),
        Side::Source,
        "a",
        &["alpha", "only_s"],
    );
    let add = |side, name: &str, as_name: &str| CompareAction::AddUnmatched {
        side,
        name: name.into(),
        as_name: as_name.into(),
    };
    let (state, _) = act(state, add(Side::Source, "only_s", "  renamed "));
    let entry = &state.compare.pairs.entries[0];
    assert_eq!(
        (entry.source.db.as_str(), entry.source.collection.as_str()),
        ("a", "only_s")
    );
    assert_eq!(
        (entry.target.db.as_str(), entry.target.collection.as_str()),
        ("b", "renamed")
    );
    let (state, _) = act(state, add(Side::Target, "only_t", "mine"));
    let entry = &state.compare.pairs.entries[1];
    assert_eq!(entry.source.collection, "mine");
    assert_eq!(entry.target.collection, "only_t");
    // Nothing typed adds nothing.
    let (state, _) = act(state, add(Side::Source, "only_s", "   "));
    assert_eq!(state.compare.pairs.entries.len(), 2);
}

#[test]
fn add_unmatched_never_pairs_with_a_view() {
    let state = databases_state(&["alpha"]);
    let views = vec![("v".to_string(), CollectionKind::View)];
    let event = CompareEvent::Collections {
        session: 1,
        side: Side::Target,
        db: "b".into(),
        entries: views,
    };
    let (state, _) = update(state, ev(event));
    let action = CompareAction::AddUnmatched {
        side: Side::Source,
        name: "alpha".into(),
        as_name: "v".into(),
    };
    let (state, _) = act(state, action);
    assert!(state.compare.pairs.entries.is_empty());
}

#[test]
fn remove_pair_discards_its_results_and_clears_open_pair() {
    let state = finished_with_results();
    assert_eq!(state.compare.open_pair, Some(RUN_PAIR));
    let (state, cmds) = act(state, CompareAction::RemovePair(ENTRY));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            session: 1,
            pair: RUN_PAIR
        })]
    ));
    assert!(state.compare.pairs.entries.is_empty());
    assert!(state.compare.results.is_empty() && state.compare.open_pair.is_none());
    assert!(state.compare.overview.row_for(ENTRY).is_none());
}

#[test]
fn removing_an_unknown_entry_changes_nothing() {
    let (state, cmds) = act(checked_state(), CompareAction::RemovePair(42));
    assert!(cmds.is_empty());
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn removing_an_entry_invalidates_the_other_verdicts() {
    let state = multi_state(&["alpha", "beta"]);
    let (state, _) = act(state, CompareAction::RunCheck);
    let state = update(state, checked_as(1, indexed(5))).0;
    let (state, _) = act(state, CompareAction::RemovePair(1));
    assert_eq!(sources(&state), vec!["alpha"]);
    assert!(all_unchecked(&state));
}

#[test]
fn editing_a_default_invalidates_checks_but_not_overrides() {
    let state = multi_state(&["alpha", "beta"]);
    let custom = PairOverrides {
        key_text: Some("sku".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(1, custom));
    let (state, _) = act(state, CompareAction::RunCheck);
    let state = update(state, checked_as(1, indexed(5))).0;
    let state = update(state, checked_as(2, indexed(5))).0;
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
    let (state, _) = act(state, CompareAction::SetKey("name".into()));
    assert!(all_unchecked(&state));
    let c = &state.compare;
    assert_eq!(c.pairs.entries[0].effective(&c.setup).key_text, "name");
    assert_eq!(c.pairs.entries[1].effective(&c.setup).key_text, "sku");
    assert_eq!(
        c.pairs.entries[1].overrides.key_text.as_deref(),
        Some("sku")
    );
}

#[test]
fn setting_a_default_to_what_it_already_is_keeps_the_verdicts() {
    let (state, _) = act(checked_state(), CompareAction::SetKey("_id".into()));
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn a_picker_change_never_touches_the_entries_or_their_verdicts() {
    let state = listing(checked_state(), Side::Source, "a", &["users", "orders"]);
    let (state, _) = act(
        state,
        CompareAction::PickSide(Side::Source, Pick::Collection("orders".into())),
    );
    assert_eq!(sources(&state), vec!["users"]);
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

#[test]
fn including_invalidates_only_when_the_flag_changes() {
    let state = multi_state(&["alpha", "beta"]);
    let (state, _) = act(state, CompareAction::RunCheck);
    let state = update(state, checked_as(1, indexed(5))).0;
    let state = update(state, checked_as(2, indexed(5))).0;
    let (same, _) = act(copy_of(&state), CompareAction::IncludePair(0, true));
    assert_eq!(same.compare.preflight_view(), PreflightView::Ok);
    let (changed, _) = act(state, CompareAction::IncludePair(0, false));
    assert!(all_unchecked(&changed));
    assert!(!changed.compare.pairs.entries[0].included);
}

#[test]
fn editing_a_pair_selects_it_without_invalidating() {
    let (state, _) = act(checked_state(), CompareAction::EditPair(Some(ENTRY)));
    assert_eq!(state.compare.pairs.editing, Some(ENTRY));
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
    let (state, _) = act(state, CompareAction::EditPair(Some(42)));
    assert_eq!(
        state.compare.pairs.editing, None,
        "an unknown entry closes it"
    );
}

#[test]
fn overrides_that_change_nothing_keep_the_verdicts() {
    let (state, _) = act(
        checked_state(),
        CompareAction::SetPairOverrides(ENTRY, PairOverrides::default()),
    );
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
    let custom = PairOverrides {
        source_filter: Some("{\"x\": 1}".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(ENTRY, custom));
    assert_eq!(state.compare.preflight_view(), PreflightView::Unchecked);
}

#[test]
fn system_collections_are_a_helper_switch_not_an_edit() {
    let (state, _) = act(
        checked_state(),
        CompareAction::ToggleSystemCollections(true),
    );
    assert!(state.compare.pairing.include_system);
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}
