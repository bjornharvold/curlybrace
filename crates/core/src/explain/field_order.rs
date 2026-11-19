//! Is the used index's key order wrong for the query?
//!
//! The rule is the equality, sort, range order of [`super::suggest`], applied to the keys of the
//! one index a plan uses. It is deliberately cautious: it judges only the keys from the start of
//! the index up to the first key the query does not use, only plain ascending or descending
//! keys, and gives no finding when unsure, because a false alarm on a good plan is worse than a
//! missed one.

use bson::{Bson, Document};

use super::suggest::{is_equality, is_range};

/// What is wrong with the order of the used index's keys for this query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldOrderFinding {
    /// `range` comes before the equality field `field` among the used keys.
    RangeBeforeEquality { range: String, field: String },
    /// `range` comes before the sort field `field`.
    RangeBeforeSort { range: String, field: String },
}

/// How the query uses one index key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    /// An exact match; `multi` for an `$in`, whose several values break an index's sort order.
    Equality {
        multi: bool,
    },
    Range,
    /// Used only by the sort.
    SortOnly,
}

fn is_plain_direction(value: &Bson) -> bool {
    match value {
        Bson::Int32(n) => matches!(n, 1 | -1),
        Bson::Int64(n) => matches!(n, 1 | -1),
        Bson::Double(n) => *n == 1.0 || *n == -1.0,
        _ => false,
    }
}

fn filter_use(value: &Bson) -> Option<Use> {
    if is_range(value) {
        Some(Use::Range)
    } else if is_equality(value) {
        let multi = matches!(value, Bson::Document(d) if d.contains_key("$in"));
        Some(Use::Equality { multi })
    } else {
        None
    }
}

/// The keys the query uses, from the start of the index to the first key it does not use (or
/// uses in a way that is neither exact nor a range).
fn used_prefix<'a>(
    keys: &'a Document,
    filter: &Document,
    sort: Option<&Document>,
) -> Vec<(&'a str, Use)> {
    keys.keys()
        .map_while(|name| {
            let usage = match filter.get(name) {
                Some(value) => filter_use(value),
                None => sort.filter(|s| s.contains_key(name)).map(|_| Use::SortOnly),
            };
            usage.map(|usage| (name.as_str(), usage))
        })
        .collect()
}

/// The first problem with the order of `keys` (the index key pattern, in order) for a query with
/// `filter` and `sort`; `None` when every used key is in equality, sort, range order, when a key
/// value is not 1 or -1, or when there is nothing to judge. Only top-level filter fields are
/// looked at: a filter with a `$`-key (`$or`, `$and`, `$expr`...) is not judged.
///
/// Deliberate gaps, all to avoid a false alarm:
/// - With a sort and an `$in` among the used keys the answer is always `None`, so a real
///   range-before-equality problem next to a sorted `$in` goes unreported (not fixed on purpose).
/// - A range field that is also a sort key is never reported.
/// - A regex (a BSON regex, or an `$in` holding one) counts as a range, not an exact match.
/// - A range before a sort key is reported only when every sort key is one of the used keys.
pub(crate) fn check(
    keys: &Document,
    filter: &Document,
    sort: Option<&Document>,
) -> Option<FieldOrderFinding> {
    if !keys.values().all(is_plain_direction) || filter.keys().any(|k| k.starts_with('$')) {
        return None;
    }
    let sort = sort.filter(|s| !s.is_empty());
    let prefix = used_prefix(keys, filter, sort);
    // An `$in` before a sort splits the index order into several runs: whether range-first is
    // wrong then depends on a merge the server may or may not do, so say nothing.
    let multi = prefix
        .iter()
        .any(|(_, usage)| *usage == Use::Equality { multi: true });
    if sort.is_some() && multi {
        return None;
    }
    // A range field that is also sorted on is range-first on purpose.
    let ranges = prefix.iter().enumerate().filter(|(_, (name, usage))| {
        *usage == Use::Range && !sort.is_some_and(|s| s.contains_key(*name))
    });
    let found = |wanted: fn(&Use) -> bool| {
        ranges.clone().find_map(|(i, (range, _))| {
            prefix[i + 1..]
                .iter()
                .find(|(_, usage)| wanted(usage))
                .map(|(field, _)| ((*range).to_owned(), (*field).to_owned()))
        })
    };
    // The key order can only be what forces the in-memory sort when every sort key is one of
    // the used keys.
    let sort_in_used_keys = sort.is_some_and(|s| {
        s.keys()
            .all(|key| prefix.iter().any(|(name, _)| *name == key.as_str()))
    });
    found(|usage| matches!(usage, Use::Equality { .. }))
        .map(|(range, field)| FieldOrderFinding::RangeBeforeEquality { range, field })
        .or_else(|| {
            found(|usage| *usage == Use::SortOnly)
                .filter(|_| sort_in_used_keys)
                .map(|(range, field)| FieldOrderFinding::RangeBeforeSort { range, field })
        })
}

/// The plan stages that read one index.
const INDEX_SCANS: [&str; 3] = ["IXSCAN", "EXPRESS_IXSCAN", "EXPRESS_CLUSTERED_IXSCAN"];

fn is_stage_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// One `name: 1` / `name: -1` entry of a summary's key list.
fn summary_key(entry: &str) -> Option<(&str, i32)> {
    let (name, direction) = entry.split_once(':')?;
    let name = name.trim();
    let bad_name = name.is_empty()
        || name.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '{' | '}' | ','));
    let direction = match direction.trim() {
        "1" => 1,
        "-1" => -1,
        _ => return None,
    };
    (!bad_name).then_some((name, direction))
}

/// The key pattern of a profiler plan summary that names exactly one index scan
/// (`IXSCAN { a: 1, b: -1 }`). `None` for no scan or several, for anything after the key list
/// but another stage, for a key that is not 1 or -1 (hashed, text...), and for a name that is
/// empty or contains whitespace, `,` `:` `{` `}` or a quote.
pub(crate) fn summary_keys(plan_summary: &str) -> Option<Document> {
    let scans = plan_summary
        .split(|c: char| !is_stage_char(c))
        .filter(|token| INDEX_SCANS.contains(token))
        .count();
    if scans != 1 {
        return None;
    }
    let start = plan_summary.find("IXSCAN {")? + "IXSCAN {".len();
    let after = &plan_summary[start..];
    let (inner, rest) = after.split_once('}')?;
    if inner.contains('{') || !(rest.is_empty() || rest.starts_with(',')) {
        return None;
    }
    inner
        .split(',')
        .map(summary_key)
        .try_fold(Document::new(), |keys, entry| {
            let (name, direction) = entry?;
            (!keys.contains_key(name)).then(|| {
                let mut next = keys;
                next.insert(name, direction);
                next
            })
        })
}
