use super::*;
use curlybrace_core::store::Namespace;

fn ns(db: &str, c: &str) -> Namespace {
    Namespace::new(db.to_string(), c.to_string())
}

fn list() -> PairList {
    PairList::default()
        .added(ns("a", "x"), ns("b", "x"))
        .added(ns("a", "y"), ns("b", "y"))
        .added(ns("a", "z"), ns("a", "z"))
}

#[test]
fn every_entry_has_its_first_issue_and_valid_means_included_without_issue() {
    let list = list().included(1, false);
    let digest = PairsDigest::of(&list, &SetupForm::default(), true);
    assert_eq!(digest.issues.len(), 3);
    assert_eq!(digest.issue_at(0), None);
    // Same collection on one connection.
    assert_eq!(digest.issue_at(2), Some(SetupError::SameCollection));
    // Entry 1 is fine but not included.
    assert_eq!(digest.valid, vec![0]);
}

#[test]
fn a_bad_default_makes_every_entry_invalid() {
    let defaults = SetupForm::default().with_key_text("a,,b");
    let digest = PairsDigest::of(&list(), &defaults, false);
    assert!(digest.valid.is_empty());
    assert_eq!(digest.issue_at(0), Some(SetupError::Key));
}

#[test]
fn another_connection_lifts_the_same_collection_issue() {
    let digest = PairsDigest::of(&list(), &SetupForm::default(), false);
    assert_eq!(digest.valid, vec![0, 1, 2]);
    assert_eq!(digest.issue_at(9), None);
}
