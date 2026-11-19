use bson::{Bson, Document, doc};

use super::field_order::{FieldOrderFinding, check, summary_keys};

fn before_equality(range: &str, field: &str) -> Option<FieldOrderFinding> {
    Some(FieldOrderFinding::RangeBeforeEquality {
        range: range.into(),
        field: field.into(),
    })
}

fn before_sort(range: &str, field: &str) -> Option<FieldOrderFinding> {
    Some(FieldOrderFinding::RangeBeforeSort {
        range: range.into(),
        field: field.into(),
    })
}

#[test]
fn range_before_equality_is_found() {
    let keys = doc! { "createdAt": 1, "status": 1 };
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    assert_eq!(
        check(&keys, &filter, None),
        before_equality("createdAt", "status")
    );
}

#[test]
fn esr_order_is_fine() {
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    let sort = doc! { "name": 1 };
    let keys = doc! { "status": 1, "name": 1, "createdAt": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
    assert_eq!(
        check(&doc! { "status": 1, "createdAt": 1 }, &filter, None),
        None
    );
}

#[test]
fn range_before_sort_is_found() {
    let keys = doc! { "createdAt": 1, "name": 1 };
    let filter = doc! { "createdAt": { "$gt": 5 } };
    let sort = doc! { "name": 1 };
    assert_eq!(
        check(&keys, &filter, Some(&sort)),
        before_sort("createdAt", "name")
    );
}

#[test]
fn range_before_sort_needs_every_sort_key_in_the_used_keys() {
    // `other` is not an index key, so the server sorts in memory whatever the key order is: the
    // order of the keys is not what is wrong.
    let keys = doc! { "createdAt": 1, "name": 1 };
    let filter = doc! { "createdAt": { "$gt": 5 } };
    let sort = doc! { "name": 1, "other": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
    // A sort key past the first unused index key is not in the used keys either.
    let keys = doc! { "createdAt": 1, "name": 1, "gap": 1, "last": 1 };
    let sort = doc! { "name": 1, "last": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
}

#[test]
fn a_range_field_that_is_also_the_sort_is_not_wrong() {
    let keys = doc! { "createdAt": 1, "name": 1 };
    let filter = doc! { "createdAt": { "$gt": 5 } };
    let sort = doc! { "createdAt": 1, "name": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
    // Range first because the sort is on the range field: a legitimate choice.
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    let sort = doc! { "createdAt": -1 };
    let keys = doc! { "createdAt": 1, "status": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
}

#[test]
fn in_counts_as_equality() {
    let keys = doc! { "createdAt": 1, "status": 1 };
    let filter = doc! { "status": { "$in": ["a", "b"] }, "createdAt": { "$gt": 5 } };
    assert_eq!(
        check(&keys, &filter, None),
        before_equality("createdAt", "status")
    );
}

#[test]
fn in_with_a_sort_is_not_judged_for_range_first() {
    // A multi-value `$in` before the sort fields breaks the index order, so range-first can be
    // deliberate there: give no verdict rather than a false alarm.
    let keys = doc! { "createdAt": 1, "status": 1, "name": 1 };
    let filter = doc! { "status": { "$in": ["a", "b"] }, "createdAt": { "$gt": 5 } };
    let sort = doc! { "name": 1 };
    assert_eq!(check(&keys, &filter, Some(&sort)), None);
}

#[test]
fn an_in_filter_with_no_sort_and_esr_order_is_fine() {
    let keys = doc! { "status": 1, "createdAt": 1 };
    let filter = doc! { "status": { "$in": ["a"] }, "createdAt": { "$gt": 5 } };
    assert_eq!(check(&keys, &filter, None), None);
}

#[test]
fn ne_and_regex_count_as_range() {
    let keys = doc! { "a": 1, "b": 1 };
    for op in [doc! { "$ne": 1 }, doc! { "$regex": "^x" }] {
        let filter = doc! { "a": op, "b": 2 };
        assert_eq!(check(&keys, &filter, None), before_equality("a", "b"));
    }
}

fn pattern(text: &str) -> Bson {
    Bson::RegularExpression(bson::Regex {
        pattern: text.into(),
        options: String::new(),
    })
}

#[test]
fn a_regex_literal_is_a_range_not_an_exact_match() {
    let keys = doc! { "ts": 1, "name": 1 };
    let filter = doc! { "ts": { "$gt": 1 }, "name": pattern("foo") };
    assert_eq!(check(&keys, &filter, None), None);
    // With a real exact match after the range the order is still found.
    let keys = doc! { "ts": 1, "name": 1, "status": 1 };
    let filter = doc! { "ts": { "$gt": 1 }, "name": pattern("foo"), "status": "a" };
    assert_eq!(check(&keys, &filter, None), before_equality("ts", "status"));
}

#[test]
fn a_regex_in_an_in_list_is_a_range_not_an_exact_match() {
    let keys = doc! { "ts": 1, "name": 1 };
    let filter = doc! { "ts": { "$gt": 1 }, "name": { "$in": [pattern("^a")] } };
    assert_eq!(check(&keys, &filter, None), None);
    // The shape's marker for the same thing.
    let filter = doc! { "ts": { "$gt": 1 }, "name": { "$in": ["?", "$regex"] } };
    assert_eq!(check(&keys, &filter, None), None);
}

#[test]
fn the_regex_operator_form_is_still_a_range() {
    let keys = doc! { "ts": 1, "name": 1 };
    let filter = doc! { "ts": { "$gt": 1 }, "name": { "$regex": "foo" } };
    assert_eq!(check(&keys, &filter, None), None);
    let keys = doc! { "name": 1, "status": 1 };
    let filter = doc! { "name": { "$regex": "^a" }, "status": "a" };
    assert_eq!(
        check(&keys, &filter, None),
        before_equality("name", "status")
    );
}

#[test]
fn keys_after_the_first_unused_key_are_not_judged() {
    // `other` is not used by the query: what follows it cannot be judged.
    let keys = doc! { "createdAt": 1, "other": 1, "status": 1 };
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    assert_eq!(check(&keys, &filter, None), None);
    // A used key that is neither equality nor range (`$exists`) also ends the prefix.
    let keys = doc! { "createdAt": 1, "flag": 1, "status": 1 };
    let filter = doc! { "status": "a", "flag": { "$exists": true }, "createdAt": { "$gt": 5 } };
    assert_eq!(check(&keys, &filter, None), None);
}

#[test]
fn hashed_text_and_2dsphere_keys_are_not_judged() {
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    for keys in [
        doc! { "createdAt": "hashed", "status": 1 },
        doc! { "createdAt": 1, "status": "text" },
        doc! { "createdAt": 1, "status": "2dsphere" },
        doc! { "createdAt": 2, "status": 1 },
        doc! { "createdAt": 0, "status": 1 },
    ] {
        assert_eq!(check(&keys, &filter, None), None, "{keys:?}");
    }
}

#[test]
fn directions_of_minus_one_and_other_widths_are_plain() {
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 } };
    for keys in [
        doc! { "createdAt": -1, "status": 1 },
        doc! { "createdAt": 1_i64, "status": -1_i64 },
        doc! { "createdAt": 1.0, "status": 1.0 },
    ] {
        assert_eq!(
            check(&keys, &filter, None),
            before_equality("createdAt", "status"),
            "{keys:?}"
        );
    }
}

#[test]
fn a_single_key_index_is_never_wrong() {
    let filter = doc! { "createdAt": { "$gt": 5 }, "status": "a" };
    assert_eq!(check(&doc! { "createdAt": 1 }, &filter, None), None);
}

#[test]
fn an_empty_filter_with_a_sort_only_index_is_fine() {
    let sort = doc! { "name": 1 };
    assert_eq!(
        check(&doc! { "name": 1, "age": 1 }, &Document::new(), Some(&sort)),
        None
    );
    assert_eq!(
        check(&doc! { "a": 1, "b": 1 }, &Document::new(), None),
        None
    );
}

#[test]
fn a_filter_with_logical_operators_is_not_judged() {
    let keys = doc! { "createdAt": 1, "status": 1 };
    let filter = doc! { "$or": [{ "status": "a" }, { "createdAt": { "$gt": 5 } }] };
    assert_eq!(check(&keys, &filter, None), None);
    let filter = doc! { "status": "a", "createdAt": { "$gt": 5 }, "$and": [{ "x": 1 }] };
    assert_eq!(check(&keys, &filter, None), None);
}

#[test]
fn a_filter_field_with_a_literal_subdocument_is_equality() {
    let keys = doc! { "createdAt": 1, "meta": 1 };
    let filter = doc! { "meta": { "a": 1 }, "createdAt": { "$gt": 5 } };
    assert_eq!(
        check(&keys, &filter, None),
        before_equality("createdAt", "meta")
    );
}

#[test]
fn summary_keys_reads_one_ixscan() {
    assert_eq!(
        summary_keys("IXSCAN { a: 1, b: -1 }"),
        Some(doc! { "a": 1, "b": -1 })
    );
    assert_eq!(summary_keys("IXSCAN { a: 1 }"), Some(doc! { "a": 1 }));
    assert_eq!(
        summary_keys("EXPRESS_IXSCAN { a: 1 }"),
        Some(doc! { "a": 1 })
    );
}

#[test]
fn summary_keys_refuses_two_scans_and_odd_names() {
    for summary in [
        "IXSCAN { a: 1 }, IXSCAN { b: 1 }",
        "COLLSCAN",
        "",
        "IDHACK",
        "IXSCAN { a: 1, b: hashed }",
        "IXSCAN { a:b: 1 }",
        "IXSCAN { a: 1",
        "IXSCAN {}",
        "IXSCAN { a.b: 1, c: x }",
        "IXSCAN { \"a,b\": 1, c: 1 }",
        "IXSCAN { a: 1, b: 1 } extra { c: 1 }",
    ] {
        assert_eq!(summary_keys(summary), None, "{summary:?}");
    }
}

#[test]
fn summary_keys_keeps_direction_and_dotted_names() {
    assert_eq!(
        summary_keys("IXSCAN { user.name: -1, createdAt: 1 }"),
        Some(doc! { "user.name": -1, "createdAt": 1 })
    );
}
