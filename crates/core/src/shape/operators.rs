//! The query operators a shape may keep. A document under a field whose keys are not all in this
//! list is treated as a literal (it collapses to the placeholder), so no key a person typed as
//! data, however it begins, ever reaches the history file.

/// Operators valid as the keys of a field's operator document (`{ age: { $gt: 1 } }`), including
/// the regex and geo companions that sit beside them.
pub(crate) const QUERY_OPERATORS: [&str; 32] = [
    "$all",
    "$bitsAllClear",
    "$bitsAllSet",
    "$bitsAnyClear",
    "$bitsAnySet",
    "$elemMatch",
    "$eq",
    "$exists",
    "$geoIntersects",
    "$geoWithin",
    "$gt",
    "$gte",
    "$in",
    "$lt",
    "$lte",
    "$maxDistance",
    "$minDistance",
    "$mod",
    "$ne",
    "$near",
    "$nearSphere",
    "$nin",
    "$not",
    "$options",
    "$regex",
    "$size",
    "$type",
    // The geometry operands are placeholders, but their names may sit beside a legacy `$near`.
    "$box",
    "$center",
    "$centerSphere",
    "$geometry",
    "$polygon",
];

/// Whether `key` is a known operator of a field's operator document.
pub(crate) fn is_query_operator(key: &str) -> bool {
    QUERY_OPERATORS.contains(&key)
}
