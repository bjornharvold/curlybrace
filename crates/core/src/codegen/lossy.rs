//! The one-line note a snippet carries when a value could not be written as it is in the
//! language and was converted (`undefined`, a symbol or a DBPointer, which the driver or the
//! language has no literal for, or a UUID binary the driver would reject). Fixed text, never
//! data; only when a conversion happened, and it names only the kinds converted in that snippet.

use std::cell::Cell;

use crate::codegen::walk::Syntax;

/// A kind of value a renderer had to convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Converted {
    Undefined,
    Symbol,
    DbPointer,
    /// A subtype 4 binary that is not 16 bytes, written as generic binary.
    ShortUuid,
}

impl Converted {
    const ALL: [Converted; 4] = [
        Converted::Undefined,
        Converted::Symbol,
        Converted::DbPointer,
        Converted::ShortUuid,
    ];

    fn label(self) -> &'static str {
        match self {
            Converted::Undefined => "undefined",
            Converted::Symbol => "symbol",
            Converted::DbPointer => "DBPointer",
            Converted::ShortUuid => "non-16-byte UUID",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// The note for `kinds`, without its comment marker; `None` when nothing was converted.
pub(crate) fn note(kinds: &[Converted]) -> Option<String> {
    let labels: Vec<&str> = Converted::ALL
        .iter()
        .filter(|kind| kinds.contains(kind))
        .map(|kind| kind.label())
        .collect();
    match labels.is_empty() {
        true => None,
        false => Some(format!(
            "Note: {} values cannot be written in this language and were converted.",
            labels.join("/")
        )),
    }
}

/// Which kinds a renderer converted; set by a [`Syntax`] while it writes.
#[derive(Debug, Default)]
pub(crate) struct Conversions(Cell<u8>);

impl Conversions {
    pub(crate) fn mark(&self, kind: Converted) {
        self.0.set(self.0.get() | kind.bit());
    }

    pub(crate) fn kinds(&self) -> Vec<Converted> {
        Converted::ALL
            .into_iter()
            .filter(|kind| self.0.get() & kind.bit() != 0)
            .collect()
    }
}

/// `body` with the note as its first line in `comment` syntax (`//` or `#`), when `syn` converted
/// a value while writing it; `body` unchanged otherwise.
pub(crate) fn with_note(syn: &dyn Syntax, comment: &str, body: String) -> String {
    match note(&syn.converted()) {
        Some(text) => format!("{comment} {text}\n{body}"),
        None => body,
    }
}
