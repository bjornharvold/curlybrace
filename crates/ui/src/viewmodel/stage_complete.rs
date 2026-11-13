//! Completion for a stage editor of the Aggregate tab: the stage's JSON, `{ "$match": { ... } }`.
//! Pure and egui-free; names are code. At the top level of the stage (or an empty text) a key
//! offers the stage names ([`STAGE_NAMES`]); deeper keys get the operator list the Shell tab
//! uses. A key is the word after `{` or `,`, bare or just after its opening quote; a word in a
//! value, an array element or any other string gets nothing.
use super::shell_complete::{Completion, Suggestion, SuggestionKind, word_start};
use super::shell_complete_ops::{OPERATORS, STAGE_NAMES};

/// The popup shows no more than this many names.
const MAX_ITEMS: usize = 12;

/// Names for the key being typed at `caret` (a byte offset). `forced` (Ctrl+Space) also lists
/// every name for an empty word.
pub fn complete_stage(text: &str, caret: usize, forced: bool) -> Option<Completion> {
    let caret = (0..=caret.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))?;
    let bytes = text.as_bytes();
    let start = word_start(bytes, caret);
    if (start == caret && !forced) || bytes.get(start).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let names = key_place(bytes, start)?;
    let prefix = text[start..caret].to_lowercase();
    let items: Vec<Suggestion> = names
        .iter()
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .take(MAX_ITEMS)
        .map(|name| Suggestion {
            label: (*name).to_string(),
            insert: (*name).to_string(),
            kind: SuggestionKind::Operator,
        })
        .collect();
    (!items.is_empty()).then_some(Completion {
        replace: start..caret,
        items,
    })
}

/// What a word starting at `start` completes to, when it sits where a key goes: the stage names
/// at the top of the stage, the operators in a body. `None` anywhere else.
fn key_place(bytes: &[u8], start: usize) -> Option<&'static [&'static str]> {
    let quoted = start > 0 && bytes[start - 1] == b'"';
    let end = start - usize::from(quoted);
    let scan = scan_json(&bytes[..end]);
    // A quote that closes a string (or any word inside one) is not a key being typed.
    if scan.in_string {
        return None;
    }
    match (scan.open.last(), scan.last) {
        (None, None) => Some(STAGE_NAMES),
        (Some(b'{'), Some(b'{' | b',')) if scan.open.len() == 1 => Some(STAGE_NAMES),
        (Some(b'{'), Some(b'{' | b',')) => Some(OPERATORS),
        _ => None,
    }
}

/// Where a look over the start of the text ended.
struct JsonScan {
    /// Brackets still open, innermost last.
    open: Vec<u8>,
    /// The last byte outside a string that is not blank; a string counts as `"`.
    last: Option<u8>,
    in_string: bool,
}

fn scan_json(bytes: &[u8]) -> JsonScan {
    let mut scan = JsonScan {
        open: Vec::new(),
        last: None,
        in_string: false,
    };
    let mut escaped = false;
    for &b in bytes {
        if scan.in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => scan.in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => {
                scan.in_string = true;
                scan.last = Some(b'"');
            }
            b'{' | b'[' => {
                scan.open.push(b);
                scan.last = Some(b);
            }
            b'}' | b']' => {
                scan.open.pop();
                scan.last = Some(b);
            }
            b if b.is_ascii_whitespace() => {}
            b => scan.last = Some(b),
        }
    }
    scan
}

#[cfg(test)]
#[path = "stage_complete_tests.rs"]
mod tests;
