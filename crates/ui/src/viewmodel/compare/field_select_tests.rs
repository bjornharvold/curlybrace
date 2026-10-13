use super::*;
use bson::Bson;

fn f(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

fn p(names: &[&str]) -> Vec<PathSegment> {
    names.iter().map(|n| f(n)).collect()
}

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

#[test]
fn toggling_adds_then_removes_a_path() {
    let s = FieldSelection::default().toggled(1, &k(1), &p(&["a"]));
    assert!(s.is_selected(1, &k(1), &p(&["a"])));
    assert_eq!(s.count(1, &k(1)), 1);
    let s = s.toggled(1, &k(1), &p(&["a"]));
    assert!(!s.is_selected(1, &k(1), &p(&["a"])));
    assert_eq!(s, FieldSelection::default(), "an emptied entry is dropped");
}

#[test]
fn paths_come_back_in_a_stable_order_whatever_the_toggle_order() {
    let a = FieldSelection::default()
        .toggled(1, &k(1), &p(&["b"]))
        .toggled(1, &k(1), &p(&["a", "z"]))
        .toggled(1, &k(1), &p(&["a"]));
    let b = FieldSelection::default()
        .toggled(1, &k(1), &p(&["a"]))
        .toggled(1, &k(1), &p(&["a", "z"]))
        .toggled(1, &k(1), &p(&["b"]));
    assert_eq!(a.paths(1, &k(1)), b.paths(1, &k(1)));
    assert_eq!(
        a.paths(1, &k(1)),
        vec![p(&["a"]), p(&["a", "z"]), p(&["b"])]
    );
}

#[test]
fn selections_are_isolated_per_pair_and_key() {
    let s = FieldSelection::default()
        .toggled(1, &k(1), &p(&["a"]))
        .toggled(1, &k(2), &p(&["b"]))
        .toggled(2, &k(1), &p(&["c"]));
    assert_eq!(s.paths(1, &k(1)), vec![p(&["a"])]);
    assert_eq!(s.paths(1, &k(2)), vec![p(&["b"])]);
    assert_eq!(s.paths(2, &k(1)), vec![p(&["c"])]);
    assert!(s.paths(3, &k(1)).is_empty());
    assert_eq!(s.count(2, &k(2)), 0);
}

#[test]
fn cleared_drops_only_that_document() {
    let s = FieldSelection::default()
        .toggled(1, &k(1), &p(&["a"]))
        .toggled(1, &k(2), &p(&["b"]))
        .cleared(1, &k(1));
    assert_eq!(s.count(1, &k(1)), 0);
    assert_eq!(s.count(1, &k(2)), 1);
    assert_eq!(s.cleared(9, &k(9)), s, "clearing nothing changes nothing");
}

#[test]
fn without_pair_drops_every_document_of_the_pair() {
    let s = FieldSelection::default()
        .toggled(1, &k(1), &p(&["a"]))
        .toggled(1, &k(2), &p(&["b"]))
        .toggled(2, &k(1), &p(&["c"]))
        .without_pair(1);
    assert_eq!(s.count(1, &k(1)), 0);
    assert_eq!(s.count(1, &k(2)), 0);
    assert_eq!(s.count(2, &k(1)), 1);
}

#[test]
fn int32_and_int64_keys_of_equal_value_are_one_document() {
    let wide = KeyTuple(vec![Bson::Int64(1)]);
    let s = FieldSelection::default().toggled(1, &k(1), &p(&["a"]));
    assert!(s.is_selected(1, &wide, &p(&["a"])));
    assert_eq!(s.toggled(1, &wide, &p(&["a"])), FieldSelection::default());
}

#[test]
fn a_nan_key_finds_its_own_selection() {
    let nan = KeyTuple(vec![Bson::Double(f64::NAN)]);
    let s = FieldSelection::default().toggled(1, &nan, &p(&["a"]));
    assert!(s.is_selected(1, &nan.clone(), &p(&["a"])));
    assert_eq!(s.count(1, &nan.clone()), 1);
    assert_eq!(s.cleared(1, &nan.clone()), FieldSelection::default());
}

fn state_with_selection() -> super::super::CompareState {
    super::super::CompareState {
        field_selection: FieldSelection::default()
            .toggled(1, &k(1), &p(&["a"]))
            .toggled(2, &k(1), &p(&["b"])),
        ..Default::default()
    }
}

#[test]
fn a_dropped_session_forgets_the_field_selection() {
    let s = state_with_selection().session_dropped();
    assert_eq!(s.field_selection, FieldSelection::default());
}

#[test]
fn discarding_results_drops_only_those_pairs_fields() {
    let s = state_with_selection().without_results(&[1]);
    assert_eq!(s.field_selection.count(1, &k(1)), 0);
    assert_eq!(s.field_selection.count(2, &k(1)), 1);
}

#[test]
fn path_key_order_is_field_then_index_then_any_element() {
    let field = PathKey(vec![f("z")]);
    let index = PathKey(vec![PathSegment::Index(0)]);
    let any = PathKey(vec![PathSegment::AnyElement]);
    assert!(field < index && index < any);
    assert!(PathKey(vec![PathSegment::Index(2)]) < PathKey(vec![PathSegment::Index(10)]));
    assert!(PathKey(vec![f("a")]) < PathKey(vec![f("b")]));
}

#[test]
fn path_key_order_is_lexicographic_and_total() {
    let all = vec![
        vec![],
        vec![f("a")],
        vec![f("a"), PathSegment::Index(0)],
        vec![f("a"), PathSegment::AnyElement],
        vec![f("a"), f("b")],
        vec![f("b")],
        vec![PathSegment::Index(1)],
    ];
    assert!(
        PathKey(vec![]) < PathKey(vec![f("a")]),
        "a prefix sorts first"
    );
    for x in &all {
        for y in &all {
            let (a, b) = (PathKey(x.clone()), PathKey(y.clone()));
            assert_eq!(a.cmp(&b), b.cmp(&a).reverse(), "{x:?} vs {y:?}");
            assert_eq!(a == b, x == y);
            for z in &all {
                let c = PathKey(z.clone());
                if a <= b && b <= c {
                    assert!(a <= c, "transitive {x:?} {y:?} {z:?}");
                }
            }
        }
    }
}
