//! Renders documents as extended-JSON lines and as NDJSON / JSON-array files.

use std::io::{self, Write};

use bson::{Bson, Document};

use super::protocol::{DocFormat, JsonMode};

/// One document as compact extended JSON on a single line. JSON escapes every
/// control character inside strings, so the result never contains a raw
/// newline.
pub fn doc_line(doc: &Document, mode: JsonMode) -> String {
    let bson = Bson::Document(doc.clone());
    let value = match mode {
        JsonMode::Canonical => bson.into_canonical_extjson(),
        JsonMode::Relaxed => bson.into_relaxed_extjson(),
    };
    // Infallible: a `Value` has only string keys and finite numbers.
    let line = serde_json::to_string(&value).expect("serde_json::Value always serializes");
    // JSON allows U+2028/U+2029 raw, but readers like Python's `str.splitlines` split on them.
    line.replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Streams documents into `W` as NDJSON or as one JSON array.
pub struct DocWriter<W: Write> {
    out: W,
    format: DocFormat,
    mode: JsonMode,
    written: u64,
}

impl<W: Write> DocWriter<W> {
    pub fn new(out: W, format: DocFormat, mode: JsonMode) -> Self {
        Self {
            out,
            format,
            mode,
            written: 0,
        }
    }

    /// Writes the file prologue (`[` for a JSON array, nothing for NDJSON).
    pub fn begin(&mut self) -> io::Result<()> {
        match self.format {
            DocFormat::Ndjson => Ok(()),
            DocFormat::JsonArray => self.out.write_all(b"["),
        }
    }

    pub fn doc(&mut self, doc: &Document) -> io::Result<()> {
        let line = doc_line(doc, self.mode);
        match self.format {
            DocFormat::Ndjson => {
                self.out.write_all(line.as_bytes())?;
                self.out.write_all(b"\n")?;
            }
            DocFormat::JsonArray => {
                let separator: &[u8] = if self.written == 0 { b"\n" } else { b",\n" };
                self.out.write_all(separator)?;
                self.out.write_all(line.as_bytes())?;
            }
        }
        self.written += 1;
        Ok(())
    }

    /// Writes the epilogue and returns the underlying writer.
    pub fn finish(mut self) -> io::Result<W> {
        if self.format == DocFormat::JsonArray {
            let tail: &[u8] = if self.written == 0 { b"]\n" } else { b"\n]\n" };
            self.out.write_all(tail)?;
        }
        Ok(self.out)
    }
}
