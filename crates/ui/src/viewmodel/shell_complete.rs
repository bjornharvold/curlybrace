//! Completion for the Shell tab's editor: given the text and the caret, the names that fit the
//! word being typed. Pure and egui-free; names are code, so there are no user-facing strings.
//!
//! Where the caret is comes from a short backward look from the caret (identifier characters,
//! `.`, `$`), after [`scan_code`] confirmed the word is code: never in a string, template text,
//! comment or regular expression. The word is a prefix; nothing is offered for an empty word
//! unless the caret follows a `.` (or is `$`, which is part of the word).
use super::shell_complete_data::{COLLECTION_METHODS, DB_METHODS, SHOW_ARGS, STATEMENT_WORDS};
use super::shell_complete_ops::OPERATORS;
use super::shell_scan::{CodeScan, scan_code};
use std::ops::Range;

/// The popup shows no more than this many names.
const MAX_ITEMS: usize = 12;

/// Where the caret is, which decides what is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A word at the start of a statement.
    StatementWord,
    /// The word after `show`.
    ShowArg,
    /// After `db.`.
    DbMember,
    /// After `db.<name>.`.
    CollectionMember,
    /// A `$word` inside an object or array.
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Collection,
    DbMethod,
    CollectionMethod,
    Operator,
    Word,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What the list shows.
    pub label: String,
    /// What replaces [`Completion::replace`] when picked.
    pub insert: String,
    pub kind: SuggestionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Bytes to replace: the typed prefix, or `db.` plus the prefix when a name needs
    /// `db.getCollection("...")` (then every `insert` starts with `db.`).
    pub replace: Range<usize>,
    pub items: Vec<Suggestion>,
}

/// What the look back from the caret found.
struct Context {
    place: Place,
    /// The typed prefix.
    word: Range<usize>,
    /// Start of the `db` of a `db.` member access.
    db_start: usize,
}

/// Suggestions for the word ending at `caret` (a byte offset; one inside a char is moved back to
/// its start). `collections` is every loaded collection name, sorted and without duplicates.
pub fn complete(text: &str, caret: usize, collections: &[String]) -> Option<Completion> {
    let caret = floor_boundary(text, caret)?;
    let ctx = context(text, caret)?;
    let prefix = text[ctx.word.clone()].to_lowercase();
    let mut items = items_for(ctx.place, &prefix, collections);
    items.truncate(MAX_ITEMS);
    if items.is_empty() {
        return None;
    }
    Some(finish(ctx, items))
}

/// Where the caret is, when completion applies there at all (ignoring whether a name matches).
pub fn place_at(text: &str, caret: usize) -> Option<Place> {
    let caret = floor_boundary(text, caret)?;
    context(text, caret).map(|c| c.place)
}

fn floor_boundary(text: &str, caret: usize) -> Option<usize> {
    if caret > text.len() {
        return None;
    }
    (0..=caret).rev().find(|&i| text.is_char_boundary(i))
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Start of the identifier that ends at `end`.
fn word_start(bytes: &[u8], end: usize) -> usize {
    (0..end)
        .rev()
        .take_while(|&i| is_word_byte(bytes[i]))
        .last()
        .unwrap_or(end)
}

/// `end` moved back over blanks and line breaks.
fn skip_ws_back(bytes: &[u8], end: usize) -> usize {
    (0..end)
        .rev()
        .take_while(|&i| bytes[i].is_ascii_whitespace())
        .last()
        .unwrap_or(end)
}

fn context(text: &str, caret: usize) -> Option<Context> {
    let bytes = text.as_bytes();
    let scan = scan_code(text);
    let start = word_start(bytes, caret);
    let word = start..caret;
    if word.is_empty() {
        // Only a `.` leads to a member list.
        return member(bytes, &scan, start, word);
    }
    if bytes[start].is_ascii_digit() || !scan.in_code[word.clone()].iter().all(|&c| c) {
        return None;
    }
    if bytes[start] == b'$' {
        return operator(bytes, &scan, start, word);
    }
    if start > 0 && bytes[start - 1] == b'.' {
        return member(bytes, &scan, start, word);
    }
    statement(bytes, &scan, start, word)
}

/// `$word` counts inside an object or array, not after a `.` or at the top of a script.
fn operator(bytes: &[u8], scan: &CodeScan, start: usize, word: Range<usize>) -> Option<Context> {
    if start > 0 && bytes[start - 1] == b'.' {
        return None;
    }
    let inside = innermost_open(bytes, scan, start)?;
    matches!(inside, b'{' | b'[').then_some(Context {
        place: Place::Operator,
        word,
        db_start: start,
    })
}

/// The bracket still open at `end`, as its opening byte.
fn innermost_open(bytes: &[u8], scan: &CodeScan, end: usize) -> Option<u8> {
    let mut open = Vec::new();
    for i in (0..end).filter(|&i| scan.in_code[i]) {
        match bytes[i] {
            b @ (b'{' | b'[' | b'(') => open.push(b),
            b'}' | b']' | b')' => {
                open.pop();
            }
            _ => {}
        }
    }
    open.last().copied()
}

/// The member after `<receiver>.`, where the word starts at `start` (just after the dot):
/// `db.` or `db.<name>.`.
fn member(bytes: &[u8], scan: &CodeScan, start: usize, word: Range<usize>) -> Option<Context> {
    if start == 0 || bytes[start - 1] != b'.' || !scan.in_code[start - 1] {
        return None;
    }
    let name_end = skip_ws_back(bytes, start - 1);
    let name_start = word_start(bytes, name_end);
    if name_start == name_end || !scan.in_code[name_end - 1] {
        return None;
    }
    let before = skip_ws_back(bytes, name_start);
    if before > 0 && bytes[before - 1] == b'.' {
        let db_end = skip_ws_back(bytes, before - 1);
        let db_start = word_start(bytes, db_end);
        return (is_db(bytes, scan, db_start, db_end)).then_some(Context {
            place: Place::CollectionMember,
            word,
            db_start,
        });
    }
    is_db(bytes, scan, name_start, name_end).then_some(Context {
        place: Place::DbMember,
        word,
        db_start: name_start,
    })
}

/// `bytes[start..end]` is a free-standing `db` in code (not `x.db`).
fn is_db(bytes: &[u8], scan: &CodeScan, start: usize, end: usize) -> bool {
    if &bytes[start..end] != b"db" || !scan.in_code[start] {
        return false;
    }
    let before = skip_ws_back(bytes, start);
    !(before > 0 && bytes[before - 1] == b'.')
}

/// A plain word: after `show`, or at the start of a statement.
fn statement(bytes: &[u8], scan: &CodeScan, start: usize, word: Range<usize>) -> Option<Context> {
    let place = if after_show(bytes, scan, start) {
        Place::ShowArg
    } else if statement_start(bytes, scan, start) {
        Place::StatementWord
    } else {
        return None;
    };
    Some(Context {
        place,
        word,
        db_start: start,
    })
}

/// The word before `start` on the same line is a `show` that starts a statement.
fn after_show(bytes: &[u8], scan: &CodeScan, start: usize) -> bool {
    let end = skip_ws_back(bytes, start);
    if end == start || bytes[end..start].contains(&b'\n') {
        return false;
    }
    let show = word_start(bytes, end);
    &bytes[show..end] == b"show" && scan.in_code[show] && statement_start(bytes, scan, show)
}

/// Whether a statement can begin at `idx`: at the start of the text, after `;` or `}`, or on a
/// new line after something that does not leave an expression open.
fn statement_start(bytes: &[u8], scan: &CodeScan, idx: usize) -> bool {
    let end = skip_ws_back(bytes, idx);
    if end == 0 {
        return true;
    }
    let last = bytes[end - 1];
    let code = scan.in_code[end - 1];
    if code && matches!(last, b';' | b'}') {
        return true;
    }
    let new_line = bytes[end..idx].contains(&b'\n');
    new_line && (!code || !b",([:=+-*/%&|?<>!.^~".contains(&last))
}

fn items_for(place: Place, prefix: &str, collections: &[String]) -> Vec<Suggestion> {
    match place {
        Place::StatementWord => names(STATEMENT_WORDS, prefix, SuggestionKind::Word),
        Place::ShowArg => names(SHOW_ARGS, prefix, SuggestionKind::Word),
        Place::Operator => names(OPERATORS, prefix, SuggestionKind::Operator),
        Place::CollectionMember => {
            names(COLLECTION_METHODS, prefix, SuggestionKind::CollectionMethod)
        }
        Place::DbMember => {
            let mut items = collection_items(collections, prefix);
            items.extend(names(DB_METHODS, prefix, SuggestionKind::DbMethod));
            items
        }
    }
}

fn names(list: &[&str], prefix: &str, kind: SuggestionKind) -> Vec<Suggestion> {
    list.iter()
        .filter(|name| name.to_lowercase().starts_with(prefix))
        .map(|name| Suggestion {
            label: (*name).to_string(),
            insert: (*name).to_string(),
            kind,
        })
        .collect()
}

fn collection_items(collections: &[String], prefix: &str) -> Vec<Suggestion> {
    collections
        .iter()
        .filter(|name| name.to_lowercase().starts_with(prefix))
        .map(|name| Suggestion {
            label: name.clone(),
            insert: if needs_get_collection(name) {
                get_collection_call(name)
            } else {
                name.clone()
            },
            kind: SuggestionKind::Collection,
        })
        .collect()
}

/// `db.<name>` does not reach it: not a plain identifier, or `db` has a method of that name.
fn needs_get_collection(name: &str) -> bool {
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    !plain || DB_METHODS.contains(&name)
}

fn get_collection_call(name: &str) -> String {
    let mut out = String::from("db.getCollection(\"");
    for c in name.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str("\")");
    out
}

/// Widens the replaced range over `db.` when a name rewrites it, and keeps the rest of the
/// names working under that wider range.
fn finish(ctx: Context, items: Vec<Suggestion>) -> Completion {
    let rewrites = items
        .iter()
        .any(|s| s.kind == SuggestionKind::Collection && s.insert.starts_with("db."));
    if !(rewrites && ctx.place == Place::DbMember) {
        return Completion {
            replace: ctx.word,
            items,
        };
    }
    let items = items
        .into_iter()
        .map(|s| {
            if s.insert.starts_with("db.") && s.kind == SuggestionKind::Collection {
                s
            } else {
                Suggestion {
                    insert: format!("db.{}", s.insert),
                    ..s
                }
            }
        })
        .collect();
    Completion {
        replace: ctx.db_start..ctx.word.end,
        items,
    }
}

#[cfg(test)]
#[path = "shell_complete_tests.rs"]
mod tests;
