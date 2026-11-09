//! Text the bulk dialog shows, worked out once when the value it comes from is stored in the
//! flow instead of on every frame: the verdict on the update text, and the one-line previews of
//! the sampled documents. Pure and egui-free.
use bson::{Bson, Document};
use curlybrace_core::bulk::{PreviewDoc, parse_update, validate_update};

/// Documents longer than this are cut in the preview, with an ellipsis.
pub const SHOWN_CHARS: usize = 400;

/// How many nodes (values) of a document are looked at to fill [`SHOWN_CHARS`]: every node
/// renders at least one character, so more than this can never be seen.
const NODE_BUDGET: usize = 1_000;

/// The verdict on an update text, tied to the text it was worked out for.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateCheck {
    text: String,
    problem: Option<&'static str>,
    valid: bool,
}

impl UpdateCheck {
    /// Parses and validates `text` once.
    pub fn of(text: &str) -> Self {
        let verdict = parse_update(text).and_then(|spec| validate_update(&spec));
        let blank = text.trim().is_empty();
        Self {
            text: text.to_owned(),
            problem: match (&verdict, blank) {
                (Err(error), false) => Some(error.i18n_key()),
                _ => None,
            },
            valid: verdict.is_ok(),
        }
    }

    /// Whether this verdict was worked out for exactly `text`.
    pub fn is_for(&self, text: &str) -> bool {
        self.text == text
    }

    /// The i18n key of what is wrong; none for blank text (the field's hint shows what to
    /// write) and for a valid update.
    pub fn problem(&self) -> Option<&'static str> {
        self.problem
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }
}

/// One sampled document as shown: what it is now and what the update makes of it, each already
/// cut to [`SHOWN_CHARS`]. A delete sample has no `after`.
#[derive(Clone, Debug, PartialEq)]
pub struct DocText {
    pub before: String,
    pub after: Option<String>,
}

impl DocText {
    pub fn of_preview(doc: &PreviewDoc) -> Self {
        Self {
            before: compact(&doc.before),
            after: doc.after.as_ref().map(compact),
        }
    }

    pub fn of_sample(doc: &Document) -> Self {
        Self {
            before: compact(doc),
            after: None,
        }
    }
}

/// One line of compact extended JSON, cut when long. Only what can be seen is serialized: the
/// document is first clipped structurally, so a megabyte string or array costs no more than a
/// short one.
pub fn compact(doc: &Document) -> String {
    let mut budget = NODE_BUDGET;
    let text = Bson::Document(clip_document(doc, &mut budget))
        .into_relaxed_extjson()
        .to_string();
    if text.chars().count() <= SHOWN_CHARS {
        return text;
    }
    text.chars()
        .take(SHOWN_CHARS)
        .chain(std::iter::once('\u{2026}'))
        .collect()
}

/// A copy of `doc` that keeps at least everything the first [`SHOWN_CHARS`] characters of its
/// serialization can show, and a bounded amount more.
pub(super) fn clip_document(doc: &Document, budget: &mut usize) -> Document {
    let mut out = Document::new();
    for (key, value) in doc.iter().take(SHOWN_CHARS) {
        if *budget == 0 {
            break;
        }
        out.insert(key.clone(), clip(value, budget));
    }
    out
}

fn clip(value: &Bson, budget: &mut usize) -> Bson {
    *budget = budget.saturating_sub(1);
    match value {
        Bson::String(s) => Bson::String(head(s)),
        Bson::Symbol(s) => Bson::Symbol(head(s)),
        Bson::JavaScriptCode(s) => Bson::JavaScriptCode(head(s)),
        Bson::Document(d) => Bson::Document(clip_document(d, budget)),
        Bson::Array(items) => Bson::Array(clip_array(items, budget)),
        Bson::Binary(b) => Bson::Binary(bson::Binary {
            subtype: b.subtype,
            bytes: b.bytes.iter().copied().take(SHOWN_CHARS).collect(),
        }),
        other => other.clone(),
    }
}

fn clip_array(items: &[Bson], budget: &mut usize) -> Vec<Bson> {
    let mut out = Vec::new();
    for item in items.iter().take(SHOWN_CHARS) {
        if *budget == 0 {
            break;
        }
        out.push(clip(item, budget));
    }
    out
}

/// The first [`SHOWN_CHARS`] characters (more than that is never shown).
fn head(text: &str) -> String {
    text.chars().take(SHOWN_CHARS).collect()
}

#[cfg(test)]
#[path = "bulk_text_tests.rs"]
mod tests;
