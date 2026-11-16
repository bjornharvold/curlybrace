//! The options of an aggregate (collation, hint, `allowDiskUse`, `maxTimeMS`, comment, read
//! preference): a validated value, saved with a pipeline and sent by every operation that runs
//! it (see `pipeline_options_apply` for what each operation sends).
//!
//! Option values are user data: nothing here logs one, and an [`OptionsError`] names the rule
//! that failed, never the value.

use bson::{Bson, Document};
use serde::{Deserialize, Serialize};

/// Longest `maxTimeMS` an option may ask for. It only ever lowers an operation's own bound.
pub const MAX_TIME_MS_CAP: u64 = 60_000;
/// Most characters of a collation locale.
pub const MAX_LOCALE_CHARS: usize = 32;
/// Most characters of an index name used as a hint.
pub const MAX_HINT_NAME_CHARS: usize = 256;
/// Most characters of a comment.
pub const MAX_COMMENT_CHARS: usize = 256;

/// Collation strength, written as the integer MongoDB takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum Strength {
    Primary = 1,
    Secondary,
    Tertiary,
    Quaternary,
    Identical,
}

impl From<Strength> for u8 {
    fn from(strength: Strength) -> u8 {
        strength as u8
    }
}

impl TryFrom<u8> for Strength {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, String> {
        match value {
            1 => Ok(Self::Primary),
            2 => Ok(Self::Secondary),
            3 => Ok(Self::Tertiary),
            4 => Ok(Self::Quaternary),
            5 => Ok(Self::Identical),
            _ => Err("collation strength is 1 to 5".into()),
        }
    }
}

/// Whether uppercase or lowercase sorts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseFirst {
    Upper,
    Lower,
    Off,
}

/// Whether whitespace and punctuation are ignorable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Alternate {
    NonIgnorable,
    Shifted,
}

/// The collation an aggregate asks for; every field but the locale is the server's default when
/// unset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollationOption {
    pub locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<Strength>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_level: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_first: Option<CaseFirst>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_ordering: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate: Option<Alternate>,
}

/// An index to force: by name, or by key pattern.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HintOption {
    Name(String),
    Keys(Document),
}

/// The read preference mode of a read (tags and staleness are not offered).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadPreferenceMode {
    Primary,
    PrimaryPreferred,
    Secondary,
    SecondaryPreferred,
    Nearest,
}

/// Which rule an option broke. No value is carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsError {
    LocaleEmpty,
    LocaleInvalid,
    HintEmpty,
    HintNameTooLong,
    HintKeysInvalid,
    HintJson,
    MaxTimeRange,
    CommentEmpty,
    CommentTooLong,
    CommentControl,
}

impl OptionsError {
    /// A stable snake_case name, safe for logs; the UI's `aggregate.options.problem.*` keys use it.
    pub fn error_variant(self) -> &'static str {
        match self {
            Self::LocaleEmpty => "locale_empty",
            Self::LocaleInvalid => "locale_invalid",
            Self::HintEmpty => "hint_empty",
            Self::HintNameTooLong => "hint_name_too_long",
            Self::HintKeysInvalid => "hint_keys_invalid",
            Self::HintJson => "hint_json",
            Self::MaxTimeRange => "max_time_range",
            Self::CommentEmpty => "comment_empty",
            Self::CommentTooLong => "comment_too_long",
            Self::CommentControl => "comment_control",
        }
    }
}

/// The options of one aggregate. `default()` is "nothing set": it sends what every operation
/// sent before options existed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawOptions", rename_all = "camelCase")]
pub struct PipelineOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collation: Option<CollationOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<HintOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_disk_use: Option<bool>,
    #[serde(rename = "maxTimeMS", skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_preference: Option<ReadPreferenceMode>,
}

/// The loaded shape of [`PipelineOptions`]: unknown keys are ignored, and the checked value is
/// built only through [`PipelineOptions::validated`], so a file with an invalid value is refused.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOptions {
    collation: Option<CollationOption>,
    hint: Option<HintOption>,
    allow_disk_use: Option<bool>,
    #[serde(rename = "maxTimeMS")]
    max_time_ms: Option<u64>,
    comment: Option<String>,
    read_preference: Option<ReadPreferenceMode>,
}

impl TryFrom<RawOptions> for PipelineOptions {
    type Error = String;

    fn try_from(raw: RawOptions) -> Result<Self, String> {
        PipelineOptions {
            collation: raw.collation,
            hint: raw.hint,
            allow_disk_use: raw.allow_disk_use,
            max_time_ms: raw.max_time_ms,
            comment: raw.comment,
            read_preference: raw.read_preference,
        }
        .validated()
        .map_err(|e| e.error_variant().to_owned())
    }
}

impl PipelineOptions {
    /// The options with locale, comment and hint name trimmed, or the first rule they break.
    pub fn validated(self) -> Result<Self, OptionsError> {
        let collation = self.collation.map(validated_collation).transpose()?;
        let hint = self.hint.map(validated_hint).transpose()?;
        if let Some(ms) = self.max_time_ms {
            if !(1..=MAX_TIME_MS_CAP).contains(&ms) {
                return Err(OptionsError::MaxTimeRange);
            }
        }
        let comment = self.comment.map(validated_comment).transpose()?;
        Ok(Self {
            collation,
            hint,
            comment,
            ..self
        })
    }

    /// Nothing is set (equal to the default).
    pub fn is_empty(&self) -> bool {
        self.set_count() == 0
    }

    /// How many top-level options are set; a collation counts once however many fields it has.
    pub fn set_count(&self) -> usize {
        [
            self.collation.is_some(),
            self.hint.is_some(),
            self.allow_disk_use.is_some(),
            self.max_time_ms.is_some(),
            self.comment.is_some(),
            self.read_preference.is_some(),
        ]
        .into_iter()
        .filter(|set| *set)
        .count()
    }
}

fn validated_collation(collation: CollationOption) -> Result<CollationOption, OptionsError> {
    let locale = collation.locale.trim().to_owned();
    if locale.is_empty() {
        return Err(OptionsError::LocaleEmpty);
    }
    if !is_locale(&locale) {
        return Err(OptionsError::LocaleInvalid);
    }
    Ok(CollationOption {
        locale,
        ..collation
    })
}

/// `simple`, or an ICU-style id: `^[A-Za-z]{2,8}(_[A-Za-z0-9]{2,8})*(@collation=[a-z]+)?$`.
fn is_locale(locale: &str) -> bool {
    if locale.chars().count() > MAX_LOCALE_CHARS {
        return false;
    }
    if locale == "simple" {
        return true;
    }
    let (id, variant) = match locale.split_once('@') {
        Some((id, rest)) => (id, Some(rest)),
        None => (locale, None),
    };
    let variant_ok = variant.is_none_or(|v| {
        v.strip_prefix("collation=")
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase()))
    });
    let mut parts = id.split('_');
    let language_ok = parts.next().is_some_and(|language| {
        (2..=8).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_alphabetic())
    });
    let subtags_ok = parts
        .all(|tag| (2..=8).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphanumeric()));
    variant_ok && language_ok && subtags_ok
}

fn validated_hint(hint: HintOption) -> Result<HintOption, OptionsError> {
    match hint {
        HintOption::Name(name) => validated_hint_name(&name).map(HintOption::Name),
        HintOption::Keys(keys) => {
            if keys_are_valid(&keys) {
                Ok(HintOption::Keys(narrowed(keys)))
            } else {
                Err(OptionsError::HintKeysInvalid)
            }
        }
    }
}

/// A name, trimmed; a name with a control character names no index, so it reads as empty.
fn validated_hint_name(name: &str) -> Result<String, OptionsError> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(OptionsError::HintEmpty);
    }
    if name.chars().count() > MAX_HINT_NAME_CHARS {
        return Err(OptionsError::HintNameTooLong);
    }
    Ok(name.to_owned())
}

/// The key pattern with whole numbers as `Int32`, so a value read back from JSON (which widens
/// them) equals the one typed.
fn narrowed(keys: Document) -> Document {
    keys.into_iter()
        .map(|(key, value)| match value {
            Bson::Int64(n) => match i32::try_from(n) {
                Ok(small) => (key, Bson::Int32(small)),
                Err(_) => (key, Bson::Int64(n)),
            },
            other => (key, other),
        })
        .collect()
}

fn is_direction(value: &Bson) -> bool {
    match value {
        Bson::Int32(n) => matches!(n, 1 | -1),
        Bson::Int64(n) => matches!(n, 1 | -1),
        Bson::Double(n) => *n == 1.0 || *n == -1.0,
        _ => false,
    }
}

/// Non-empty; each value a direction (1 or -1) or an index type name; `$natural` only alone.
fn keys_are_valid(keys: &Document) -> bool {
    if keys.is_empty() {
        return false;
    }
    if keys.contains_key("$natural") {
        return keys.len() == 1 && keys.values().all(is_direction);
    }
    keys.values().all(|value| match value {
        Bson::String(kind) => !kind.is_empty(),
        other => is_direction(other),
    })
}

fn validated_comment(comment: String) -> Result<String, OptionsError> {
    let comment = comment.trim();
    if comment.is_empty() {
        return Err(OptionsError::CommentEmpty);
    }
    if comment.chars().any(char::is_control) {
        return Err(OptionsError::CommentControl);
    }
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(OptionsError::CommentTooLong);
    }
    Ok(comment.to_owned())
}

impl HintOption {
    /// Reads typed text: starting with `{` it is a key pattern (extended JSON), otherwise an
    /// index name. The result is validated like any hint.
    pub fn parse(text: &str) -> Result<HintOption, OptionsError> {
        let text = text.trim();
        let hint = if text.starts_with('{') {
            let keys =
                crate::ejson::editor_json_to_doc(text).map_err(|_| OptionsError::HintJson)?;
            HintOption::Keys(keys)
        } else {
            HintOption::Name(text.to_owned())
        };
        validated_hint(hint)
    }
}

#[cfg(test)]
#[path = "pipeline_options_tests.rs"]
mod tests;
