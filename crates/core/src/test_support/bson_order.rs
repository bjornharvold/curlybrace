//! Total ordering over `Bson`, used by `InMemoryStore` for sorting and equality.
//!
//! Follows MongoDB's cross-type comparison order:
//! MinKey < Null < Numbers < String < Document < Array < Binary < ObjectId
//! < Bool < DateTime < Timestamp < Regex < MaxKey.
//! All numeric types compare by value (so `Int32(1) == Int64(1) == Double(1.0)`);
//! NaN sorts before every other number and equals itself. Other BSON types
//! (JavaScript, Symbol, DbPointer, ...) rank after Regex and before MaxKey and
//! compare by their debug representation, which is stable but not Mongo-exact.

use std::cmp::Ordering;

use bson::{Bson, Document};

const NUMBER_RANK: u8 = 2;
const OTHER_RANK: u8 = 12;

fn rank(b: &Bson) -> u8 {
    match b {
        Bson::MinKey => 0,
        Bson::Null | Bson::Undefined => 1,
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => NUMBER_RANK,
        Bson::String(_) => 3,
        Bson::Document(_) => 4,
        Bson::Array(_) => 5,
        Bson::Binary(_) => 6,
        Bson::ObjectId(_) => 7,
        Bson::Boolean(_) => 8,
        Bson::DateTime(_) => 9,
        Bson::Timestamp(_) => 10,
        Bson::RegularExpression(_) => 11,
        Bson::MaxKey => 13,
        _ => OTHER_RANK,
    }
}

fn as_f64(b: &Bson) -> f64 {
    match b {
        Bson::Int32(n) => f64::from(*n),
        Bson::Int64(n) => *n as f64,
        Bson::Double(n) => *n,
        Bson::Decimal128(d) => d.to_string().parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

fn cmp_docs(a: &Document, b: &Document) -> Ordering {
    let mut left = a.iter();
    let mut right = b.iter();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((ka, va)), Some((kb, vb))) => {
                let ord = cmp_bson(va, vb).then_with(|| ka.cmp(kb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn cmp_arrays(a: &[Bson], b: &[Bson]) -> Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| cmp_bson(x, y))
        .find(|o| *o != Ordering::Equal)
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}

/// Total order over `Bson` values; see the module docs for the rules.
pub fn cmp_bson(a: &Bson, b: &Bson) -> Ordering {
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Bson::String(x), Bson::String(y)) => x.cmp(y),
        (Bson::Document(x), Bson::Document(y)) => cmp_docs(x, y),
        (Bson::Array(x), Bson::Array(y)) => cmp_arrays(x, y),
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
        _ if ra == NUMBER_RANK => cmp_f64(as_f64(a), as_f64(b)),
        _ if ra == OTHER_RANK => format!("{a:?}").cmp(&format!("{b:?}")),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn numbers_compare_by_value_across_types() {
        assert_eq!(cmp_bson(&Bson::Int32(1), &Bson::Int64(1)), Ordering::Equal);
        assert_eq!(
            cmp_bson(&Bson::Double(1.5), &Bson::Int32(2)),
            Ordering::Less
        );
        assert_eq!(
            cmp_bson(&Bson::Double(f64::NAN), &Bson::Int32(-9)),
            Ordering::Less
        );
    }

    #[test]
    fn types_follow_mongo_order() {
        let ordered = [
            Bson::MinKey,
            Bson::Null,
            Bson::Int32(0),
            Bson::String("a".into()),
            Bson::Document(doc! {}),
            Bson::Array(vec![]),
            Bson::ObjectId(bson::oid::ObjectId::new()),
            Bson::Boolean(false),
            Bson::MaxKey,
        ];
        for pair in ordered.windows(2) {
            assert_eq!(cmp_bson(&pair[0], &pair[1]), Ordering::Less, "{pair:?}");
        }
    }

    #[test]
    fn documents_and_arrays_compare_elementwise() {
        let a = Bson::Document(doc! { "x": 1 });
        let b = Bson::Document(doc! { "x": 2 });
        assert_eq!(cmp_bson(&a, &b), Ordering::Less);
        let short = Bson::Array(vec![Bson::Int32(1)]);
        let long = Bson::Array(vec![Bson::Int32(1), Bson::Int32(0)]);
        assert_eq!(cmp_bson(&short, &long), Ordering::Less);
    }
}
