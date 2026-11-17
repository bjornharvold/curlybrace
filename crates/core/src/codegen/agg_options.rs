//! The options of an aggregate in a form every renderer shares: what the code sets, with the
//! read preference dropped when the code writes (the confirmed run always reads on the primary,
//! so code that keeps `$out` / `$merge` must not say otherwise).
//!
//! Strings from the options reach the code only through a language's own `Syntax::string`, so
//! they cannot break out of a literal.

use bson::{Bson, Document};

use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, render, render_document};
use crate::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};

/// What the code sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CodeAggOptions {
    pub collation: Option<CollationOption>,
    pub hint: Option<HintOption>,
    pub allow_disk_use: Option<bool>,
    pub max_time_ms: Option<i32>,
    pub comment: Option<String>,
    pub read_preference: Option<ReadPreferenceMode>,
    /// A read preference was set and left out because the code ends with a write stage.
    pub read_preference_left_out: bool,
}

/// The note the code carries when the read preference was left out (generated code is not UI
/// text, so it is English).
const LEFT_OUT_NOTE: &str = "Note: this pipeline writes with $out or $merge, which runs on the primary, so the read preference is left out.";

impl CodeAggOptions {
    /// The options the code sets: nothing when `include` is off or nothing is set; the options
    /// must validate (the run would refuse them).
    pub(crate) fn of(
        options: &PipelineOptions,
        include: bool,
        writes: bool,
    ) -> Result<Self, CodegenError> {
        if !include || options.is_empty() {
            return Ok(Self::default());
        }
        let options = options
            .clone()
            .validated()
            .map_err(|_| CodegenError::InvalidOptions)?;
        let max_time_ms = options
            .max_time_ms
            .map(i32::try_from)
            .transpose()
            .map_err(|_| CodegenError::InvalidOptions)?;
        Ok(Self {
            collation: options.collation,
            hint: options.hint,
            allow_disk_use: options.allow_disk_use,
            max_time_ms,
            comment: options.comment,
            read_preference: options.read_preference.filter(|_| !writes),
            read_preference_left_out: writes && options.read_preference.is_some(),
        })
    }

    /// The fixed note line, with its comment marker (`//` or `#`).
    pub(crate) fn note(&self, comment: &str) -> Option<String> {
        self.read_preference_left_out
            .then(|| format!("{comment} {LEFT_OUT_NOTE}"))
    }

    /// `body` with the note as its first line, when there is one.
    pub(crate) fn with_note(&self, comment: &str, body: String) -> String {
        match self.note(comment) {
            Some(line) => format!("{line}\n{body}"),
            None => body,
        }
    }
}

pub(crate) fn mode_camel(mode: ReadPreferenceMode) -> &'static str {
    match mode {
        ReadPreferenceMode::Primary => "primary",
        ReadPreferenceMode::PrimaryPreferred => "primaryPreferred",
        ReadPreferenceMode::Secondary => "secondary",
        ReadPreferenceMode::SecondaryPreferred => "secondaryPreferred",
        ReadPreferenceMode::Nearest => "nearest",
    }
}

pub(crate) fn mode_snake(mode: ReadPreferenceMode) -> &'static str {
    match mode {
        ReadPreferenceMode::Primary => "primary",
        ReadPreferenceMode::PrimaryPreferred => "primary_preferred",
        ReadPreferenceMode::Secondary => "secondary",
        ReadPreferenceMode::SecondaryPreferred => "secondary_preferred",
        ReadPreferenceMode::Nearest => "nearest",
    }
}

pub(crate) fn mode_upper_snake(mode: ReadPreferenceMode) -> &'static str {
    match mode {
        ReadPreferenceMode::Primary => "PRIMARY",
        ReadPreferenceMode::PrimaryPreferred => "PRIMARY_PREFERRED",
        ReadPreferenceMode::Secondary => "SECONDARY",
        ReadPreferenceMode::SecondaryPreferred => "SECONDARY_PREFERRED",
        ReadPreferenceMode::Nearest => "NEAREST",
    }
}

pub(crate) fn mode_pascal(mode: ReadPreferenceMode) -> &'static str {
    match mode {
        ReadPreferenceMode::Primary => "Primary",
        ReadPreferenceMode::PrimaryPreferred => "PrimaryPreferred",
        ReadPreferenceMode::Secondary => "Secondary",
        ReadPreferenceMode::SecondaryPreferred => "SecondaryPreferred",
        ReadPreferenceMode::Nearest => "Nearest",
    }
}

/// A collation enum's member as `(PascalCase, UPPER_SNAKE)`: C# and Rust use the first, Java the
/// second.
pub(crate) type Words = (&'static str, &'static str);

pub(crate) fn strength_words(strength: Strength) -> Words {
    match strength {
        Strength::Primary => ("Primary", "PRIMARY"),
        Strength::Secondary => ("Secondary", "SECONDARY"),
        Strength::Tertiary => ("Tertiary", "TERTIARY"),
        Strength::Quaternary => ("Quaternary", "QUATERNARY"),
        Strength::Identical => ("Identical", "IDENTICAL"),
    }
}

pub(crate) fn case_first_words(case_first: CaseFirst) -> Words {
    match case_first {
        CaseFirst::Upper => ("Upper", "UPPER"),
        CaseFirst::Lower => ("Lower", "LOWER"),
        CaseFirst::Off => ("Off", "OFF"),
    }
}

pub(crate) fn alternate_words(alternate: Alternate) -> Words {
    match alternate {
        Alternate::NonIgnorable => ("NonIgnorable", "NON_IGNORABLE"),
        Alternate::Shifted => ("Shifted", "SHIFTED"),
    }
}

/// The hint's key pattern in a language's own spelling; names go through `Syntax::string`.
pub(crate) type HintKeys<'a> = &'a dyn Fn(&Document) -> Result<String, CodegenError>;

/// `(rendered key, rendered value)` of the options that are plain BSON, in the order collation,
/// hint, allowDiskUse, maxTimeMS, comment. `key` spells MongoDB's name in the language; `level` is
/// the indent level of the lines the entries end up on (nested documents break relative to it).
pub(crate) fn plain_entries(
    syn: &dyn Syntax,
    agg: &CodeAggOptions,
    key: fn(&str) -> String,
    level: usize,
    hint_keys: Option<HintKeys<'_>>,
) -> Result<Vec<(String, String)>, CodegenError> {
    let mut out = Vec::new();
    if let Some(collation) = &agg.collation {
        let value = render_document(syn, &collation.to_document(), level)?;
        out.push((key("collation"), value));
    }
    if let Some(hint) = &agg.hint {
        let value = match (hint, hint_keys) {
            (HintOption::Keys(keys), Some(spell)) => spell(keys)?,
            (hint, _) => render(syn, &hint.to_bson(), level)?,
        };
        out.push((key("hint"), value));
    }
    if let Some(on) = agg.allow_disk_use {
        out.push((key("allowDiskUse"), syn.boolean(on)));
    }
    if let Some(ms) = agg.max_time_ms {
        out.push((key("maxTimeMS"), syn.int32(ms)));
    }
    if let Some(comment) = &agg.comment {
        out.push((
            key("comment"),
            render(syn, &Bson::String(comment.clone()), level)?,
        ));
    }
    Ok(out)
}
