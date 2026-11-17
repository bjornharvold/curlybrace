//! The options form of the Aggregate tab: what the person typed and picked, and the options it
//! parses to (or the problem of each field that does not). Every op returns a new form and
//! parses once, so a repaint never parses. Option values are user data: nothing here logs one.
use crate::I18n;
use bson::{Bson, Document};
use curlybrace_core::pipeline::{
    Alternate, CaseFirst, CollationOption, HintOption, OptionsError, PipelineOptions,
    ReadPreferenceMode, Strength,
};
use curlybrace_core::pipeline_options::{MAX_COMMENT_CHARS, MAX_HINT_NAME_CHARS, MAX_TIME_MS_CAP};

/// The locales the picker offers beside free text.
pub const COMMON_LOCALES: [&str; 10] = [
    "simple",
    "en",
    "en_US",
    "de",
    "de@collation=phonebook",
    "fr",
    "es",
    "ja",
    "zh",
    "zh_Hant",
];

/// A flag that may be left to the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tri {
    #[default]
    Default,
    On,
    Off,
}

impl Tri {
    pub fn of(value: Option<bool>) -> Self {
        match value {
            None => Self::Default,
            Some(true) => Self::On,
            Some(false) => Self::Off,
        }
    }

    pub fn value(self) -> Option<bool> {
        match self {
            Self::Default => None,
            Self::On => Some(true),
            Self::Off => Some(false),
        }
    }
}

/// The text fields, where a problem is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionField {
    Locale,
    Hint,
    MaxTime,
    Comment,
}

/// One change of the form.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsEdit {
    CollationOn(bool),
    Locale(String),
    Strength(Option<Strength>),
    CaseLevel(Tri),
    CaseFirst(Option<CaseFirst>),
    NumericOrdering(Tri),
    Alternate(Option<Alternate>),
    HintText(String),
    HintPicked(String),
    HintCleared,
    AllowDiskUse(Tri),
    MaxTime(String),
    Comment(String),
    ReadPreference(Option<ReadPreferenceMode>),
}

impl OptionsEdit {
    /// Typed text: it waits for Enter, focus loss or a pause before it is applied. Every other
    /// edit is a pick and applies at once.
    pub fn is_typing(&self) -> bool {
        matches!(
            self,
            Self::Locale(_) | Self::HintText(_) | Self::MaxTime(_) | Self::Comment(_)
        )
    }
}

type Problems = Vec<(OptionField, OptionsError)>;

/// The form as typed. `parsed` is recomputed by every op (never per frame).
#[derive(Clone, PartialEq)]
pub struct OptionsForm {
    pub collation_on: bool,
    pub locale: String,
    pub strength: Option<Strength>,
    pub case_level: Tri,
    pub case_first: Option<CaseFirst>,
    pub numeric_ordering: Tri,
    pub alternate: Option<Alternate>,
    pub hint_text: String,
    pub allow_disk_use: Tri,
    pub max_time: String,
    pub comment: String,
    pub read_preference: Option<ReadPreferenceMode>,
    /// The locale was edited since the collation was ticked (see `locale_pending`).
    locale_touched: bool,
    parsed: Result<PipelineOptions, Problems>,
}

/// Names the shape, never what was typed: the locale, the hint, the time limit and the comment are
/// the person's data, and the form is printed by every type that holds it.
impl std::fmt::Debug for OptionsForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let problems: Vec<&str> = self
            .parsed()
            .err()
            .unwrap_or_default()
            .iter()
            .map(|(_, e)| e.error_variant())
            .collect();
        f.debug_struct("OptionsForm")
            .field("collation_on", &self.collation_on)
            .field(
                "typed",
                &[
                    !self.locale.is_empty(),
                    !self.hint_text.is_empty(),
                    !self.max_time.is_empty(),
                    !self.comment.is_empty(),
                ],
            )
            .field("problems", &problems)
            .finish_non_exhaustive()
    }
}

impl Default for OptionsForm {
    fn default() -> Self {
        Self {
            collation_on: false,
            locale: String::new(),
            strength: None,
            case_level: Tri::Default,
            case_first: None,
            numeric_ordering: Tri::Default,
            alternate: None,
            hint_text: String::new(),
            allow_disk_use: Tri::Default,
            max_time: String::new(),
            comment: String::new(),
            read_preference: None,
            locale_touched: false,
            parsed: Ok(PipelineOptions::default()),
        }
    }
}

impl OptionsForm {
    /// The form showing `options`: a key pattern as compact JSON, the time limit as digits.
    pub fn from_options(options: &PipelineOptions) -> Self {
        let collation = options.collation.as_ref();
        let hint_text = match &options.hint {
            None => String::new(),
            Some(HintOption::Name(name)) => name.clone(),
            Some(HintOption::Keys(keys)) => keys_text(keys),
        };
        Self {
            collation_on: collation.is_some(),
            locale: collation.map(|c| c.locale.clone()).unwrap_or_default(),
            strength: collation.and_then(|c| c.strength),
            case_level: Tri::of(collation.and_then(|c| c.case_level)),
            case_first: collation.and_then(|c| c.case_first),
            numeric_ordering: Tri::of(collation.and_then(|c| c.numeric_ordering)),
            alternate: collation.and_then(|c| c.alternate),
            hint_text,
            allow_disk_use: Tri::of(options.allow_disk_use),
            max_time: options
                .max_time_ms
                .map(|ms| ms.to_string())
                .unwrap_or_default(),
            comment: options.comment.clone().unwrap_or_default(),
            read_preference: options.read_preference,
            // A loaded collation always has a locale; without one nothing is pending.
            locale_touched: false,
            parsed: Ok(options.clone()),
        }
        .reparsed()
    }

    /// The form after `edit`.
    pub fn edited(self, edit: OptionsEdit) -> Self {
        let next = match edit {
            // Ticking starts a locale that is still to be typed; unticking forgets the edit.
            OptionsEdit::CollationOn(collation_on) => Self {
                collation_on,
                locale_touched: false,
                ..self
            },
            OptionsEdit::Locale(locale) => Self {
                locale,
                locale_touched: true,
                ..self
            },
            OptionsEdit::Strength(strength) => Self { strength, ..self },
            OptionsEdit::CaseLevel(case_level) => Self { case_level, ..self },
            OptionsEdit::CaseFirst(case_first) => Self { case_first, ..self },
            OptionsEdit::NumericOrdering(numeric_ordering) => Self {
                numeric_ordering,
                ..self
            },
            OptionsEdit::Alternate(alternate) => Self { alternate, ..self },
            OptionsEdit::HintText(hint_text) | OptionsEdit::HintPicked(hint_text) => {
                Self { hint_text, ..self }
            }
            OptionsEdit::HintCleared => Self {
                hint_text: String::new(),
                ..self
            },
            OptionsEdit::AllowDiskUse(allow_disk_use) => Self {
                allow_disk_use,
                ..self
            },
            OptionsEdit::MaxTime(max_time) => Self { max_time, ..self },
            OptionsEdit::Comment(comment) => Self { comment, ..self },
            OptionsEdit::ReadPreference(read_preference) => Self {
                read_preference,
                ..self
            },
        };
        next.reparsed()
    }

    fn reparsed(self) -> Self {
        let parsed = parse(&self);
        Self { parsed, ..self }
    }

    /// The options the form says, or the problem of each field that has one.
    pub fn parsed(&self) -> Result<&PipelineOptions, &[(OptionField, OptionsError)]> {
        self.parsed.as_ref().map_err(Vec::as_slice)
    }

    /// The problem of `field`, if it has one.
    pub fn problem(&self, field: OptionField) -> Option<OptionsError> {
        let found = self
            .parsed()
            .err()
            .and_then(|all| all.iter().find_map(|(f, e)| (*f == field).then_some(*e)));
        // A pending locale is no problem of the form, but the field says what to do.
        found.or_else(|| {
            (field == OptionField::Locale && self.locale_pending())
                .then_some(OptionsError::LocaleEmpty)
        })
    }

    /// The collation is ticked and its locale was never edited since: the person is about to type
    /// it. Nothing is applied for it and nothing is blocked (the last preview and the counts
    /// stay), but a run waits, since it would not carry the collation the screen shows. Once the
    /// locale has been edited, empty again included, an empty locale is a problem.
    pub fn locale_pending(&self) -> bool {
        self.collation_on && self.locale.is_empty() && !self.locale_touched
    }

    /// Some field has a problem: nothing that sends a pipeline may go on.
    pub fn is_blocked(&self) -> bool {
        self.parsed.is_err()
    }
}

/// The field a rule of the core belongs to.
fn field_of(error: OptionsError) -> OptionField {
    match error {
        OptionsError::LocaleEmpty | OptionsError::LocaleInvalid => OptionField::Locale,
        OptionsError::HintEmpty
        | OptionsError::HintNameTooLong
        | OptionsError::HintKeysInvalid
        | OptionsError::HintJson => OptionField::Hint,
        OptionsError::MaxTimeRange => OptionField::MaxTime,
        OptionsError::CommentEmpty
        | OptionsError::CommentTooLong
        | OptionsError::CommentControl => OptionField::Comment,
    }
}

/// The collation the form asks for; none when it is off (its fields are then not looked at).
fn collation_of(form: &OptionsForm) -> Result<Option<CollationOption>, OptionsError> {
    if !form.collation_on || form.locale_pending() {
        return Ok(None);
    }
    let collation = CollationOption {
        locale: form.locale.clone(),
        strength: form.strength,
        case_level: form.case_level.value(),
        case_first: form.case_first,
        numeric_ordering: form.numeric_ordering.value(),
        alternate: form.alternate,
    };
    let checked = PipelineOptions {
        collation: Some(collation),
        ..PipelineOptions::default()
    }
    .validated()?;
    Ok(checked.collation)
}

fn hint_of(text: &str) -> Result<Option<HintOption>, OptionsError> {
    match text {
        "" => Ok(None),
        text => HintOption::parse(text).map(Some),
    }
}

/// Digits only (spaces around them are ignored), then the core's range; anything else is out of
/// range, text of only spaces included: nothing typed is silently dropped.
fn max_time_of(text: &str) -> Result<Option<u64>, OptionsError> {
    if text.is_empty() {
        return Ok(None);
    }
    let text = text.trim();
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let ms = text.parse::<u64>().ok().filter(|_| digits);
    match ms {
        Some(ms) if (1..=MAX_TIME_MS_CAP).contains(&ms) => Ok(Some(ms)),
        _ => Err(OptionsError::MaxTimeRange),
    }
}

fn comment_of(text: &str) -> Result<Option<String>, OptionsError> {
    if text.is_empty() {
        return Ok(None);
    }
    let checked = PipelineOptions {
        comment: Some(text.to_owned()),
        ..PipelineOptions::default()
    }
    .validated()?;
    Ok(checked.comment)
}

/// Each field is checked on its own so several problems show at once; the whole is then
/// validated again as the core does when it loads a saved pipeline.
fn parse(form: &OptionsForm) -> Result<PipelineOptions, Problems> {
    let collation = collation_of(form);
    let hint = hint_of(&form.hint_text);
    let max_time = max_time_of(&form.max_time);
    let comment = comment_of(&form.comment);
    let problems: Problems = [
        collation.as_ref().err(),
        hint.as_ref().err(),
        max_time.as_ref().err(),
        comment.as_ref().err(),
    ]
    .into_iter()
    .flatten()
    .map(|e| (field_of(*e), *e))
    .collect();
    if !problems.is_empty() {
        return Err(problems);
    }
    let (Ok(collation), Ok(hint), Ok(max_time_ms), Ok(comment)) =
        (collation, hint, max_time, comment)
    else {
        return Err(problems);
    };
    PipelineOptions {
        collation,
        hint,
        allow_disk_use: form.allow_disk_use.value(),
        max_time_ms,
        comment,
        read_preference: form.read_preference,
    }
    .validated()
    .map_err(|e| vec![(field_of(e), e)])
}

/// A key pattern as compact JSON, in the order it was typed: `{"status":1,"title":"text"}`.
pub fn keys_text(keys: &Document) -> String {
    let fields: Vec<String> = keys
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Bson::Int32(n) => n.to_string(),
                Bson::Int64(n) => n.to_string(),
                Bson::Double(n) => format!("{}", *n as i64),
                Bson::String(kind) => serde_json::Value::String(kind.clone()).to_string(),
                _ => String::new(),
            };
            format!("{}:{value}", serde_json::Value::String(key.clone()))
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}

/// The i18n key of a problem: one literal per rule, so the catalogue test sees every one.
pub fn problem_key(error: OptionsError) -> &'static str {
    match error {
        OptionsError::LocaleEmpty => "aggregate.options.problem.locale_empty",
        OptionsError::LocaleInvalid => "aggregate.options.problem.locale_invalid",
        OptionsError::HintEmpty => "aggregate.options.problem.hint_empty",
        OptionsError::HintNameTooLong => "aggregate.options.problem.hint_name_too_long",
        OptionsError::HintKeysInvalid => "aggregate.options.problem.hint_keys_invalid",
        OptionsError::HintJson => "aggregate.options.problem.hint_json",
        OptionsError::MaxTimeRange => "aggregate.options.problem.max_time_range",
        OptionsError::CommentEmpty => "aggregate.options.problem.comment_empty",
        OptionsError::CommentTooLong => "aggregate.options.problem.comment_too_long",
        OptionsError::CommentControl => "aggregate.options.problem.comment_control",
    }
}

/// The words of a problem; a message that names a limit has it filled in from the constant that
/// enforces it, so the text can never disagree with the rule.
pub fn problem_message(i18n: &I18n, error: OptionsError) -> String {
    let key = problem_key(error);
    let max = match error {
        OptionsError::CommentTooLong => MAX_COMMENT_CHARS.to_string(),
        OptionsError::HintNameTooLong => MAX_HINT_NAME_CHARS.to_string(),
        OptionsError::MaxTimeRange => MAX_TIME_MS_CAP.to_string(),
        _ => return i18n.t(key),
    };
    i18n.t_with(key, &[("max", &max)])
}

#[cfg(test)]
#[path = "aggregate_options_tests.rs"]
mod tests;
