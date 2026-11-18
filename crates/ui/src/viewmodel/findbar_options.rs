//! The query bar's Query options: the form's state around the find bar (is the section open, is the
//! form blocked, what an edit does to an error the options caused) and the lines that say which
//! options shaped the shown page. Every op returns a new value. Option values are user data:
//! nothing here logs one.
use super::aggregate_options::{OptionsEdit, OptionsForm, Tri};
use super::aggregate_options_view::{StatusLine, options_status};
use super::findbar::FindBarState;
use super::findbar_more::hidden_count;
use crate::I18n;
use curlybrace_core::pipeline::{MaxTimeEffect, ReadPreferenceMode};
use curlybrace_core::query_options::{FindOperation, QueryOptions};

/// How many options the form holds: the ones set, counted from what is typed so a form with a
/// problem still says how much it holds.
pub fn options_set_count(form: &OptionsForm) -> usize {
    [
        form.collation_on,
        !form.hint_text.is_empty(),
        form.allow_disk_use != Tri::Default,
        !form.max_time.is_empty(),
        !form.comment.is_empty(),
        form.read_preference.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count()
}

/// How many fields behind "More options" hold a value: Sort, Projection, Skip and Limit, and each
/// option set.
pub fn more_set_count(state: &FindBarState) -> usize {
    hidden_count(&state.text) + options_set_count(&state.options)
}

/// Whether the Query options section is open: the choice made, else open while an option is set.
pub fn options_open(state: &FindBarState) -> bool {
    state
        .options_open
        .unwrap_or_else(|| options_set_count(&state.options) > 0)
}

impl FindBarState {
    /// The section flipped from what it shows now; the choice then sticks until Clear.
    pub fn options_toggled(&self) -> Self {
        Self {
            options_open: Some(!options_open(self)),
            ..self.clone()
        }
    }

    /// Opening by itself happens once: shown open with no choice made, that becomes the choice,
    /// so emptying the form does not close the section under the cursor.
    pub fn options_settled(&self) -> Self {
        match (self.options_open, options_open(self)) {
            (None, true) => Self {
                options_open: Some(true),
                ..self.clone()
            },
            _ => self.clone(),
        }
    }

    /// The form after `edit`. An error the options caused is about the form as it was: it goes.
    pub fn options_edited(self, edit: OptionsEdit) -> Self {
        let (last_error, error_from_options) = self.error_after_options_change();
        Self {
            options: self.options.edited(edit),
            last_error,
            error_from_options,
            ..self
        }
    }

    /// The form emptied (Reset options); the section's open choice stays.
    pub fn options_reset(self) -> Self {
        let (last_error, error_from_options) = self.error_after_options_change();
        Self {
            options: OptionsForm::default(),
            last_error,
            error_from_options,
            ..self
        }
    }

    fn error_after_options_change(&self) -> (Option<curlybrace_core::error::CoreError>, bool) {
        match self.error_from_options {
            true => (None, false),
            false => (self.last_error.clone(), false),
        }
    }

    /// Some option has a problem, or a collation still waits for its locale: Find and Copy as
    /// code would not carry what the screen shows, so both are off.
    pub fn options_blocked(&self) -> bool {
        self.options.is_blocked() || self.options.locale_pending()
    }

    /// The key of why Copy as code has nothing to offer because of the options, if so.
    pub fn find_code_block(&self) -> Option<&'static str> {
        self.options_blocked().then_some("query.options.blocked")
    }
}

/// The weak lines that say which options shaped the shown page: the read preference, the hint,
/// the collation, a time limit that lowered the find's own, disk use, and, when it read from a
/// node other than the primary, that recent writes may not show yet.
pub fn page_options_status(options: &QueryOptions, i18n: &I18n) -> Vec<StatusLine> {
    let collation = options.collation.as_ref().map(|c| {
        (
            "query.options.status.collation",
            vec![("locale", c.locale.clone())],
        )
    });
    let max_time = options
        .max_time_ms
        .filter(|_| options.find_max_time_effect(FindOperation::Find) == MaxTimeEffect::Lowered)
        .map(|ms| {
            (
                "query.options.status.max_time",
                vec![("ms", ms.to_string())],
            )
        });
    let disk_use = (options.allow_disk_use == Some(true))
        .then(|| ("query.options.status.disk_use", Vec::new()));
    let secondary = options
        .read_preference
        .filter(|mode| *mode != ReadPreferenceMode::Primary)
        .map(|_| ("query.options.status.secondary_note", Vec::new()));
    options_status(options, i18n)
        .into_iter()
        .chain(collation)
        .chain(max_time)
        .chain(disk_use)
        .chain(secondary)
        .collect()
}

#[cfg(test)]
#[path = "findbar_options_tests.rs"]
mod tests;
