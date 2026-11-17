//! The draft's applied options and the form that edits them. A form edit never sends anything by
//! itself: the caller applies a valid form, which goes through `changed()` so the request number,
//! the "modified" memo, the run dialog and a running count sequence react as for a stage edit.
//! An invalid form blocks everything that sends a pipeline and leaves the last good preview.
use super::aggregate::AggregateState;
use super::aggregate_hint_list::HintListUi;
use super::aggregate_options::{OptionsEdit, OptionsForm};
use super::aggregate_preview_state::{PreviewError, PreviewState};

/// The options section of one session's draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionsUi {
    pub form: OptionsForm,
    /// Whether the section is expanded.
    pub open: bool,
    /// The collection's index names for the hint picker.
    pub hint: HintListUi,
}

impl OptionsUi {
    /// The hint list forgotten: the pipeline now runs against another collection.
    pub fn hint_reset(self) -> Self {
        Self {
            hint: self.hint.reset(),
            ..self
        }
    }
}

/// What applying the form did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// A valid form that says what is already applied.
    Unchanged,
    /// The options changed (the request number moved).
    Changed,
    /// The form is valid and the options did not change, but the pane showed the block: the
    /// caller previews again.
    Unblocked,
    /// The form has a problem; the last applied options stay.
    Blocked,
}

impl AggregateState {
    /// The section opened or closed; nothing else changes.
    pub fn options_toggled(self) -> Self {
        let OptionsUi { form, open, hint } = self.options_ui;
        // Opening again lets a list that could not be read be asked for again.
        let hint = match open {
            true => hint,
            false => hint.retried(),
        };
        Self {
            options_ui: OptionsUi {
                form,
                open: !open,
                hint,
            },
            ..self
        }
    }

    /// The form after `edit`. Only the form: whether the edit applies is the caller's call. The
    /// "modified" memo is forgotten, since a form with a problem counts as modified.
    pub fn options_edited(self, edit: OptionsEdit) -> Self {
        let OptionsUi { form, open, hint } = self.options_ui;
        Self {
            options_ui: OptionsUi {
                form: form.edited(edit),
                open,
                hint,
            },
            saved: self.saved.with_modified_forgotten(),
            ..self
        }
    }

    /// Applies the form: a valid one that differs from the applied options replaces them through
    /// `changed()`.
    pub fn options_applied(self) -> (Self, Applied) {
        let wanted = match self.options_ui.form.parsed() {
            Err(_) => return (self, Applied::Blocked),
            Ok(options) => options.clone(),
        };
        if wanted != self.options {
            let next = Self {
                options: wanted,
                ..self
            };
            return (next.changed(), Applied::Changed);
        }
        let was_blocked = matches!(
            self.preview,
            PreviewState::Failed {
                error: PreviewError::Options,
                ..
            }
        );
        match was_blocked {
            true => (self, Applied::Unblocked),
            false => (self, Applied::Unchanged),
        }
    }

    /// The form emptied (the section stays open) and applied.
    pub fn options_reset(self) -> (Self, Applied) {
        let OptionsUi { open, hint, .. } = self.options_ui;
        Self {
            options_ui: OptionsUi {
                form: OptionsForm::default(),
                open,
                hint,
            },
            saved: self.saved.with_modified_forgotten(),
            ..self
        }
        .options_applied()
    }

    /// Whether the form has a problem.
    pub fn options_blocked(&self) -> bool {
        self.options_ui.form.is_blocked()
    }

    /// A run must wait: the form has a problem, or a ticked collation still lacks its locale (a
    /// run would not carry the collation the screen shows).
    pub fn options_hold_run(&self) -> bool {
        self.options_blocked() || self.options_ui.form.locale_pending()
    }

    /// The pane says the options block the preview, keeping the last good one; a preview that
    /// waited for the slot is dropped and a running count sequence ends, so the last valid
    /// options are not sent behind the person's back.
    pub fn preview_blocked(self) -> Self {
        let last = self.preview.into_last();
        Self {
            preview: PreviewState::Failed {
                error: PreviewError::Options,
                last,
            },
            queued: None,
            counts: self.counts.cancelled(),
            ..self
        }
    }

    /// The hint list replaced by `f(list)`.
    pub fn hint_edited(self, f: impl FnOnce(HintListUi) -> HintListUi) -> Self {
        let OptionsUi { form, open, hint } = self.options_ui;
        Self {
            options_ui: OptionsUi {
                form,
                open,
                hint: f(hint),
            },
            ..self
        }
    }
}

#[cfg(test)]
#[path = "aggregate_options_state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_options_stale_tests.rs"]
mod stale_tests;
