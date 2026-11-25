//! Wire types for the document export.

use std::fmt;
use std::path::PathBuf;

use bson::Document;

use crate::compare::protocol::RunId;
use crate::error::CoreError;
use crate::runtime::SessionId;
use crate::store::Namespace;

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

/// What to export: the documents of `ns` matching `filter`, written to `path`.
#[derive(Clone, PartialEq)]
pub struct DocExportRequest {
    pub session: SessionId,
    pub ns: Namespace,
    /// Which documents to export; empty means the whole collection.
    pub filter: Document,
    pub format: DocFormat,
    pub mode: JsonMode,
    pub path: PathBuf,
}

/// Prints the layout only: the path, the filter and the namespace are never shown.
impl fmt::Debug for DocExportRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocExportRequest")
            .field("format", &self.format)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportCommand {
    /// Starts an export run.
    Start {
        run: RunId,
        request: DocExportRequest,
    },
    /// Ends the run `run`; unknown runs are ignored.
    Cancel { run: RunId },
}

/// How a finished export ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocExportOutcome {
    /// Documents written to the file.
    pub written: u64,
    /// Bytes written to the file.
    pub bytes: u64,
    /// Whether the run was cancelled before the end.
    pub cancelled: bool,
    /// Whether a partial file was left on disk.
    pub file_left: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExportEvent {
    Progress {
        run: RunId,
        done: u64,
        total: Option<u64>,
    },
    Done {
        run: RunId,
        outcome: DocExportOutcome,
    },
    Failed {
        run: RunId,
        error: CoreError,
    },
}
