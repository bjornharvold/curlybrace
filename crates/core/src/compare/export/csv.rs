//! The differences report as CSV (RFC 4180, CRLF line ends).
//!
//! The file starts with `#` comment lines (the summary: counts per pair and in total, no
//! durations; each line is ONE csv_cell-quoted field), then the header row `pair,source,target,category,key,changed_paths`, then one row
//! per difference. `key` is the key tuple as a relaxed extended JSON array; `changed_paths` is
//! empty, or a JSON array of display paths (`a.b`, `c[2]`). It never carries values. Every cell
//! goes through [`csv_cell`], so a spreadsheet cannot read one as a formula.

use std::io::Write;

use bson::Bson;

use super::RenderError;
use super::sanitize::{comment_text, csv_cell};
use crate::compare::diff::{PathSegment, render_path};
use crate::compare::engine::Summary;
use crate::compare::key::KeyTuple;
use crate::compare::results::Category;

const EOL: &str = "\r\n";
const HEADER_ROW: &str = "pair,source,target,category,key,changed_paths";

/// One compared pair in the summary. `source` and `target` are display names (`db.collection`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairSummary {
    pub source: String,
    pub target: String,
    pub summary: Summary,
}

/// What the summary comments say. `label` is free text from the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportHeader {
    pub label: String,
    pub pairs: Vec<PairSummary>,
}

/// One difference. `pair` is the 1-based position of the pair in [`ReportHeader::pairs`].
#[derive(Clone, Debug)]
pub struct CsvRow<'a> {
    pub pair: usize,
    pub source: &'a str,
    pub target: &'a str,
    pub category: Category,
    pub key: &'a KeyTuple,
    pub changed_paths: &'a [Vec<PathSegment>],
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::OnlyInSource => "only_in_source",
        Category::OnlyInTarget => "only_in_target",
        Category::Different => "different",
    }
}

fn counts_line(s: &Summary) -> String {
    format!(
        "only_in_source={}, only_in_target={}, different={}, identical={}, scanned_source={}, scanned_target={}",
        s.only_in_source,
        s.only_in_target,
        s.different,
        s.equal,
        s.scanned_source,
        s.scanned_target
    )
}

fn total(pairs: &[PairSummary]) -> Summary {
    pairs.iter().fold(Summary::default(), |acc, p| Summary {
        equal: acc.equal.saturating_add(p.summary.equal),
        only_in_source: acc.only_in_source.saturating_add(p.summary.only_in_source),
        only_in_target: acc.only_in_target.saturating_add(p.summary.only_in_target),
        different: acc.different.saturating_add(p.summary.different),
        scanned_source: acc.scanned_source.saturating_add(p.summary.scanned_source),
        scanned_target: acc.scanned_target.saturating_add(p.summary.scanned_target),
    })
}

fn summary_lines(header: &ReportHeader) -> Vec<String> {
    let mut lines = vec![
        "# curlybrace data compare report".to_string(),
        format!("# label: {}", comment_text(&header.label))
            .trim_end()
            .to_string(),
        format!("# pairs: {}", header.pairs.len()),
    ];
    for (i, pair) in header.pairs.iter().enumerate() {
        let names = comment_text(&format!("{} -> {}", pair.source, pair.target));
        lines.push(format!(
            "# pair {}: {names}: {}",
            i + 1,
            counts_line(&pair.summary)
        ));
    }
    lines.push(format!("# total: {}", counts_line(&total(&header.pairs))));
    lines
}

/// The key tuple as a relaxed extended JSON array.
fn key_cell(key: &KeyTuple) -> Result<String, RenderError> {
    serde_json::to_string(&Bson::Array(key.0.clone()).into_relaxed_extjson())
        .map_err(|_| RenderError::Unrenderable)
}

/// Empty for no paths, else a JSON array of display paths.
fn paths_cell(paths: &[Vec<PathSegment>]) -> Result<String, RenderError> {
    if paths.is_empty() {
        return Ok(String::new());
    }
    let shown: Vec<String> = paths.iter().map(|p| render_path(p)).collect();
    serde_json::to_string(&shown).map_err(|_| RenderError::Unrenderable)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    New,
    Rows,
}

/// Writes the report: [`begin`](Self::begin), any number of [`row`](Self::row)s,
/// [`finish`](Self::finish).
pub struct CsvWriter<W: Write> {
    inner: W,
    state: State,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            state: State::New,
        }
    }

    /// The summary comments and the header row. Must be the first call.
    pub fn begin(&mut self, header: &ReportHeader) -> Result<(), RenderError> {
        if self.state != State::New {
            return Err(RenderError::Sequence);
        }
        let mut text = String::new();
        for line in summary_lines(header) {
            // One quoted field per line: commas, quotes and semicolons in a label or a name
            // must not split the line into cells (the `#` start needs no formula prefix).
            text.push_str(&csv_cell(&line));
            text.push_str(EOL);
        }
        text.push_str(HEADER_ROW);
        text.push_str(EOL);
        self.inner.write_all(text.as_bytes())?;
        self.state = State::Rows;
        Ok(())
    }

    /// One difference.
    pub fn row(&mut self, row: &CsvRow<'_>) -> Result<(), RenderError> {
        if self.state != State::Rows {
            return Err(RenderError::Sequence);
        }
        let cells = [
            row.pair.to_string(),
            row.source.to_string(),
            row.target.to_string(),
            category_name(row.category).to_string(),
            key_cell(row.key)?,
            paths_cell(row.changed_paths)?,
        ];
        let line = cells
            .iter()
            .map(|c| csv_cell(c))
            .collect::<Vec<_>>()
            .join(",");
        self.inner.write_all(line.as_bytes())?;
        self.inner.write_all(EOL.as_bytes())?;
        Ok(())
    }

    /// Flushes and returns the sink.
    pub fn finish(mut self) -> Result<W, RenderError> {
        if self.state != State::Rows {
            return Err(RenderError::Sequence);
        }
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
#[path = "csv_tests.rs"]
mod tests;
