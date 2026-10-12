//! Pure helpers behind `MongoStore::scan_sorted` and `key_index_status`, kept free of the driver
//! so they are unit-testable.
//!
//! MongoDB's `$gt` and `$gte` only match values of the SAME type class (numbers with numbers,
//! strings with strings), while its sort order is total across classes. A plain `$gt` keyset
//! would therefore skip everything after a change of type class. Each key field's condition is
//! "at or after `v` within its class, OR of a higher class", where the higher classes are a
//! POSITIVE `$type` list of the classes ranked above v's: the planner turns every alternative
//! into index bounds, so each batch is an index range scan starting at `v` (a negated `$type`
//! cannot be bounded and rescans the sort index from its start every batch).
//!
//! The filter is flattened into one `$or` of conjunctions ("prefix equal AND this alternative"),
//! so every branch is bounded on the whole compound index and the branches merge in index order.
//! `array` is never listed: `$type: "array"` cannot be bounded, and an array-valued key is
//! refused by `extract_key` anyway (it still matches a listed class through its elements).

use bson::{Bson, Document, doc};

use super::key::{KeySpec, KeyTuple, type_rank};
use super::number::Dec;

const CLASSES: [&[&str]; 16] = [
    &["minKey"],
    &["null", "undefined"],
    &["int", "long", "double", "decimal"],
    &["string", "symbol"],
    &["object"],
    &["array"],
    &["binData"],
    &["objectId"],
    &["bool"],
    &["date"],
    &["timestamp"],
    &["regex"],
    &["dbPointer"],
    &["javascript"],
    &["javascriptWithScope"],
    &["maxKey"],
];

/// The `$type` names of every class ranked strictly above `rank` (see `key::type_rank`),
/// without `array` (see the module docs). Empty for `maxKey`.
pub fn higher_class_names(rank: u8) -> &'static [&'static str] {
    // Built once on first use: a rank-indexed suffix of CLASSES, flattened.
    static TABLE: std::sync::OnceLock<Vec<Vec<&'static str>>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..CLASSES.len())
            .map(|r| {
                CLASSES[r + 1..]
                    .iter()
                    .flat_map(|c| c.iter().copied())
                    .filter(|name| *name != "array")
                    .collect()
            })
            .collect()
    });
    let idx = usize::from(rank).min(table.len() - 1);
    &table[idx]
}

fn is_nan(value: &Bson) -> bool {
    match value {
        Bson::Double(d) => d.is_nan(),
        Bson::Decimal128(d) => Dec::of(d) == Dec::NaN,
        _ => false,
    }
}

/// "`field` is at or after `value` within its own class" (`inclusive`), or strictly after.
/// `None` when nothing of the class qualifies.
fn own_class(field: &str, value: &Bson, inclusive: bool) -> Option<Document> {
    if is_nan(value) {
        // The server sorts NaN below every number, but `$gt: NaN` matches nothing and
        // `$gte: NaN` only NaN. At or after NaN is every number; strictly after is every
        // number that is not NaN.
        return Some(if inclusive {
            doc! { field: { "$type": "number" } }
        } else {
            doc! { field: { "$type": "number", "$ne": value.clone() } }
        });
    }
    match value {
        // `null`, `minKey` and `maxKey` only compare equal to themselves in a query, so an
        // exclusive start has nothing in its own class.
        Bson::Null | Bson::Undefined | Bson::MinKey | Bson::MaxKey => {
            inclusive.then(|| doc! { field: value.clone() })
        }
        _ => {
            let operator = if inclusive { "$gte" } else { "$gt" };
            Some(doc! { field: { operator: value.clone() } })
        }
    }
}

/// The alternatives for "`field` is after `value`": the in-class comparison and the positive
/// list of every higher class. Each is a single-field condition, so it can be bounded.
fn after(field: &str, value: &Bson, inclusive: bool) -> Vec<Document> {
    let higher: Vec<Bson> = higher_class_names(type_rank(value))
        .iter()
        .map(|n| Bson::String((*n).into()))
        .collect();
    let higher = (!higher.is_empty()).then(|| doc! { field: { "$type": higher } });
    own_class(field, value, inclusive)
        .into_iter()
        .chain(higher)
        .collect()
}

/// The keyset condition alone: a flat `$or` of "the first i fields equal, field i after".
fn keyset_branches(key: &KeySpec, from: &KeyTuple) -> Vec<Document> {
    let last = key.fields.len() - 1;
    let mut branches = Vec::new();
    for i in 0..key.fields.len() {
        let prefix: Document = key.fields[..i]
            .iter()
            .zip(&from.0)
            .map(|(field, value)| (field.clone(), Bson::Document(doc! { "$eq": value.clone() })))
            .collect();
        for alternative in after(&key.fields[i], &from.0[i], i == last) {
            let mut branch = prefix.clone();
            branch.extend(alternative);
            branches.push(branch);
        }
    }
    branches
}

/// The filter for "documents matching `user_filter` whose key is at or after `from`".
pub fn keyset_filter(user_filter: &Document, key: &KeySpec, from: Option<&KeyTuple>) -> Document {
    let Some(from) = from else {
        return user_filter.clone();
    };
    if from.0.len() != key.fields.len() {
        // A caller bug: the start tuple must have one value per key field.
        debug_assert!(false, "start key arity differs from the key spec");
        return user_filter.clone();
    }
    let mut branches = keyset_branches(key, from);
    // Never empty: the last field is inclusive, so its own class always has a branch.
    let keyset = if branches.len() == 1 {
        branches.remove(0)
    } else {
        doc! { "$or": branches }
    };
    if user_filter.is_empty() {
        keyset
    } else {
        doc! { "$and": [user_filter.clone(), keyset] }
    }
}

/// What `key_index_status` needs to know about one index.
#[derive(Clone, Debug)]
pub struct IndexInfo {
    pub keys: Document,
    pub sparse: bool,
    pub has_partial_filter: bool,
    /// A hidden index is invisible to the query planner, so it cannot serve the sort.
    pub hidden: bool,
    /// The index sorts by simple (binary) collation: it has an explicit non-simple collation
    /// (`false`) or either an explicit simple one or none at all (`true`; the latter inherits the
    /// collection default, which `index_serves_sort` checks separately).
    pub collation_is_simple: bool,
}

/// Whether `index` can serve `sort(key fields ascending)` under simple collation: a plain
/// (not hidden, sparse, partial, hashed or text) index whose leading fields are the key fields in order,
/// all in the same direction (a uniformly reversed index is read backwards), with no
/// non-simple collation.
pub fn index_serves_sort(
    index: &IndexInfo,
    collection_default_is_simple: bool,
    key: &KeySpec,
) -> bool {
    if index.sparse || index.has_partial_filter || index.hidden {
        return false;
    }
    // An index without a collation inherits the collection's default, so a non-simple default
    // disqualifies it. An explicit simple index on a non-simple collection is also refused:
    // `collation_is_simple` cannot tell it apart, and refusing is the safe direction.
    if !index.collation_is_simple || !collection_default_is_simple {
        return false;
    }
    if key.fields.len() > index.keys.len() {
        return false;
    }
    let mut direction: Option<i64> = None;
    for ((name, value), wanted) in index.keys.iter().zip(&key.fields) {
        if name != wanted {
            return false;
        }
        let this = match value {
            Bson::Int32(n) => i64::from(*n),
            Bson::Int64(n) => *n,
            Bson::Double(d) if *d == 1.0 || *d == -1.0 => *d as i64,
            _ => return false,
        };
        if this != 1 && this != -1 {
            return false;
        }
        match direction {
            None => direction = Some(this),
            Some(d) if d != this => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, doc};

    fn key(fields: &[&str]) -> KeySpec {
        KeySpec::new(fields.iter().map(|s| s.to_string()).collect()).unwrap()
    }

    #[test]
    fn with_no_start_the_filter_is_the_users_filter() {
        let f = keyset_filter(&doc! { "tenant": "a" }, &key(&["_id"]), None);
        assert_eq!(f, doc! { "tenant": "a" });
        assert_eq!(keyset_filter(&doc! {}, &key(&["_id"]), None), doc! {});
    }

    /// The classes above numbers, as a `$type` list.
    fn above_numbers() -> Vec<&'static str> {
        higher_class_names(2).to_vec()
    }

    #[test]
    fn a_single_key_is_at_or_after_or_of_a_higher_type_class() {
        let from = KeyTuple(vec![Bson::Int32(5)]);
        let f = keyset_filter(&doc! {}, &key(&["_id"]), Some(&from));
        // _id >= 5 among numbers, OR the value is of a type class ranked above numbers.
        assert_eq!(
            f,
            doc! { "$or": [
                { "_id": { "$gte": 5 } },
                { "_id": { "$type": above_numbers() } },
            ] }
        );
    }

    #[test]
    fn the_user_filter_is_combined_with_and() {
        let from = KeyTuple(vec![Bson::String("m".into())]);
        let f = keyset_filter(&doc! { "tenant": "a" }, &key(&["k"]), Some(&from));
        assert_eq!(f.get_array("$and").unwrap().len(), 2);
        assert_eq!(
            f.get_array("$and").unwrap()[0],
            Bson::Document(doc! { "tenant": "a" })
        );
    }

    #[test]
    fn a_compound_key_is_a_flat_or_of_conjunctions_with_an_inclusive_last_field() {
        let from = KeyTuple(vec![Bson::Int32(1), Bson::String("x".into())]);
        let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
        assert_eq!(
            f,
            doc! { "$or": [
                // a is strictly greater, or of a higher class.
                { "a": { "$gt": 1 } },
                { "a": { "$type": above_numbers() } },
                // a equals, and b is at or after.
                { "a": { "$eq": 1 }, "b": { "$gte": "x" } },
                { "a": { "$eq": 1 }, "b": { "$type": higher_class_names(3).to_vec() } },
            ] }
        );
    }

    #[test]
    fn the_higher_classes_are_every_class_above_without_array() {
        assert_eq!(higher_class_names(14), ["maxKey"]);
        assert!(higher_class_names(15).is_empty(), "nothing is above maxKey");
        assert_eq!(
            &higher_class_names(0)[..5],
            ["null", "undefined", "int", "long", "double"]
        );
        for r in 0..=15u8 {
            let names = higher_class_names(r);
            assert!(!names.contains(&"array"), "rank {r}");
            assert!(!names.contains(&"minKey"), "rank {r}");
            if r < 15 {
                assert!(names.contains(&"maxKey"), "rank {r}");
                assert!(names.len() <= higher_class_names(r.saturating_sub(1)).len());
            }
        }
        assert!(
            !higher_class_names(3).contains(&"string"),
            "not its own class"
        );
        assert!(higher_class_names(3).contains(&"object"));
    }

    #[test]
    fn a_maxkey_start_has_only_its_own_class() {
        let from = KeyTuple(vec![Bson::MaxKey]);
        let f = keyset_filter(&doc! {}, &key(&["k"]), Some(&from));
        assert_eq!(f, doc! { "k": Bson::MaxKey });
    }

    #[test]
    fn a_null_prefix_has_no_strictly_greater_null() {
        let from = KeyTuple(vec![Bson::Null, Bson::Int32(1)]);
        let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
        let branches = f.get_array("$or").unwrap();
        assert_eq!(
            branches[0],
            Bson::Document(doc! { "a": { "$type": higher_class_names(1).to_vec() } })
        );
        assert_eq!(branches.len(), 3);
    }

    #[test]
    fn a_nan_start_matches_every_number_when_inclusive() {
        let from = KeyTuple(vec![Bson::Double(f64::NAN)]);
        let f = keyset_filter(&doc! {}, &key(&["k"]), Some(&from));
        assert_eq!(
            f,
            doc! { "$or": [
                { "k": { "$type": "number" } },
                { "k": { "$type": above_numbers() } },
            ] }
        );
    }

    #[test]
    fn a_nan_prefix_excludes_nan_and_is_matched_by_equality() {
        let nan = Bson::Decimal128("NaN".parse().unwrap());
        let from = KeyTuple(vec![nan.clone(), Bson::Int32(1)]);
        let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
        let branches = f.get_array("$or").unwrap();
        assert_eq!(
            branches[0],
            Bson::Document(doc! { "a": { "$type": "number", "$ne": nan.clone() } })
        );
        assert_eq!(
            branches[1],
            Bson::Document(doc! { "a": { "$type": above_numbers() } })
        );
        match &branches[2] {
            Bson::Document(d) => assert_eq!(d.get_document("a").unwrap(), &doc! { "$eq": nan }),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prefix_equality_uses_eq_so_document_values_are_not_read_as_operators() {
        let from = KeyTuple(vec![Bson::Document(doc! { "$x": 1 }), Bson::Int32(1)]);
        let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
        let branches = f.get_array("$or").unwrap();
        match &branches[2] {
            Bson::Document(d) => {
                assert_eq!(d.get_document("a").unwrap(), &doc! { "$eq": { "$x": 1 } })
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_mismatched_start_key_falls_back_to_the_users_filter_in_release() {
        if cfg!(debug_assertions) {
            return;
        }
        let from = KeyTuple(vec![Bson::Int32(1)]);
        let f = keyset_filter(&doc! { "t": 1 }, &key(&["a", "b"]), Some(&from));
        assert_eq!(f, doc! { "t": 1 });
    }

    /// `DbPointer` has no public constructor, so decode one from raw BSON.
    fn db_pointer() -> Bson {
        let mut raw: Vec<u8> = vec![26, 0, 0, 0, 0x0C, b'p', 0, 2, 0, 0, 0, b'c', 0];
        raw.extend([0u8; 12]);
        raw.push(0);
        Document::from_reader(raw.as_slice())
            .unwrap()
            .get("p")
            .unwrap()
            .clone()
    }

    #[test]
    fn every_variants_type_name_sits_in_its_own_rank() {
        use bson::oid::ObjectId;
        use bson::spec::BinarySubtype;
        let samples: Vec<(Bson, &str)> = vec![
            (Bson::MinKey, "minKey"),
            (Bson::Null, "null"),
            (Bson::Undefined, "undefined"),
            (Bson::Int32(1), "int"),
            (Bson::Int64(1), "long"),
            (Bson::Double(1.0), "double"),
            (Bson::Decimal128("1".parse().unwrap()), "decimal"),
            (Bson::String("s".into()), "string"),
            (Bson::Symbol("s".into()), "symbol"),
            (Bson::Document(doc! {}), "object"),
            (Bson::Array(vec![]), "array"),
            (
                Bson::Binary(bson::Binary {
                    subtype: BinarySubtype::Generic,
                    bytes: vec![],
                }),
                "binData",
            ),
            (Bson::ObjectId(ObjectId::from_bytes([0; 12])), "objectId"),
            (Bson::Boolean(true), "bool"),
            (Bson::DateTime(bson::DateTime::from_millis(0)), "date"),
            (
                Bson::Timestamp(bson::Timestamp {
                    time: 0,
                    increment: 1,
                }),
                "timestamp",
            ),
            (
                Bson::RegularExpression(bson::Regex {
                    pattern: "a".into(),
                    options: String::new(),
                }),
                "regex",
            ),
            (db_pointer(), "dbPointer"),
            (Bson::JavaScriptCode("x".into()), "javascript"),
            (
                Bson::JavaScriptCodeWithScope(bson::JavaScriptCodeWithScope {
                    code: "x".into(),
                    scope: doc! {},
                }),
                "javascriptWithScope",
            ),
            (Bson::MaxKey, "maxKey"),
        ];
        for (value, name) in samples {
            let rank = type_rank(&value);
            assert!(
                !higher_class_names(rank).contains(&name),
                "{value:?} is not above its own class {name}"
            );
            if rank > 0 && name != "array" {
                assert!(
                    higher_class_names(rank - 1).contains(&name),
                    "{value:?} is above the class below {name}"
                );
            }
        }
    }

    fn index(keys: Document) -> IndexInfo {
        IndexInfo {
            keys,
            sparse: false,
            has_partial_filter: false,
            hidden: false,
            collation_is_simple: true,
        }
    }

    #[test]
    fn an_index_serves_a_prefix_in_the_same_or_the_reverse_direction() {
        assert!(index_serves_sort(
            &index(doc! { "a": 1 }),
            true,
            &key(&["a"])
        ));
        assert!(
            index_serves_sort(&index(doc! { "a": -1 }), true, &key(&["a"])),
            "reverse scan"
        );
        assert!(index_serves_sort(
            &index(doc! { "a": 1, "b": 1, "c": 1 }),
            true,
            &key(&["a", "b"])
        ));
        assert!(index_serves_sort(
            &index(doc! { "a": -1, "b": -1 }),
            true,
            &key(&["a", "b"])
        ));
        assert!(
            !index_serves_sort(&index(doc! { "a": 1, "b": -1 }), true, &key(&["a", "b"])),
            "mixed directions"
        );
        assert!(
            !index_serves_sort(&index(doc! { "b": 1, "a": 1 }), true, &key(&["a", "b"])),
            "wrong order"
        );
        assert!(
            !index_serves_sort(&index(doc! { "a": 1 }), true, &key(&["a", "b"])),
            "too short"
        );
    }

    #[test]
    fn special_indexes_cannot_serve_the_sort() {
        assert!(!index_serves_sort(
            &index(doc! { "a": "hashed" }),
            true,
            &key(&["a"])
        ));
        assert!(!index_serves_sort(
            &index(doc! { "a": "text" }),
            true,
            &key(&["a"])
        ));
        let sparse = IndexInfo {
            sparse: true,
            ..index(doc! { "a": 1 })
        };
        assert!(!index_serves_sort(&sparse, true, &key(&["a"])));
        let partial = IndexInfo {
            has_partial_filter: true,
            ..index(doc! { "a": 1 })
        };
        assert!(!index_serves_sort(&partial, true, &key(&["a"])));
        let hidden = IndexInfo {
            hidden: true,
            ..index(doc! { "a": 1 })
        };
        assert!(!index_serves_sort(&hidden, true, &key(&["a"])));
    }

    #[test]
    fn a_non_simple_collation_cannot_serve_a_simple_sort() {
        let ci = IndexInfo {
            collation_is_simple: false,
            ..index(doc! { "a": 1 })
        };
        assert!(!index_serves_sort(&ci, true, &key(&["a"])));
        // An index with no collation inherits the collection's default.
        assert!(!index_serves_sort(
            &index(doc! { "a": 1 }),
            false,
            &key(&["a"])
        ));
    }
}
