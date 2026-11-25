//! Wire types for the document export.

/// Output layout of an exported document file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocFormat {
    /// One compact JSON document per line.
    #[default]
    Ndjson,
    /// A single JSON array holding one document per line.
    JsonArray,
}

/// Extended-JSON flavour used for each exported document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JsonMode {
    /// Type-exact canonical extended JSON.
    #[default]
    Canonical,
    /// Readable relaxed extended JSON (numbers lose their BSON width).
    Relaxed,
}
