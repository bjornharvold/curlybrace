//! The driver side of aggregate options: what [`AppliedOptions`] become as `AggregateOptions`, as
//! the fields of an explained aggregate, and how a server answer is read given what was sent.
//!
//! The safety rules of the policy are enforced again here, whatever the applied options say: a
//! preview or a count (`read_options`) never spills to disk, a run (`write_options`) always
//! selects the primary, and every time limit is within 1 ms and the cap (`driver_max_time`).
//! An explain and a run send `allowDiskUse` as set. Server text is matched, never stored or
//! echoed.

use bson::{Bson, Document};
use mongodb::error::ErrorKind;
use mongodb::options::{
    AggregateOptions, Collation, CollationAlternate, CollationCaseFirst, CollationStrength, Hint,
    ReadPreference, SelectionCriteria,
};

use crate::driver_error::classify_driver_error;
use crate::error::CoreError;
use crate::pipeline::{
    Alternate, AppliedOptions, CaseFirst, CollationOption, HintOption, ReadPreferenceMode,
    Strength, clamped_time,
};

/// The server code of `BadValue`, which a refused hint or collation answers with.
const CODE_BAD_VALUE: i32 = 2;
/// The server's phrase (lowercased) for a hint that names no index, as MongoDB 9.0 words it:
/// `... planner returned error :: caused by :: hint provided does not correspond to an existing
/// index`. Matched whole so an unrelated `BadValue` that merely says "hint" is not taken for it.
const HINT_NOT_FOUND_TEXT: &str = "hint provided does not correspond to an existing index";
/// The server's phrase (lowercased) for an unknown collation locale, as MongoDB 9.0 words it:
/// `Field 'locale' is invalid in: { locale: "xx" }`.
const LOCALE_INVALID_TEXT: &str = "field 'locale' is invalid";

/// The server-side time limit sent for `o`: the one place the driver boundary clamps (the policy
/// clamps when it builds the options, this guards a value built any other way).
pub(crate) fn driver_max_time(o: &AppliedOptions) -> std::time::Duration {
    clamped_time(o.max_time)
}

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
        .max_time(driver_max_time(o))
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
        .max_time(driver_max_time(o))
        .collation(o.collation.as_ref().map(driver_collation))
        .hint(o.hint.as_ref().map(driver_hint))
        .comment(o.comment.clone().map(Bson::String))
        .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
        .build()
}

/// The collation as the server reads it, built field by field (a serialisation that could fail
/// would drop the collation and run the explain un-collated).
pub(crate) fn collation_document(c: &CollationOption) -> Document {
    let mut out = Document::new();
    out.insert("locale", c.locale.clone());
    if let Some(strength) = c.strength {
        out.insert("strength", i32::from(u8::from(strength)));
    }
    if let Some(on) = c.case_level {
        out.insert("caseLevel", on);
    }
    if let Some(first) = c.case_first {
        let name = match first {
            CaseFirst::Upper => "upper",
            CaseFirst::Lower => "lower",
            CaseFirst::Off => "off",
        };
        out.insert("caseFirst", name);
    }
    if let Some(on) = c.numeric_ordering {
        out.insert("numericOrdering", on);
    }
    if let Some(alternate) = c.alternate {
        let name = match alternate {
            Alternate::NonIgnorable => "non-ignorable",
            Alternate::Shifted => "shifted",
        };
        out.insert("alternate", name);
    }
    out
}

/// The fields of `o` that go inside the `aggregate` of an explain command (the time limit and
/// the verbosity stay outside it). Empty when `o` sets none.
pub(crate) fn explain_fields(o: &AppliedOptions) -> Document {
    let mut fields = Document::new();
    if let Some(collation) = &o.collation {
        fields.insert("collation", collation_document(collation));
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

/// Whether a failed server selection says servers were available but none suited the criteria
/// (the driver's own wording), as opposed to no server being reachable at all.
///
/// This relies on the wording of `mongodb` 3.6.0 (`TopologyDescription::
/// server_selection_timeout_error_message`): "Server selection timeout: None of the available
/// servers suitable for criteria ..." against "... No available servers ...". The unit test
/// `a_deployment_that_is_down_stays_unreachable_whatever_the_read_preference` pins the second
/// wording with a real failed selection (`selection_failure`), and the replica-set container test
/// `secondary_on_a_one_member_set_is_read_preference_unavailable` pins the first; a driver
/// upgrade that rewords either fails one of them. Only the first is
/// an unsatisfiable read preference; the second is a lost deployment and keeps its ordinary
/// classification (`Unreachable`, `TlsHandshake`).
pub(crate) fn selection_found_servers(message: &str) -> bool {
    message.contains("None of the available servers suitable")
}

/// Classifies a driver error knowing what was sent: a missing hint, a refused collation and an
/// unsatisfiable read preference have their own error, only when the matching option was sent.
/// A failed server selection under a non-primary read preference is an unsatisfiable preference
/// only when the driver says servers were available but none matched; a deployment that cannot
/// be reached stays `Unreachable` or `TlsHandshake`, so the app learns the server is gone. The
/// server text is matched here and never kept.
pub(crate) fn classify_with(e: &mongodb::error::Error, o: &AppliedOptions) -> CoreError {
    match e.kind.as_ref() {
        ErrorKind::ServerSelection { message, .. }
            if asks_for_non_primary(o) && selection_found_servers(message) =>
        {
            CoreError::ReadPreferenceUnavailable
        }
        ErrorKind::Command(c) if c.code == CODE_BAD_VALUE => {
            let text = c.message.to_lowercase();
            if o.hint.is_some() && text.contains(HINT_NOT_FOUND_TEXT) {
                CoreError::HintNotFound
            } else if o.collation.is_some() && text.contains(LOCALE_INVALID_TEXT) {
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
