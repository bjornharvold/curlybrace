use super::*;
use crate::viewmodel::compare::setup::SideForm;
use crate::viewmodel::compare::{CompareState, SessionPhase};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::store::CollectionKind;

fn coll(names: &[&str]) -> Vec<(String, CollectionKind)> {
    names
        .iter()
        .map(|n| (n.to_string(), CollectionKind::Collection))
        .collect()
}

fn state(source: &[&str], target: &[&str]) -> CompareState {
    let side = |db: &str, names: &[&str]| {
        SideForm::default()
            .with_database(db)
            .with_collections(db, coll(names))
    };
    CompareState {
        session: Some(SessionPhase::Ready(1)),
        setup: SetupForm::default()
            .with_side(Side::Source, side("a", source))
            .with_side(Side::Target, side("b", target)),
        ..CompareState::default()
    }
}

#[test]
fn the_list_view_counts_what_add_all_would_add_and_lists_the_unmatched_names() {
    let s = state(&["x", "y", "only_s"], &["x", "y", "only_t"]);
    let view = ListView::of(&s);
    assert_eq!(view.addable, 2);
    assert_eq!(
        view.unmatched,
        vec![
            Unmatched {
                side: Side::Source,
                name: "only_s".into()
            },
            Unmatched {
                side: Side::Target,
                name: "only_t".into()
            },
        ]
    );
    assert!(view.can_add_all());
}

#[test]
fn already_paired_names_are_not_added_again() {
    let mut s = state(&["x"], &["x"]);
    s.pairs = s.pairs.added(
        curlybrace_core::store::Namespace::new("a", "x"),
        curlybrace_core::store::Namespace::new("b", "x"),
    );
    let view = ListView::of(&s);
    assert_eq!(view.addable, 0);
    assert!(!view.can_add_all());
}

#[test]
fn the_system_switch_decides_whether_system_collections_are_offered() {
    let mut s = state(&["system.views"], &["system.views"]);
    assert_eq!(ListView::of(&s).addable, 0);
    s.pairing.include_system = true;
    assert_eq!(ListView::of(&s).addable, 1);
}

#[test]
fn included_rows_are_the_indices_of_the_included_entries() {
    let mut s = state(&[], &[]);
    let ns = |db: &str, c: &str| curlybrace_core::store::Namespace::new(db, c);
    s.pairs = s
        .pairs
        .added(ns("a", "x"), ns("b", "x"))
        .added(ns("a", "y"), ns("b", "y"))
        .added(ns("a", "z"), ns("b", "z"))
        .included(1, false);
    assert_eq!(ListView::of(&s).included, vec![0, 2]);
}

#[test]
fn the_view_is_current_only_for_the_state_it_was_built_from() {
    let s = state(&["x"], &["x"]);
    let inputs = ListInputs::of(&s);
    assert!(inputs.is_for(&s));
    let edited = CompareState {
        setup: s.setup.clone().with_key_text("sku"),
        ..s.clone()
    };
    assert!(!inputs.is_for(&edited));
    let toggled = CompareState {
        pairing: crate::viewmodel::compare::DatabasePairing {
            include_system: true,
        },
        ..s
    };
    assert!(!inputs.is_for(&toggled));
}

#[test]
fn can_add_pair_needs_both_picks_and_a_pair_not_yet_in_the_list() {
    let mut s = state(&["x"], &["x"]);
    assert!(!s.can_add_pair(), "no collection picked");
    s.setup = s
        .setup
        .clone()
        .with_side(Side::Source, s.setup.source.clone().with_collection("x"))
        .with_side(Side::Target, s.setup.target.clone().with_collection("x"));
    assert!(s.can_add_pair());
    let (src, tgt) = s.setup.pick_pair().unwrap();
    s.pairs = s.pairs.added(src, tgt);
    assert!(!s.can_add_pair(), "already in the list");
}

#[test]
fn pairing_an_unmatched_name_needs_text() {
    assert!(can_pair_with("orders"));
    assert!(!can_pair_with("   "));
    assert!(!can_pair_with(""));
}
