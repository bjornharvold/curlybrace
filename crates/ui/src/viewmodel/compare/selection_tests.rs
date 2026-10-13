use super::*;
use bson::Bson;
use curlybrace_core::compare::apply::kind_fits;
use std::cmp::Ordering;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

const SRC: Category = Category::OnlyInSource;
const TGT: Category = Category::OnlyInTarget;

#[test]
fn toggle_on_and_off() {
    let on = Selection::default().toggled(1, SRC, &k(1), &[]);
    assert!(on.is_selected(1, SRC, &k(1)));
    let off = on.toggled(1, SRC, &k(1), &[]);
    assert!(!off.is_selected(1, SRC, &k(1)));
    assert!(off.is_empty_for(1, SRC));
    assert!(on.is_selected(1, SRC, &k(1)), "original untouched");
}

#[test]
fn page_select_adds_keys() {
    let s = Selection::default().page_selected(1, SRC, &[k(1), k(2), k(3)]);
    assert_eq!(s.count(1, SRC, 1000), 3);
    assert!(s.is_selected(1, SRC, &k(2)));
}

#[test]
fn all_counts_total_without_enumerating() {
    let s = Selection::default().all_selected(1, SRC);
    assert_eq!(s.count(1, SRC, 1_000_000), 1_000_000);
    assert!(s.is_selected(1, SRC, &k(42)));
    assert!(!s.is_empty_for(1, SRC));
}

#[test]
fn all_clears_previous_keys() {
    let s = Selection::default()
        .toggled(1, SRC, &k(1), &[])
        .all_selected(1, SRC);
    let entry = &s.by[&(1, CatKey::from(SRC))];
    assert!(entry.all);
    assert!(entry.keys.is_empty());
}

#[test]
fn unticking_under_all_seeds_keys_with_loaded_minus_the_key() {
    let loaded = [k(1), k(2), k(3)];
    let s = Selection::default()
        .all_selected(1, SRC)
        .toggled(1, SRC, &k(2), &loaded);
    assert!(!s.is_selected(1, SRC, &k(2)));
    assert!(s.is_selected(1, SRC, &k(1)));
    assert!(s.is_selected(1, SRC, &k(3)));
    assert!(
        !s.is_selected(1, SRC, &k(99)),
        "unloaded rows become unselected"
    );
    assert_eq!(s.count(1, SRC, 1000), 2);
}

#[test]
fn ticking_when_not_under_all_ignores_loaded() {
    let s = Selection::default().toggled(1, SRC, &k(1), &[k(5), k(6)]);
    assert_eq!(s.count(1, SRC, 1000), 1);
}

#[test]
fn cleared_and_without_pair() {
    let s = Selection::default()
        .toggled(1, SRC, &k(1), &[])
        .toggled(1, TGT, &k(1), &[])
        .toggled(2, SRC, &k(1), &[]);
    let c = s.cleared(1, SRC);
    assert!(c.is_empty_for(1, SRC));
    assert!(!c.is_empty_for(1, TGT));
    let w = s.without_pair(1);
    assert!(w.is_empty_for(1, SRC) && w.is_empty_for(1, TGT));
    assert!(!w.is_empty_for(2, SRC));
}

#[test]
fn independent_per_pair_and_category() {
    let s = Selection::default().toggled(1, SRC, &k(1), &[]);
    assert!(!s.is_selected(2, SRC, &k(1)));
    assert!(!s.is_selected(1, TGT, &k(1)));
    assert!(!s.is_selected(1, Category::Different, &k(1)));
}

#[test]
fn ord_key_treats_numeric_types_as_equal() {
    let a = OrdKey(KeyTuple(vec![Bson::Int32(1)]));
    let b = OrdKey(KeyTuple(vec![Bson::Int64(1)]));
    assert_eq!(a, b);
    let s = Selection::default().toggled(1, SRC, &a.0, &[]);
    assert!(s.is_selected(1, SRC, &b.0));
}

#[test]
fn ord_key_orders_like_compare_tuples() {
    let keys = [k(1), k(2), KeyTuple(vec![Bson::Int32(2), Bson::Int32(0)])];
    for a in &keys {
        for b in &keys {
            let want = curlybrace_core::compare::key::compare_tuples(a, b);
            assert_eq!(OrdKey(a.clone()).cmp(&OrdKey(b.clone())), want);
        }
    }
    assert_eq!(OrdKey(k(1)).cmp(&OrdKey(k(2))), Ordering::Less);
}

#[test]
fn large_selection_lookups() {
    let keys: Vec<KeyTuple> = (0..100_000).map(k).collect();
    let s = Selection::default().page_selected(1, SRC, &keys);
    assert_eq!(s.count(1, SRC, 0), 100_000);
    for n in 0..100 {
        assert!(s.is_selected(1, SRC, &k(n * 997)));
    }
    assert!(!s.is_selected(1, SRC, &k(100_000)));
}

#[test]
fn offered_table() {
    use RowAction::*;
    assert_eq!(
        RowAction::offered_for(SRC),
        &[CopyToTarget, DeleteFromSource]
    );
    assert_eq!(
        RowAction::offered_for(TGT),
        &[CopyToSource, DeleteFromTarget]
    );
    assert_eq!(
        RowAction::offered_for(Category::Different),
        &[CopyToTarget, CopyToSource]
    );
}

#[test]
fn offered_iff_kind_fits() {
    let all = [
        RowAction::CopyToTarget,
        RowAction::CopyToSource,
        RowAction::DeleteFromSource,
        RowAction::DeleteFromTarget,
    ];
    let shapes = [
        (SRC, true, false),
        (TGT, false, true),
        (Category::Different, true, true),
    ];
    for (cat, s, t) in shapes {
        for a in all {
            let offered = RowAction::offered_for(cat).contains(&a);
            assert_eq!(
                offered,
                kind_fits(a.kind_for(cat), a.direction(), s, t),
                "{cat:?} {a:?}"
            );
        }
    }
}

#[test]
fn i18n_keys() {
    assert_eq!(
        RowAction::CopyToTarget.i18n_key(),
        "compare.apply.action.copy_to_target"
    );
    assert_eq!(
        RowAction::CopyToSource.i18n_key(),
        "compare.apply.action.copy_to_source"
    );
    assert_eq!(
        RowAction::DeleteFromSource.i18n_key(),
        "compare.apply.action.delete_from_source"
    );
    assert_eq!(
        RowAction::DeleteFromTarget.i18n_key(),
        "compare.apply.action.delete_from_target"
    );
}

fn nan() -> KeyTuple {
    KeyTuple(vec![Bson::Double(f64::NAN)])
}

#[test]
fn ord_key_total_order_across_mixed_numerics_and_nan() {
    let seq = [
        k(1),
        KeyTuple(vec![Bson::Int64(1)]),
        KeyTuple(vec![Bson::Double(1.0)]),
        k(2),
        KeyTuple(vec![Bson::Double(2.5)]),
        nan(),
        nan(),
    ];
    for a in &seq {
        for b in &seq {
            let ab = OrdKey(a.clone()).cmp(&OrdKey(b.clone()));
            let ba = OrdKey(b.clone()).cmp(&OrdKey(a.clone()));
            assert_eq!(ab, ba.reverse(), "{a:?} {b:?}");
        }
    }
    assert_eq!(OrdKey(nan()), OrdKey(nan()));
    let mut sorted: Vec<OrdKey> = seq.iter().cloned().map(OrdKey).collect();
    sorted.sort();
    assert!(sorted.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn field_actions_are_fields_ops_in_the_named_direction_for_any_category() {
    for cat in [SRC, TGT, Category::Different] {
        for a in RowAction::offered_for_fields() {
            assert_eq!(a.kind_for(cat), ApplyKind::Fields);
        }
    }
    assert_eq!(
        RowAction::CopyFieldsToTarget.direction(),
        Direction::SourceToTarget
    );
    assert_eq!(
        RowAction::CopyFieldsToSource.direction(),
        Direction::TargetToSource
    );
}

#[test]
fn field_actions_are_never_offered_for_whole_rows() {
    let fields = RowAction::offered_for_fields();
    assert_eq!(
        fields,
        &[RowAction::CopyFieldsToTarget, RowAction::CopyFieldsToSource]
    );
    for cat in [SRC, TGT, Category::Different] {
        assert!(
            RowAction::offered_for(cat)
                .iter()
                .all(|a| !fields.contains(a))
        );
    }
}

#[test]
fn field_action_keys_are_distinct_literals() {
    assert_eq!(
        RowAction::CopyFieldsToTarget.i18n_key(),
        "compare.apply.action.copy_fields_to_target"
    );
    assert_eq!(
        RowAction::CopyFieldsToSource.i18n_key(),
        "compare.apply.action.copy_fields_to_source"
    );
}
