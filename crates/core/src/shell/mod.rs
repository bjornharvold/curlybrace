//! The Shell tab's backend pieces: building the `mongosh` invocation and hiding the connection
//! password in its output.
mod invocation;
mod scrub;
pub use invocation::{Invocation, ShellError, build_invocation, percent_encode_component};
pub use scrub::{MASK, Scrubber};
