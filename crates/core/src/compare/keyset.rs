//! Pure helpers behind `MongoStore::scan_sorted` and `key_index_status`, kept free of the driver
//! so they are unit-testable.
//!
//! MongoDB's `$gt` and `$gte` only match values of the SAME type class (numbers with numbers,
//! strings with strings), while its sort order is total across classes. A plain `$gt` keyset
//! would therefore skip everything after a change of type class. Each key field's condition is
//! "at or after `v` within its class, OR of a higher class", where the higher classes are
//! written as "not one of the classes ranked at or below v's".

use bson::{Bson, Document, doc};

use super::key::{KeySpec, KeyTuple};

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

/// The `$type` names of every class ranked at or below `rank` (see `key::type_rank`).
pub fn type_class_names(rank: u8) -> &'static [&'static str] {
    // Built once per rank on first use: a rank-indexed prefix of CLASSES, flattened.
    static TABLE: std::sync::OnceLock<Vec<Vec<&'static str>>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..CLASSES.len())
            .map(|r| {
                CLASSES[..=r]
                    .iter()
                    .flat_map(|c| c.iter().copied())
                    .collect()
            })
            .collect()
    });
    let idx = usize::from(rank).min(table.len() - 1);
    &table[idx]
}

fn rank_of(value: &Bson) -> u8 {
    match value {
        Bson::MinKey => 0,
        Bson::Null | Bson::Undefined => 1,
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => 2,
        Bson::String(_) | Bson::Symbol(_) => 3,
        Bson::Document(_) => 4,
        Bson::Array(_) => 5,
        Bson::Binary(_) => 6,
        Bson::ObjectId(_) => 7,
        Bson::Boolean(_) => 8,
        Bson::DateTime(_) => 9,
        Bson::Timestamp(_) => 10,
        Bson::RegularExpression(_) => 11,
        Bson::DbPointer(_) => 12,
        Bson::JavaScriptCode(_) => 13,
        Bson::JavaScriptCodeWithScope(_) => 14,
        Bson::MaxKey => 15,
    }
}

/// "`field` is after `value`": comparison within the class, or any higher class. `inclusive`
/// makes the in-class comparison `>=`.
fn after(field: &str, value: &Bson, inclusive: bool) -> Document {
    let operator = if inclusive { "$gte" } else { "$gt" };
    let higher = doc! { field: { "$not": { "$type": type_class_names(rank_of(value)).iter().map(|n| Bson::String((*n).into())).collect::<Vec<_>>() } } };
    // `null`, `minKey` and `maxKey` only compare equal to themselves in a query, so the
    // in-class branch is omitted for them (an exclusive start has nothing in its own class).
    let own_class_branch = match value {
        Bson::Null | Bson::Undefined | Bson::MinKey | Bson::MaxKey => {
            inclusive.then(|| doc! { field: value.clone() })
        }
        _ => Some(doc! { field: { operator: value.clone() } }),
    };
    match own_class_branch {
        Some(own) => doc! { "$or": [own, higher] },
        None => higher,
    }
}

/// The filter for "documents matching `user_filter` whose key is at or after `from`".
pub fn keyset_filter(user_filter: &Document, key: &KeySpec, from: Option<&KeyTuple>) -> Document {
    let Some(from) = from else {
        return user_filter.clone();
    };
    let last = key.fields.len() - 1;
    let mut branches: Vec<Bson> = Vec::new();
    for i in 0..key.fields.len() {
        let mut branch = Document::new();
        for (field, value) in key.fields[..i].iter().zip(&from.0) {
            branch.insert(field.clone(), value.clone());
        }
        let tail = after(&key.fields[i], &from.0[i], i == last);
        if branch.is_empty() {
            branch = tail;
        } else {
            for (k, v) in tail {
                branch.insert(k, v);
            }
        }
        branches.push(Bson::Document(branch));
    }
    let keyset = if branches.len() == 1 {
        match branches.remove(0) {
            Bson::Document(d) => d,
            _ => Document::new(),
        }
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
    /// The index sorts by simple (binary) collation: it has an explicit non-simple collation
    /// (`false`) or either an explicit simple one or none at all (`true`; the latter inherits the
    /// collection default, which `index_serves_sort` checks separately).
    pub collation_is_simple: bool,
}

/// Whether `index` can serve `sort(key fields ascending)` under simple collation: a plain
/// (not sparse, partial, hashed or text) index whose leading fields are the key fields in order,
/// all in the same direction (a uniformly reversed index is read backwards), with no
/// non-simple collation.
pub fn index_serves_sort(
    index: &IndexInfo,
    collection_default_is_simple: bool,
    key: &KeySpec,
) -> bool {
    if index.sparse || index.has_partial_filter {
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

    #[test]
    fn a_single_key_is_at_or_after_or_of_a_higher_type_class() {
        let from = KeyTuple(vec![Bson::Int32(5)]);
        let f = keyset_filter(&doc! {}, &key(&["_id"]), Some(&from));
        // _id >= 5 among numbers, OR the value is not of a type class ranked at or below numbers.
        assert_eq!(
            f,
            doc! { "$or": [
                { "_id": { "$gte": 5 } },
                { "_id": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal"] } } },
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
    fn a_compound_key_is_a_lexicographic_or_chain_with_an_inclusive_last_field() {
        let from = KeyTuple(vec![Bson::Int32(1), Bson::String("x".into())]);
        let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
        let branches = f.get_array("$or").unwrap();
        assert_eq!(branches.len(), 2);
        // Branch 0: a is strictly greater (or a higher class).
        assert_eq!(
            branches[0],
            Bson::Document(doc! { "$or": [
                { "a": { "$gt": 1 } },
                { "a": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal"] } } },
            ] })
        );
        // Branch 1: a equals, and b is at or after.
        assert_eq!(
            branches[1],
            Bson::Document(doc! { "a": 1, "$or": [
                { "b": { "$gte": "x" } },
                { "b": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal", "string", "symbol"] } } },
            ] })
        );
    }

    #[test]
    fn the_type_classes_cover_every_rank_in_order() {
        assert_eq!(type_class_names(0), ["minKey"]);
        assert_eq!(type_class_names(1), ["minKey", "null", "undefined"]);
        assert!(type_class_names(15).contains(&"maxKey"));
        for r in 1..=15 {
            assert!(
                type_class_names(r).len() >= type_class_names(r - 1).len(),
                "rank {r}"
            );
        }
    }

    fn index(keys: Document) -> IndexInfo {
        IndexInfo {
            keys,
            sparse: false,
            has_partial_filter: false,
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
