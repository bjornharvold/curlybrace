//! Keys for the compare feature: which fields identify a document, and the client-side
//! comparator that reproduces MongoDB's sort order for them.
//!
//! The merge-join in `engine` is only correct if this order matches the server's sort under
//! simple (binary) collation exactly. The order is MongoDB's documented BSON comparison order:
//! MinKey < Null < numbers < String/Symbol < Document < Array < Binary < ObjectId < Boolean <
//! DateTime < Timestamp < Regex < (deprecated types) < MaxKey. Numbers compare by exact value
//! across types (see `number`); strings compare by UTF-8 bytes; embedded documents compare field
//! by field as (type class, field name, value).

use std::cmp::Ordering;

use bson::{Bson, Document};
use serde::{Deserialize, Serialize};

use super::number::compare_numbers;
use crate::error::{CoreError, Result};

/// The fields that identify a document: dotted paths, in sort order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySpec {
    pub fields: Vec<String>,
}

impl KeySpec {
    /// The default key: `_id`.
    pub fn id() -> Self {
        Self {
            fields: vec!["_id".into()],
        }
    }

    /// A key over `fields`: non-empty, no empty path segments, no field listed twice.
    pub fn new(fields: Vec<String>) -> Result<Self> {
        if fields.is_empty() {
            return Err(CoreError::CompareSetupInvalid);
        }
        for (i, field) in fields.iter().enumerate() {
            // A leading `$` would become an operator or variable in find, $group and $expr.
            if field.is_empty() || field.split('.').any(|p| p.is_empty() || p.starts_with('$')) {
                return Err(CoreError::CompareSetupInvalid);
            }
            if fields[..i].contains(field) {
                return Err(CoreError::CompareSetupInvalid);
            }
        }
        Ok(Self { fields })
    }

    pub fn contains_id(&self) -> bool {
        self.fields.iter().any(|f| f == "_id")
    }
}

/// One value per key field, in key order.
///
/// The derived `PartialEq` is structural (`Int32(1) != Int64(1)`, `NaN != NaN`); key matching must
/// use `compare_tuples`.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyTuple(pub Vec<Bson>);

/// Rank of a value's canonical type; values of different ranks never compare equal.
pub(crate) fn type_rank(b: &Bson) -> u8 {
    match b {
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

fn text(b: &Bson) -> &str {
    match b {
        Bson::String(s) | Bson::Symbol(s) => s,
        _ => "",
    }
}

fn compare_docs(a: &Document, b: &Document) -> Ordering {
    let mut left = a.iter();
    let mut right = b.iter();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((ka, va)), Some((kb, vb))) => {
                let ord = type_rank(va)
                    .cmp(&type_rank(vb))
                    .then_with(|| ka.as_bytes().cmp(kb.as_bytes()))
                    .then_with(|| compare_bson(va, vb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn compare_arrays(a: &[Bson], b: &[Bson]) -> Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            type_rank(x)
                .cmp(&type_rank(y))
                .then_with(|| compare_bson(x, y))
        })
        .find(|o| *o != Ordering::Equal)
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}

/// Total order over `Bson`, matching MongoDB's sort order under simple collation.
pub fn compare_bson(a: &Bson, b: &Bson) -> Ordering {
    let (ra, rb) = (type_rank(a), type_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Bson::Document(x), Bson::Document(y)) => compare_docs(x, y),
        (Bson::Array(x), Bson::Array(y)) => compare_arrays(x, y),
        (Bson::Binary(x), Bson::Binary(y)) => (x.bytes.len(), u8::from(x.subtype), &x.bytes)
            .cmp(&(y.bytes.len(), u8::from(y.subtype), &y.bytes)),
        (Bson::ObjectId(x), Bson::ObjectId(y)) => x.bytes().cmp(&y.bytes()),
        (Bson::Boolean(x), Bson::Boolean(y)) => x.cmp(y),
        (Bson::DateTime(x), Bson::DateTime(y)) => x.timestamp_millis().cmp(&y.timestamp_millis()),
        (Bson::Timestamp(x), Bson::Timestamp(y)) => {
            (x.time, x.increment).cmp(&(y.time, y.increment))
        }
        (Bson::RegularExpression(x), Bson::RegularExpression(y)) => {
            (&x.pattern, &x.options).cmp(&(&y.pattern, &y.options))
        }
        _ => match ra {
            1 => matches!(a, Bson::Null).cmp(&matches!(b, Bson::Null)),
            2 => compare_numbers(a, b),
            3 => text(a).as_bytes().cmp(text(b).as_bytes()),
            // Undefined sorts strictly below Null (the server's canonical order); MinKey and
            // MaxKey are equal within their class. The deprecated
            // classes never occur as keys (extract_key refuses them); this fallback only
            // keeps the order total for a nested value.
            12..=14 => format!("{a:?}").cmp(&format!("{b:?}")),
            _ => Ordering::Equal,
        },
    }
}

/// Lexicographic comparison of two key tuples.
pub fn compare_tuples(a: &KeyTuple, b: &KeyTuple) -> Ordering {
    a.0.iter()
        .zip(&b.0)
        .map(|(x, y)| compare_bson(x, y))
        .find(|o| *o != Ordering::Equal)
        .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
}

/// A dotted-path lookup that only descends through documents (never through arrays).
fn lookup<'a>(doc: &'a Document, path: &str) -> Option<&'a Bson> {
    let mut parts = path.split('.');
    let mut current = doc.get(parts.next()?)?;
    for part in parts {
        match current {
            Bson::Document(inner) => current = inner.get(part)?,
            _ => return None,
        }
    }
    Some(current)
}

/// The key tuple of `doc`, or `None` when a key field is missing or unreachable. Unlike
/// `extract_key` it does not refuse arrays or deprecated types (the duplicate finder reports
/// whatever collides).
pub fn key_values(doc: &Document, spec: &KeySpec) -> Option<KeyTuple> {
    spec.fields
        .iter()
        .map(|path| lookup(doc, path).cloned())
        .collect::<Option<Vec<_>>>()
        .map(KeyTuple)
}

/// A key value must sort strictly: arrays (the server sorts them by their smallest element),
/// regular expressions (the server rejects them as `$gte` arguments, so a keyset scan could not
/// resume from one) and deprecated types are refused.
fn check_key_value(value: &Bson) -> Result<()> {
    match value {
        Bson::Array(_)
        | Bson::RegularExpression(_)
        | Bson::Undefined
        | Bson::Symbol(_)
        | Bson::DbPointer(_)
        | Bson::JavaScriptCode(_)
        | Bson::JavaScriptCodeWithScope(_) => Err(CoreError::CompareKeyInvalid),
        _ => Ok(()),
    }
}

/// The key tuple of `doc`: `CompareKeyMissing` when a key field is absent (or unreachable
/// because the path crosses an array), `CompareKeyInvalid` for an unusable value.
pub fn extract_key(doc: &Document, spec: &KeySpec) -> Result<KeyTuple> {
    let mut values = Vec::with_capacity(spec.fields.len());
    for path in &spec.fields {
        let value = lookup(doc, path).ok_or(CoreError::CompareKeyMissing)?;
        check_key_value(value)?;
        values.push(value.clone());
    }
    Ok(KeyTuple(values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::oid::ObjectId;
    use bson::spec::BinarySubtype;
    use bson::{Binary, DateTime, Decimal128, Regex, Timestamp, doc};
    use std::cmp::Ordering::{Equal, Greater, Less};
    use std::str::FromStr;

    #[test]
    fn key_components_starting_with_a_dollar_are_refused() {
        for bad in ["$x", "a.$b", "$"] {
            assert_eq!(
                KeySpec::new(vec![bad.into()]),
                Err(CoreError::CompareSetupInvalid),
                "{bad}"
            );
        }
        assert!(KeySpec::new(vec!["a$b".into()]).is_ok());
    }

    #[test]
    fn key_values_returns_none_for_a_missing_field_and_keeps_unusual_values() {
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        assert_eq!(key_values(&doc! { "x": 1 }, &spec), None);
        assert_eq!(
            key_values(&doc! { "k": [1, 2] }, &spec),
            Some(KeyTuple(vec![Bson::Array(vec![
                Bson::Int32(1),
                Bson::Int32(2)
            ])]))
        );
        let nested = KeySpec::new(vec!["a.b".into()]).unwrap();
        assert_eq!(
            key_values(&doc! { "a": { "b": 3 } }, &nested),
            Some(KeyTuple(vec![Bson::Int32(3)]))
        );
        assert_eq!(
            key_values(&doc! { "a": [{ "b": 3 }] }, &nested),
            None,
            "a path never crosses an array"
        );
    }

    fn strictly_increasing(values: &[Bson]) {
        for pair in values.windows(2) {
            assert_eq!(
                compare_bson(&pair[0], &pair[1]),
                Less,
                "{:?} < {:?}",
                pair[0],
                pair[1]
            );
            assert_eq!(compare_bson(&pair[1], &pair[0]), Greater);
        }
    }

    #[test]
    fn types_follow_mongodbs_canonical_order() {
        strictly_increasing(&[
            Bson::MinKey,
            Bson::Null,
            Bson::Int32(-5),
            Bson::Double(-0.5),
            Bson::Int64(0),
            Bson::Decimal128(Decimal128::from_str("0.25").unwrap()),
            Bson::Int32(1),
            Bson::String("a".into()),
            Bson::String("b".into()),
            Bson::Document(doc! { "a": 1 }),
            Bson::Array(vec![]),
            Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![1],
            }),
            Bson::ObjectId(ObjectId::from_bytes([0; 12])),
            Bson::ObjectId(ObjectId::from_bytes([1; 12])),
            Bson::Boolean(false),
            Bson::Boolean(true),
            Bson::DateTime(DateTime::from_millis(0)),
            Bson::DateTime(DateTime::from_millis(1)),
            Bson::Timestamp(Timestamp {
                time: 0,
                increment: 1,
            }),
            Bson::Timestamp(Timestamp {
                time: 1,
                increment: 0,
            }),
            Bson::RegularExpression(Regex {
                pattern: "a".into(),
                options: "".into(),
            }),
            Bson::MaxKey,
        ]);
    }

    #[test]
    fn undefined_sorts_below_null_and_symbol_equals_string() {
        assert_eq!(compare_bson(&Bson::Undefined, &Bson::Null), Less);
        assert_eq!(compare_bson(&Bson::Null, &Bson::Undefined), Greater);
        assert_eq!(compare_bson(&Bson::Null, &Bson::Null), Equal);
        assert_eq!(compare_bson(&Bson::Undefined, &Bson::Undefined), Equal);
        assert_eq!(
            compare_bson(&Bson::Symbol("a".into()), &Bson::String("a".into())),
            Equal
        );
        assert_eq!(compare_bson(&Bson::MinKey, &Bson::MinKey), Equal);
        assert_eq!(compare_bson(&Bson::MaxKey, &Bson::MaxKey), Equal);
    }

    #[test]
    fn strings_compare_by_utf8_bytes() {
        assert_eq!(
            compare_bson(&Bson::String("B".into()), &Bson::String("a".into())),
            Less
        );
        assert_eq!(
            compare_bson(&Bson::String("abc".into()), &Bson::String("abd".into())),
            Less
        );
        assert_eq!(
            compare_bson(&Bson::String("z".into()), &Bson::String("é".into())),
            Less
        );
        assert_eq!(
            compare_bson(&Bson::String("a".into()), &Bson::String("ab".into())),
            Less
        );
        assert_eq!(
            compare_bson(&Bson::String("".into()), &Bson::String("a".into())),
            Less
        );
    }

    #[test]
    fn embedded_documents_compare_type_then_name_then_value() {
        // The name decides before the value does: {a: 2} sorts before {b: 1}.
        let a2 = Bson::Document(doc! { "a": 2 });
        let b1 = Bson::Document(doc! { "b": 1 });
        assert_eq!(compare_bson(&a2, &b1), Less);
        assert_eq!(compare_bson(&Bson::Document(doc! { "a": 1 }), &a2), Less);
        // A field of a lower type class sorts first, whatever the names are.
        let number = Bson::Document(doc! { "z": 1 });
        let string = Bson::Document(doc! { "a": "x" });
        assert_eq!(compare_bson(&number, &string), Less);
        // A document that is a prefix of another is smaller.
        let short = Bson::Document(doc! { "a": 1 });
        let long = Bson::Document(doc! { "a": 1, "b": 1 });
        assert_eq!(compare_bson(&short, &long), Less);
        assert_eq!(compare_bson(&Bson::Document(doc! {}), &short), Less);
    }

    #[test]
    fn arrays_compare_elementwise_and_a_prefix_is_smaller() {
        let one = Bson::Array(vec![Bson::Int32(1)]);
        let one_zero = Bson::Array(vec![Bson::Int32(1), Bson::Int32(0)]);
        let two = Bson::Array(vec![Bson::Int32(2)]);
        assert_eq!(compare_bson(&one, &one_zero), Less);
        assert_eq!(compare_bson(&one_zero, &two), Less);
        assert_eq!(
            compare_bson(
                &two,
                &Bson::Array(vec![Bson::Int32(1), Bson::Int32(9), Bson::Int32(9)])
            ),
            Greater
        );
    }

    #[test]
    fn binary_compares_length_then_subtype_then_bytes() {
        let bin = |subtype, bytes: &[u8]| {
            Bson::Binary(Binary {
                subtype,
                bytes: bytes.to_vec(),
            })
        };
        assert_eq!(
            compare_bson(
                &bin(BinarySubtype::Generic, &[0xff]),
                &bin(BinarySubtype::Generic, &[0, 0])
            ),
            Less
        );
        assert_eq!(
            compare_bson(
                &bin(BinarySubtype::Generic, &[9]),
                &bin(BinarySubtype::Uuid, &[0])
            ),
            Less
        );
        assert_eq!(
            compare_bson(
                &bin(BinarySubtype::Uuid, &[1]),
                &bin(BinarySubtype::Uuid, &[2])
            ),
            Less
        );
    }

    #[test]
    fn tuples_compare_field_by_field() {
        let t = |a: i32, b: &str| KeyTuple(vec![Bson::Int32(a), Bson::String(b.into())]);
        assert_eq!(compare_tuples(&t(1, "b"), &t(1, "c")), Less);
        assert_eq!(compare_tuples(&t(1, "z"), &t(2, "a")), Less);
        assert_eq!(compare_tuples(&t(2, "a"), &t(2, "a")), Equal);
        assert_eq!(
            compare_tuples(
                &KeyTuple(vec![Bson::Int32(1)]),
                &KeyTuple(vec![Bson::Int64(1)])
            ),
            Equal
        );
    }

    #[test]
    fn key_spec_validation() {
        assert_eq!(KeySpec::id().fields, vec!["_id".to_string()]);
        assert!(KeySpec::id().contains_id());
        assert!(KeySpec::new(vec!["email".into(), "tenant.id".into()]).is_ok());
        assert!(!KeySpec::new(vec!["email".into()]).unwrap().contains_id());
        for bad in [
            vec![],
            vec!["".to_string()],
            vec!["a..b".to_string()],
            vec!["a.".to_string()],
            vec!["a".to_string(), "a".to_string()],
        ] {
            assert_eq!(
                KeySpec::new(bad.clone()),
                Err(CoreError::CompareSetupInvalid),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn extract_key_reads_plain_compound_and_dotted_fields() {
        let d = doc! { "_id": 7, "email": "a@x", "tenant": { "id": 3 } };
        assert_eq!(
            extract_key(&d, &KeySpec::id()).unwrap(),
            KeyTuple(vec![Bson::Int32(7)])
        );
        let spec = KeySpec::new(vec!["tenant.id".into(), "email".into()]).unwrap();
        assert_eq!(
            extract_key(&d, &spec).unwrap(),
            KeyTuple(vec![Bson::Int32(3), Bson::String("a@x".into())])
        );
    }

    #[test]
    fn extract_key_refuses_missing_and_unusable_values() {
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        assert_eq!(
            extract_key(&doc! { "x": 1 }, &spec),
            Err(CoreError::CompareKeyMissing)
        );
        // A dotted path cannot reach through an array.
        let through = KeySpec::new(vec!["a.b".into()]).unwrap();
        assert_eq!(
            extract_key(&doc! { "a": [{ "b": 1 }] }, &through),
            Err(CoreError::CompareKeyMissing)
        );
        for bad in [
            Bson::Array(vec![Bson::Int32(1)]),
            Bson::Undefined,
            Bson::RegularExpression(Regex {
                pattern: "a".into(),
                options: String::new(),
            }),
            Bson::Symbol("s".into()),
            Bson::JavaScriptCode("x".into()),
        ] {
            assert_eq!(
                extract_key(&doc! { "k": bad.clone() }, &spec),
                Err(CoreError::CompareKeyInvalid),
                "{bad:?}"
            );
        }
        // A single explicit null is a usable key; two of them are caught later as duplicates.
        assert_eq!(
            extract_key(&doc! { "k": Bson::Null }, &spec).unwrap(),
            KeyTuple(vec![Bson::Null])
        );
    }
}
