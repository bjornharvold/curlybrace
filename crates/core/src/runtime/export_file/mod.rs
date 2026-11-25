//! Writing an export file safely: validating the chosen path, creating the file with owner-only
//! permissions, and removing a file that must not stay.
//!
//! Shared by every job that writes a file the person chose (the compare exports today). Paths,
//! system error text and contents are never logged and never carried in an error.

mod guard;
#[cfg(test)]
mod guard_tests;
mod path;
mod sink;
#[cfg(test)]
mod sink_tests;
#[cfg(test)]
pub(in crate::runtime) mod test_hooks;

pub(in crate::runtime) use guard::FileGuard;
pub(in crate::runtime) use path::validate_path;
pub(in crate::runtime) use sink::{ExportSink, Out, buffered, into_file};
