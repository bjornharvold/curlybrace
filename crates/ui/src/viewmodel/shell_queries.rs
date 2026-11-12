//! The collection calls of a mongosh script whose queries the index-miss guard can check: plain
//! `db.<name>.<method>(...)` and `db.getCollection("<name>").<method>(...)` calls of
//! [`METHODS`], with literal filters (and, after `find`/`findOne`, a literal projection and a
//! chained literal `sort`). A matching call whose query is not made of literals is counted in
//! [`Reading::unchecked`], never guessed at. Calls in comments, strings, template text and regex
//! literals are not code (see [`scan_code`](super::shell_scan::scan_code)) and are not read.
//!
//! The database starts as the one the script runs against; a `use <name>` line and
//! `db.getSiblingDB("<name>")` change it for later calls when the name is plain, and make later
//! calls unchecked otherwise, until a plain one follows.
//!
//! One pass over paired tokens: linear in the script, total on any input. Every readable call is
//! returned (the run flow caps how many it checks). Pure and egui-free; no user-facing strings.
use super::shell_queries_call::{Cursor, Outcome, read_db};
use super::shell_queries_lex::{Kind, tokens};
use curlybrace_core::store::Query;
use std::ops::Range;

/// The collection methods whose filter is read.
pub const METHODS: [&str; 7] = [
    "find",
    "findOne",
    "countDocuments",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
];

/// One readable call.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellCall {
    pub db: String,
    pub collection: String,
    /// One of [`METHODS`].
    pub method: &'static str,
    /// The filter, and for `find`/`findOne` the projection, chained sort, skip and limit.
    pub query: Query,
    /// Bytes of the whole call expression, chained calls included, in the text read.
    pub range: Range<usize>,
}

/// The calls of a script, in text order, and how many matching calls were not readable.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    pub calls: Vec<ShellCall>,
    pub unchecked: usize,
}

/// Reads `text`, which runs against the database `start_db`; see the module docs.
pub fn read_calls(text: &str, start_db: &str) -> Reading {
    let tokens = tokens(text);
    let c = Cursor {
        text,
        tokens: &tokens,
    };
    let mut db = Some(start_db.to_string());
    let mut reading = Reading::default();
    let mut t = 0;
    while t < tokens.len() {
        if let Some((next, name)) = use_line(&c, t) {
            db = name;
            t = next;
            continue;
        }
        if is_db(&c, t) {
            let found = read_db(&c, t, &db);
            if let Some(switch) = found.switch {
                db = switch;
            }
            match found.outcome {
                Outcome::Nothing => {}
                Outcome::Unchecked => reading.unchecked += 1,
                Outcome::Call(call) => reading.calls.push(*call),
            }
        }
        t += 1;
    }
    reading
}

/// A `db` that is not a property of something else.
fn is_db(c: &Cursor<'_>, t: usize) -> bool {
    c.ident(t) == Some("db") && !(t > 0 && c.is_punct(t - 1, b'.'))
}

/// A `use <name>` command at token `t`: the token to go on from (the `;` that ends it, or the
/// next line's first) and the database, `None` when the name is not plain. Not a command when
/// `use` does not start its line or is followed by punctuation (`use = 1`, `use: 1`).
fn use_line(c: &Cursor<'_>, t: usize) -> Option<(usize, Option<String>)> {
    let starts_line = c.tokens.get(t).is_some_and(|tok| tok.line_start);
    if !starts_line || c.ident(t) != Some("use") {
        return None;
    }
    let first = c.tokens.get(t + 1).filter(|tok| !tok.line_start)?;
    if !matches!(first.kind, Kind::Ident | Kind::Opaque) {
        return None;
    }
    let end = (t + 1..c.tokens.len())
        .find(|&k| c.tokens[k].line_start || c.is_punct(k, b';'))
        .unwrap_or(c.tokens.len());
    Some((end, plain_name(c, t + 1, end)))
}

/// The tokens `from..to` as one database name: identifiers and `-`, with nothing between them.
fn plain_name(c: &Cursor<'_>, from: usize, to: usize) -> Option<String> {
    let parts = c.tokens.get(from..to)?;
    let plain = parts
        .iter()
        .all(|tok| matches!(tok.kind, Kind::Ident | Kind::Punct(b'-')));
    let joined = parts.windows(2).all(|w| w[0].end == w[1].start);
    let first = parts.first().filter(|tok| tok.kind == Kind::Ident)?;
    let last = parts.last()?;
    let name = c.text.get(first.start..last.end)?;
    (plain && joined && !name.contains('$')).then(|| name.to_string())
}

#[cfg(test)]
#[path = "shell_queries_tests.rs"]
mod tests;
