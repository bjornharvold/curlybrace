//! The in-memory store sorts and matches with the same comparator the compare feature uses, so
//! the fake and the real merge-join agree on key order.
pub use crate::compare::key::compare_bson as cmp_bson;
