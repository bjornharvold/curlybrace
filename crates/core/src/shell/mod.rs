//! The Shell tab's backend pieces: building the `mongosh` invocation, running it with streamed
//! output, and hiding the connection password in that output.
mod classify;
mod control;
mod decode;
#[cfg(test)]
pub(crate) mod fake;
mod invocation;
mod pipe;
mod run;
mod scrub;
pub use invocation::{Invocation, ShellError, build_invocation, percent_encode_component};
pub use run::{Outcome, RunLimits, ShellHandle, Stream, spawn_shell};
pub use scrub::{MASK, Scrubber, secret_forms};
