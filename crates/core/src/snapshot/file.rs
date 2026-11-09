//! The snapshot file: one header line, then one canonical extended-JSON line per document.
//! Canonical form keeps every BSON type exact, and a line is complete only when it ends in a
//! newline, so a torn tail is an error rather than a quietly shorter snapshot.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use bson::{Bson, Document, doc};

use crate::bulk::{BulkRunId, UpdateSpec};
use crate::error::{CoreError, Result};
use crate::store::Namespace;

const FORMAT_VERSION: i32 = 1;

/// What the run was going to do, kept so the report and the undo can name it.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotKind {
    Update {
        filter: Document,
        update: UpdateSpec,
        upsert: bool,
    },
    Delete {
        filter: Document,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotHeader {
    pub run: BulkRunId,
    /// The connection the documents came from: an undo only runs against that one.
    pub connection: uuid::Uuid,
    pub ns: Namespace,
    pub kind: SnapshotKind,
    pub created_unix: i64,
}

impl SnapshotHeader {
    fn to_doc(&self) -> Document {
        let mut d = doc! {
            "format": FORMAT_VERSION,
            "run": self.run.0.to_string(),
            "connection": self.connection.to_string(),
            "db": &self.ns.db,
            "collection": &self.ns.collection,
            "created": self.created_unix,
        };
        match &self.kind {
            SnapshotKind::Delete { filter } => {
                d.insert("kind", "delete");
                d.insert("filter", filter.clone());
            }
            SnapshotKind::Update {
                filter,
                update,
                upsert,
            } => {
                d.insert("kind", "update");
                d.insert("filter", filter.clone());
                d.insert("update", update_to_bson(update));
                d.insert("upsert", *upsert);
            }
        }
        d
    }

    fn from_doc(d: &Document) -> Option<Self> {
        if d.get_i32("format").ok()? != FORMAT_VERSION {
            return None;
        }
        let filter = d.get_document("filter").ok()?.clone();
        let kind = match d.get_str("kind").ok()? {
            "delete" => SnapshotKind::Delete { filter },
            "update" => SnapshotKind::Update {
                filter,
                update: update_from_bson(d.get("update")?)?,
                upsert: d.get_bool("upsert").ok()?,
            },
            _ => return None,
        };
        Some(Self {
            run: BulkRunId(d.get_str("run").ok()?.parse().ok()?),
            connection: d.get_str("connection").ok()?.parse().ok()?,
            ns: Namespace::new(d.get_str("db").ok()?, d.get_str("collection").ok()?),
            kind,
            created_unix: d.get_i64("created").ok()?,
        })
    }
}

fn update_to_bson(spec: &UpdateSpec) -> Bson {
    match spec {
        UpdateSpec::Operators(ops) => Bson::Document(ops.clone()),
        UpdateSpec::Pipeline(stages) => {
            Bson::Array(stages.iter().cloned().map(Bson::Document).collect())
        }
    }
}

fn update_from_bson(value: &Bson) -> Option<UpdateSpec> {
    match value {
        Bson::Document(ops) => Some(UpdateSpec::Operators(ops.clone())),
        Bson::Array(stages) => stages
            .iter()
            .map(|s| s.as_document().cloned())
            .collect::<Option<Vec<_>>>()
            .map(UpdateSpec::Pipeline),
        _ => None,
    }
}

/// One complete line: canonical extended JSON plus a newline.
fn encode_line(doc: &Document) -> Result<Vec<u8>> {
    let mut line = serde_json::to_vec(&Bson::Document(doc.clone()).into_canonical_extjson())
        .map_err(|_| CoreError::SnapshotWriteFailed)?;
    line.push(b'\n');
    Ok(line)
}

fn decode_line(line: &str) -> Result<Document> {
    let value: serde_json::Value = serde_json::from_str(line.trim_end_matches('\n'))
        .map_err(|_| CoreError::SnapshotUnreadable)?;
    match Bson::try_from(value) {
        Ok(Bson::Document(d)) => Ok(d),
        _ => Err(CoreError::SnapshotUnreadable),
    }
}

/// Writes a snapshot, enforcing a byte cap. The file is removed unless [`Self::finish`] ran, so a
/// failed or abandoned run never leaves a partial snapshot that looks complete.
pub struct SnapshotWriter {
    out: Option<BufWriter<File>>,
    path: PathBuf,
    written: u64,
    max_bytes: u64,
}

impl SnapshotWriter {
    /// Creates `<dir>/<run>.jsonl` (owner-only; the directory too) and writes the header.
    pub fn create(dir: &Path, header: SnapshotHeader, max_bytes: u64) -> Result<Self> {
        let name = format!("{}.jsonl", header.run.0);
        Self::create_as(dir, &name, header, max_bytes)
    }

    /// Like [`Self::create`] with an explicit file name (the after-state file of a run).
    pub fn create_as(
        dir: &Path,
        file_name: &str,
        header: SnapshotHeader,
        max_bytes: u64,
    ) -> Result<Self> {
        create_private_dir(dir)?;
        let path = dir.join(file_name);
        let file = open_private(&path)?;
        let mut writer = Self {
            out: Some(BufWriter::new(file)),
            path,
            written: 0,
            max_bytes,
        };
        let line = encode_line(&header.to_doc())?;
        writer.write_line(&line)?;
        Ok(writer)
    }

    /// Appends one document. Past the cap the file is removed and the call fails.
    pub fn push(&mut self, doc: &Document) -> Result<()> {
        let line = encode_line(doc)?;
        self.write_line(&line)
    }

    fn write_line(&mut self, line: &[u8]) -> Result<()> {
        let total = self.written.saturating_add(line.len() as u64);
        if total > self.max_bytes {
            self.discard();
            return Err(CoreError::SnapshotTooLarge);
        }
        let ok = self
            .out
            .as_mut()
            .is_some_and(|out| out.write_all(line).is_ok());
        if !ok {
            self.discard();
            return Err(CoreError::SnapshotWriteFailed);
        }
        self.written = total;
        Ok(())
    }

    /// Flushes to disk and keeps the file; returns its path.
    pub fn finish(mut self) -> Result<PathBuf> {
        let synced = self
            .out
            .take()
            .is_some_and(|out| out.into_inner().is_ok_and(|file| file.sync_all().is_ok()));
        if !synced {
            let _ = std::fs::remove_file(&self.path);
            return Err(CoreError::SnapshotWriteFailed);
        }
        Ok(self.path.clone())
    }

    fn discard(&mut self) {
        if self.out.take().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Drop for SnapshotWriter {
    fn drop(&mut self) {
        self.discard();
    }
}

fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|_| CoreError::SnapshotWriteFailed)
}

fn open_private(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| CoreError::SnapshotWriteFailed)
}

/// Streams a snapshot's documents back, one at a time.
pub struct SnapshotReader {
    lines: BufReader<File>,
    header: SnapshotHeader,
}

impl SnapshotReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|_| CoreError::SnapshotUnreadable)?;
        let mut lines = BufReader::new(file);
        let mut first = String::new();
        let n = lines
            .read_line(&mut first)
            .map_err(|_| CoreError::SnapshotUnreadable)?;
        if n == 0 || !first.ends_with('\n') {
            return Err(CoreError::SnapshotUnreadable);
        }
        let header = decode_line(&first)
            .ok()
            .and_then(|d| SnapshotHeader::from_doc(&d))
            .ok_or(CoreError::SnapshotUnreadable)?;
        Ok(Self { lines, header })
    }

    pub fn header(&self) -> &SnapshotHeader {
        &self.header
    }
}

impl Iterator for SnapshotReader {
    type Item = Result<Document>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        match self.lines.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) if line.ends_with('\n') => Some(decode_line(&line)),
            _ => Some(Err(CoreError::SnapshotUnreadable)),
        }
    }
}
