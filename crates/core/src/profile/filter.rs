use super::ProfileEntry;
use crate::shape::from_profile_command;

/// Whether the profiled operation had a non-empty filter (for update/remove it is the first
/// statement's `q`). `false` for an op that has no filter or no supported shape.
pub fn entry_has_filter(entry: &ProfileEntry) -> bool {
    from_profile_command(&entry.op, &entry.command).is_some_and(|shape| shape.has_filter())
}
