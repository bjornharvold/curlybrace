//! Turns a document into a canonical form under a set of [`CompareRules`], so that documents
//! that are equal under the rules are byte-for-byte equal afterwards. The input is never
//! modified. Order of operations: field scope, `_id`, numbers, then field and array order.

use std::collections::BTreeMap;

use bson::{Bson, Document};

use super::number::Dec;
use super::rules::{CompareRules, FieldScope};
use crate::error::{CoreError, Result};

#[derive(Default)]
struct PathTree {
    terminal: bool,
    children: BTreeMap<String, PathTree>,
}

impl PathTree {
    fn from_paths(paths: &[String]) -> Self {
        let mut root = Self::default();
        for path in paths {
            let mut node = &mut root;
            for part in path.split('.') {
                node = node.children.entry(part.to_string()).or_default();
            }
            node.terminal = true;
        }
        root
    }
}

fn drop_paths(doc: &Document, tree: &PathTree) -> Document {
    let mut out = Document::new();
    for (key, value) in doc {
        match tree.children.get(key) {
            None => {
                out.insert(key.clone(), value.clone());
            }
            Some(child) if child.terminal => {}
            Some(child) => {
                out.insert(key.clone(), drop_in_value(value, child));
            }
        }
    }
    out
}

fn drop_in_value(value: &Bson, child: &PathTree) -> Bson {
    match value {
        Bson::Document(d) => Bson::Document(drop_paths(d, child)),
        Bson::Array(items) => Bson::Array(items.iter().map(|i| drop_in_value(i, child)).collect()),
        other => other.clone(),
    }
}

fn keep_paths(doc: &Document, tree: &PathTree) -> Document {
    let mut out = Document::new();
    for (key, value) in doc {
        let Some(child) = tree.children.get(key) else {
            continue;
        };
        if child.terminal {
            out.insert(key.clone(), value.clone());
        } else if let Some(kept) = keep_in_value(value, child, false) {
            out.insert(key.clone(), kept);
        }
    }
    out
}

/// `in_array` keeps documents (possibly empty) so array positions stay aligned; a nested
/// document that matches nothing is dropped.
fn keep_in_value(value: &Bson, child: &PathTree, in_array: bool) -> Option<Bson> {
    match value {
        Bson::Document(d) => {
            let kept = keep_paths(d, child);
            (in_array || !kept.is_empty()).then_some(Bson::Document(kept))
        }
        Bson::Array(items) => {
            let kept: Vec<Bson> = items
                .iter()
                .filter_map(|i| keep_in_value(i, child, true))
                .collect();
            (!kept.is_empty()).then_some(Bson::Array(kept))
        }
        _ => None,
    }
}

/// The canonical value of a number when "numbers equal across types" is on.
fn canonical_number(value: &Bson) -> Bson {
    match value {
        Bson::Int32(n) => Bson::Int64(i64::from(*n)),
        Bson::Double(d) => {
            if d.is_nan() {
                Bson::Double(f64::NAN)
            } else if d.is_finite()
                && d.fract() == 0.0
                && *d >= -9_223_372_036_854_775_808.0
                && *d < 9_223_372_036_854_775_808.0
            {
                Bson::Int64(*d as i64)
            } else {
                Bson::Double(*d)
            }
        }
        Bson::Decimal128(d) => {
            let dec = Dec::of(d);
            if let Some(n) = dec.to_i64() {
                Bson::Int64(n)
            } else if let Some(canonical) = dec.to_decimal128() {
                Bson::Decimal128(canonical)
            } else {
                // NaN and the infinities have one representation each.
                value.clone()
            }
        }
        other => other.clone(),
    }
}

fn element_bytes(value: &Bson) -> Result<Vec<u8>> {
    let wrapped = Document::from_iter([("v".to_string(), value.clone())]);
    bson::to_vec(&wrapped)
        .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))
}

fn canonicalize(value: &Bson, rules: &CompareRules) -> Result<Bson> {
    Ok(match value {
        Bson::Document(d) => Bson::Document(canonical_doc(d, rules)?),
        Bson::Array(items) => {
            let mut out = items
                .iter()
                .map(|i| canonicalize(i, rules))
                .collect::<Result<Vec<_>>>()?;
            if rules.ignore_array_order {
                let mut keyed = out
                    .into_iter()
                    .map(|v| element_bytes(&v).map(|bytes| (bytes, v)))
                    .collect::<Result<Vec<_>>>()?;
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                out = keyed.into_iter().map(|(_, v)| v).collect();
            }
            Bson::Array(out)
        }
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_)
            if rules.numbers_equal_across_types =>
        {
            canonical_number(value)
        }
        other => other.clone(),
    })
}

fn canonical_doc(doc: &Document, rules: &CompareRules) -> Result<Document> {
    let mut pairs: Vec<(&String, &Bson)> = doc.iter().collect();
    if rules.ignore_field_order {
        pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    }
    pairs
        .into_iter()
        .map(|(k, v)| canonicalize(v, rules).map(|c| (k.clone(), c)))
        .collect()
}

/// The canonical form of `doc` under `rules`.
pub fn normalize(doc: &Document, rules: &CompareRules) -> Result<Document> {
    let scoped = match &rules.field_scope {
        FieldScope::All => doc.clone(),
        FieldScope::AllExcept(paths) => drop_paths(doc, &PathTree::from_paths(paths)),
        FieldScope::Only(paths) => keep_paths(doc, &PathTree::from_paths(paths)),
    };
    let scoped = if rules.include_id {
        scoped
    } else {
        let mut without = scoped;
        without.remove("_id");
        without
    };
    canonical_doc(&scoped, rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::key::KeySpec;
    use bson::{Bson, Decimal128, doc};
    use std::str::FromStr;

    fn rules() -> CompareRules {
        let mut r = CompareRules::for_key(&KeySpec::new(vec!["k".into()]).unwrap());
        r.include_id = true;
        r.ignore_field_order = false;
        r
    }

    fn n(doc: Document, r: &CompareRules) -> Document {
        normalize(&doc, r).expect("normalizes")
    }

    #[test]
    fn the_input_is_never_modified() {
        let original = doc! { "b": 1, "a": [3, 1, 2], "_id": 1 };
        let copy = original.clone();
        let mut r = rules();
        r.ignore_field_order = true;
        r.ignore_array_order = true;
        r.include_id = false;
        let _ = n(original.clone(), &r);
        assert_eq!(original, copy);
    }

    #[test]
    fn all_except_removes_paths_including_through_arrays() {
        let mut r = rules();
        r.field_scope = FieldScope::AllExcept(vec![
            "updatedAt".into(),
            "items.price".into(),
            "meta.sync".into(),
        ]);
        let d = doc! {
            "k": 1, "updatedAt": 5,
            "items": [{ "price": 1, "qty": 2 }, { "price": 3, "qty": 4 }, 7],
            "meta": { "sync": true, "owner": "x" },
        };
        assert_eq!(
            n(d, &r),
            doc! { "k": 1, "items": [{ "qty": 2 }, { "qty": 4 }, 7], "meta": { "owner": "x" } }
        );
    }

    #[test]
    fn a_terminal_path_wins_over_a_longer_one() {
        let mut r = rules();
        r.field_scope = FieldScope::AllExcept(vec!["a".into(), "a.b".into()]);
        assert_eq!(
            n(doc! { "k": 1, "a": { "b": 1, "c": 2 } }, &r),
            doc! { "k": 1 }
        );
    }

    #[test]
    fn only_keeps_just_the_listed_paths() {
        let mut r = rules();
        r.field_scope = FieldScope::Only(vec![
            "name".into(),
            "meta.owner".into(),
            "items.price".into(),
        ]);
        let d = doc! {
            "k": 1, "name": "n", "other": 1,
            "meta": { "owner": "x", "sync": true },
            "gone": { "z": 1 },
            "items": [{ "price": 1, "qty": 2 }, { "qty": 3 }, 7],
        };
        assert_eq!(
            n(d, &r),
            doc! { "name": "n", "meta": { "owner": "x" }, "items": [{ "price": 1 }, {}] }
        );
    }

    #[test]
    fn id_is_removed_unless_it_is_included() {
        let mut r = rules();
        r.include_id = false;
        assert_eq!(n(doc! { "_id": 9, "k": 1 }, &r), doc! { "k": 1 });
        r.include_id = true;
        assert_eq!(n(doc! { "_id": 9, "k": 1 }, &r), doc! { "_id": 9, "k": 1 });
    }

    #[test]
    fn numbers_collapse_only_when_asked() {
        let mut r = rules();
        let d = doc! { "k": 1, "a": 1_i32, "b": 1_i64, "c": 1.0_f64, "d": 1.5_f64 };
        let strict = n(d.clone(), &r);
        assert_eq!(strict.get("a"), Some(&Bson::Int32(1)));
        r.numbers_equal_across_types = true;
        let loose = n(d, &r);
        for field in ["a", "b", "c"] {
            assert_eq!(loose.get(field), Some(&Bson::Int64(1)), "{field}");
        }
        assert_eq!(loose.get("d"), Some(&Bson::Double(1.5)));
    }

    #[test]
    fn number_collapsing_is_exact() {
        let mut r = rules();
        r.numbers_equal_across_types = true;
        let dec = |s: &str| Bson::Decimal128(Decimal128::from_str(s).unwrap());
        let out = n(
            doc! { "k": 1, "i": dec("12.0"), "f": dec("1.50"), "g": dec("1.5"), "z": -0.0_f64, "big": 1e30_f64, "nan": f64::NAN },
            &r,
        );
        assert_eq!(out.get("i"), Some(&Bson::Int64(12)));
        // Equal decimals written differently become identical values.
        assert_eq!(out.get("f"), out.get("g"));
        assert_eq!(out.get("z"), Some(&Bson::Int64(0)));
        // A double is never equal to a decimal unless the decimal is exactly its value.
        assert_eq!(out.get("big"), Some(&Bson::Double(1e30)));
        match out.get("nan") {
            Some(Bson::Double(x)) => assert!(x.is_nan()),
            other => panic!("{other:?}"),
        }
        let tenth = n(doc! { "k": 1, "d": dec("0.1"), "x": 0.1_f64 }, &r);
        assert_ne!(tenth.get("d"), tenth.get("x"));
    }

    #[test]
    fn field_order_is_sorted_recursively_only_when_ignored() {
        let mut r = rules();
        let d = doc! { "k": 1, "b": { "y": 1, "x": 2 }, "a": 0 };
        assert_eq!(n(d.clone(), &r).keys().collect::<Vec<_>>(), ["k", "b", "a"]);
        r.ignore_field_order = true;
        let sorted = n(d, &r);
        assert_eq!(sorted.keys().collect::<Vec<_>>(), ["a", "b", "k"]);
        assert_eq!(
            sorted.get_document("b").unwrap().keys().collect::<Vec<_>>(),
            ["x", "y"]
        );
    }

    #[test]
    fn array_order_is_ignored_for_scalars_and_documents() {
        let mut r = rules();
        r.ignore_field_order = true;
        let a = doc! { "k": 1, "xs": [3, 1, 2], "ds": [{ "a": 2 }, { "a": 1 }] };
        let b = doc! { "k": 1, "xs": [2, 3, 1], "ds": [{ "a": 1 }, { "a": 2 }] };
        assert_ne!(
            n(a.clone(), &r),
            n(b.clone(), &r),
            "order matters by default"
        );
        r.ignore_array_order = true;
        assert_eq!(n(a, &r), n(b, &r));
    }

    #[test]
    fn array_order_sorting_applies_inside_nested_arrays() {
        let mut r = rules();
        r.ignore_array_order = true;
        let a = doc! { "k": 1, "m": [[2, 1], [4, 3]] };
        let b = doc! { "k": 1, "m": [[3, 4], [1, 2]] };
        assert_eq!(n(a, &r), n(b, &r));
    }
}
