//! The one-line note a snippet carries when a value could not be written as it is in the
//! language and was converted (`undefined` or a symbol or a DBPointer, which the driver or the
//! language has no literal for). Fixed text, never data; only when a conversion happened.

use std::cell::Cell;

use crate::codegen::walk::Syntax;

/// The note, without its comment marker.
pub(crate) const NOTE: &str = "Note: undefined/symbol/DBPointer values cannot be written in this language and were converted.";

/// Whether a renderer converted a value; set by a [`Syntax`] while it writes.
#[derive(Debug, Default)]
pub(crate) struct Conversions(Cell<bool>);

impl Conversions {
    pub(crate) fn mark(&self) {
        self.0.set(true);
    }

    pub(crate) fn happened(&self) -> bool {
        self.0.get()
    }
}

/// `body` with the note as its first line in `comment` syntax (`//` or `#`), when `syn` converted
/// a value while writing it; `body` unchanged otherwise.
pub(crate) fn with_note(syn: &dyn Syntax, comment: &str, body: String) -> String {
    match syn.converted() {
        true => format!("{comment} {NOTE}\n{body}"),
        false => body,
    }
}
