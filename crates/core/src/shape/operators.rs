//! The query operators a shape may keep. A document under a field whose keys are not all in this
//! list is treated as a literal (it collapses to the placeholder), so a `$`-key a person typed as
//! data inside a field's value is not stored. This does not cover every key: an unknown `$`-key
//! at the root of a filter, or in a document inside `$and` / `$or` / `$nor`, is still kept as a
//! key (its value is `"?"`); the server rejects such a query, so it is never profiled.
//!
//! The list is the operators mongod 9.0.8 accepts in a field's operator document, found by
//! probing the server with every `$`-word of its binary plus the geo companions by hand.
//! Not field-level, so not here: `$box`, `$center`, `$centerSphere`, `$polygon`, `$geometry`,
//! `$uniqueDocs` (operands of the geo operators, collapsed with them), `$text`, `$expr`,
//! `$comment`, `$jsonSchema`, `$where`, `$sampleRate` (root only or refused) and the server's own
//! `$_internal*` forms. `$maxDistance` and `$minDistance` are valid only beside `$near` or
//! `$nearSphere`, which is where they appear.

/// Operators valid as the keys of a field's operator document (`{ age: { $gt: 1 } }`), including
/// the regex and geo companions that sit beside them.
pub(crate) const QUERY_OPERATORS: [&str; 28] = [
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
    "$within",
];

/// Words valid at the root of a query. They are not field-level operators, but a `$elemMatch`
/// operand holding one keeps its field names, as it did before the allowlist.
const ROOT_OPERATORS: [&str; 6] = [
    "$comment",
    "$expr",
    "$jsonSchema",
    "$sampleRate",
    "$text",
    "$where",
];

/// Whether `key` is a root-level query word.
pub(crate) fn is_root_operator(key: &str) -> bool {
    ROOT_OPERATORS.contains(&key)
}

/// Whether `key` is a known operator of a field's operator document.
pub(crate) fn is_query_operator(key: &str) -> bool {
    QUERY_OPERATORS.contains(&key)
}
