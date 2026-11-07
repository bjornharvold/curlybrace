use super::{VerdictKind, worst};

const ALL: [VerdictKind; 7] = VerdictKind::ALL;

#[test]
fn code_round_trips_for_every_variant() {
    for kind in ALL {
        assert_eq!(VerdictKind::from_code(kind.code()), Some(kind));
    }
    let mut codes: Vec<&str> = ALL.iter().map(|k| k.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), ALL.len(), "codes are unique");
}

#[test]
fn codes_are_pinned_because_history_persists_them() {
    let pinned = [
        (VerdictKind::IndexUsed, "index_used"),
        (VerdictKind::CoveredByIndex, "covered_by_index"),
        (VerdictKind::CollectionScan, "collection_scan"),
        (VerdictKind::SortNotCovered, "sort_not_covered"),
        (VerdictKind::InefficientIndex, "inefficient_index"),
        (VerdictKind::NoFilter, "no_filter"),
        (VerdictKind::EmptyCollection, "empty_collection"),
    ];
    for (kind, code) in pinned {
        assert_eq!(kind.code(), code);
    }
}

#[test]
fn from_code_rejects_unknown() {
    assert_eq!(VerdictKind::from_code("nope"), None);
    assert_eq!(VerdictKind::from_code(""), None);
    assert_eq!(VerdictKind::from_code("IndexUsed"), None);
}

#[test]
fn severity_orders_kinds() {
    let order = [
        VerdictKind::NoFilter,
        VerdictKind::IndexUsed,
        VerdictKind::InefficientIndex,
        VerdictKind::SortNotCovered,
        VerdictKind::CollectionScan,
    ];
    for pair in order.windows(2) {
        assert!(pair[0].severity() < pair[1].severity(), "{pair:?}");
    }
    assert_eq!(
        VerdictKind::EmptyCollection.severity(),
        VerdictKind::NoFilter.severity()
    );
    assert_eq!(
        VerdictKind::CoveredByIndex.severity(),
        VerdictKind::IndexUsed.severity()
    );
}

#[test]
fn worst_picks_the_higher_severity() {
    assert_eq!(
        worst(VerdictKind::IndexUsed, VerdictKind::CollectionScan),
        VerdictKind::CollectionScan
    );
    assert_eq!(
        worst(VerdictKind::SortNotCovered, VerdictKind::InefficientIndex),
        VerdictKind::SortNotCovered
    );
    // A tie keeps the first argument.
    assert_eq!(
        worst(VerdictKind::IndexUsed, VerdictKind::CoveredByIndex),
        VerdictKind::IndexUsed
    );
    assert_eq!(
        worst(VerdictKind::CoveredByIndex, VerdictKind::IndexUsed),
        VerdictKind::CoveredByIndex
    );
}
