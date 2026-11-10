//! The Shell tab's backend pieces: hiding the connection password in `mongosh` output.
mod scrub;
pub use scrub::{MASK, Scrubber};
