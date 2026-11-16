//! The driver side of aggregate options: what [`AppliedOptions`] become as `AggregateOptions`, as
//! the fields of an explained aggregate, and how a server answer is read given what was sent.
//!
//! The two safety rules of the policy are enforced again here: a read never spills to disk and a
//! write always selects the primary, whatever the applied options say. Server text is matched,
//! never stored or echoed.

use bson::{Bson, Document};
use mongodb::error::ErrorKind;
use mongodb::options::{
    AggregateOptions, Collation, CollationAlternate, CollationCaseFirst, CollationStrength, Hint,
    ReadPreference, SelectionCriteria,
};

use crate::driver_error::classify_driver_error;
use crate::error::CoreError;
use crate::pipeline::{
    Alternate, AppliedOptions, CaseFirst, CollationOption, HintOption, ReadPreferenceMode, Strength,
};

/// The server code of `BadValue`, which a refused hint or collation answers with.
const CODE_BAD_VALUE: i32 = 2;

pub(crate) fn driver_collation(c: &CollationOption) -> Collation {
    Collation::builder()
        .locale(c.locale.clone())
        .strength(c.strength.map(driver_strength))
        .case_level(c.case_level)
        .case_first(c.case_first.map(driver_case_first))
        .numeric_ordering(c.numeric_ordering)
        .alternate(c.alternate.map(driver_alternate))
        .build()
}

fn driver_strength(s: Strength) -> CollationStrength {
    match s {
        Strength::Primary => CollationStrength::Primary,
        Strength::Secondary => CollationStrength::Secondary,
        Strength::Tertiary => CollationStrength::Tertiary,
        Strength::Quaternary => CollationStrength::Quaternary,
        Strength::Identical => CollationStrength::Identical,
    }
}

fn driver_case_first(c: CaseFirst) -> CollationCaseFirst {
    match c {
        CaseFirst::Upper => CollationCaseFirst::Upper,
        CaseFirst::Lower => CollationCaseFirst::Lower,
        CaseFirst::Off => CollationCaseFirst::Off,
    }
}

fn driver_alternate(a: Alternate) -> CollationAlternate {
    match a {
        Alternate::NonIgnorable => CollationAlternate::NonIgnorable,
        Alternate::Shifted => CollationAlternate::Shifted,
    }
}

pub(crate) fn driver_hint(h: &HintOption) -> Hint {
    match h {
        HintOption::Name(name) => Hint::Name(name.clone()),
        HintOption::Keys(keys) => Hint::Keys(keys.clone()),
    }
}

fn driver_read_preference(mode: ReadPreferenceMode) -> ReadPreference {
    match mode {
        ReadPreferenceMode::Primary => ReadPreference::Primary,
        ReadPreferenceMode::PrimaryPreferred => ReadPreference::PrimaryPreferred { options: None },
        ReadPreferenceMode::Secondary => ReadPreference::Secondary { options: None },
        ReadPreferenceMode::SecondaryPreferred => {
            ReadPreference::SecondaryPreferred { options: None }
        }
        ReadPreferenceMode::Nearest => ReadPreference::Nearest { options: None },
    }
}

/// Where a read goes; `None` is the database's own preference.
pub(crate) fn read_criteria(o: &AppliedOptions) -> Option<SelectionCriteria> {
    o.read_preference
        .map(|mode| SelectionCriteria::ReadPreference(driver_read_preference(mode)))
}

/// Options of a read (preview, count): no spill to disk whatever `o` says, the server-side
/// limit, and the person's collation, hint, comment and read preference.
pub(crate) fn read_options(o: &AppliedOptions) -> AggregateOptions {
    AggregateOptions::builder()
        .allow_disk_use(false)
        .max_time(o.max_time)
        .collation(o.collation.as_ref().map(driver_collation))
        .hint(o.hint.as_ref().map(driver_hint))
        .comment(o.comment.clone().map(Bson::String))
        .selection_criteria(read_criteria(o))
        .build()
}

/// Options of a write run: always selecting the primary, whatever the connection or `o` say.
pub(crate) fn write_options(o: &AppliedOptions) -> AggregateOptions {
    AggregateOptions::builder()
        .allow_disk_use(o.allow_disk_use)
        .max_time(o.max_time)
        .collation(o.collation.as_ref().map(driver_collation))
        .hint(o.hint.as_ref().map(driver_hint))
        .comment(o.comment.clone().map(Bson::String))
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build()
}

/// The fields of `o` that go inside the `aggregate` of an explain command (the time limit and
/// the verbosity stay outside it). Empty when `o` sets none.
pub(crate) fn explain_fields(o: &AppliedOptions) -> Document {
    let mut fields = Document::new();
    if let Some(collation) = &o.collation {
        if let Ok(document) = bson::to_document(&driver_collation(collation)) {
            fields.insert("collation", document);
        }
    }
    match &o.hint {
        Some(HintOption::Name(name)) => {
            fields.insert("hint", name.clone());
        }
        Some(HintOption::Keys(keys)) => {
            fields.insert("hint", keys.clone());
        }
        None => {}
    }
    if let Some(on) = o.allow_disk_use {
        fields.insert("allowDiskUse", on);
    }
    if let Some(comment) = &o.comment {
        fields.insert("comment", comment.clone());
    }
    fields
}

/// A read preference that is set and is not `primary`.
pub(crate) fn asks_for_non_primary(o: &AppliedOptions) -> bool {
    matches!(o.read_preference, Some(mode) if mode != ReadPreferenceMode::Primary)
}

/// Classifies a driver error knowing what was sent: a missing hint, a refused collation and an
/// unsatisfiable read preference have their own error, only when the matching option was sent.
/// A failed server selection under a non-primary read preference is not a lost connection (the
/// server may be fine, no member just matches), so it is decided before the ordinary
/// classification. The server text is matched here and never kept.
pub(crate) fn classify_with(e: &mongodb::error::Error, o: &AppliedOptions) -> CoreError {
    match e.kind.as_ref() {
        ErrorKind::ServerSelection { .. } if asks_for_non_primary(o) => {
            CoreError::ReadPreferenceUnavailable
        }
        ErrorKind::Command(c) if c.code == CODE_BAD_VALUE => {
            let text = c.message.to_lowercase();
            if o.hint.is_some() && text.contains("hint") {
                CoreError::HintNotFound
            } else if o.collation.is_some()
                && (text.contains("collation") || text.contains("locale"))
            {
                CoreError::CollationRejected
            } else {
                classify_driver_error(e)
            }
        }
        _ => classify_driver_error(e),
    }
}

#[cfg(test)]
#[path = "mongo_pipeline_options_tests.rs"]
mod tests;
