//! What the Options section and the status around it need from the draft: the pause before typed
//! text is applied, the status lines of the preview, the options a run lists, and the keys that
//! depend on the options. Pure and egui-free. Option values are user data: nothing here logs one.
use super::aggregate_options::keys_text;
use super::aggregate_view::{AggregateView, EDIT_PAUSE_SECS};
use crate::I18n;
use curlybrace_core::pipeline::{
    HintOption, MaxTimeEffect, Operation, PipelineOptions, ReadPreferenceMode,
};

/// A status line: its i18n key and parameters.
pub type StatusLine = (&'static str, Vec<(&'static str, String)>);

impl AggregateView {
    /// Text was typed into an options field at `now`: it is applied once the typing pauses.
    pub fn options_typed(self, now: f64) -> Self {
        Self {
            options_typed_at: Some(now),
            ..self
        }
    }

    /// The typed text was applied (or is being): nothing waits.
    pub fn options_applied(self) -> Self {
        Self {
            options_typed_at: None,
            ..self
        }
    }

    /// Seconds left before the typed text is applied, if some waits.
    pub fn options_due_in(&self, now: f64) -> Option<f64> {
        self.options_typed_at
            .map(|at| (at + EDIT_PAUSE_SECS - now).max(0.0))
    }

    /// Whether the waiting text is due: the typing paused, or `force` (another action follows
    /// and must see the typed text). Taking it clears the wait.
    pub fn take_options_due(self, now: f64, force: bool) -> (Self, bool) {
        let due = self.options_typed_at.is_some()
            && (force || self.options_due_in(now).is_some_and(|left| left <= 0.0));
        match due {
            true => (self.options_applied(), true),
            false => (self, false),
        }
    }
}

/// The lines under the preview that say a result was shaped by options: the read preference and
/// the hint.
pub fn options_status(options: &PipelineOptions, i18n: &I18n) -> Vec<StatusLine> {
    let mode = options.read_preference.map(|m| {
        (
            "aggregate.options.status.read_from",
            vec![("mode", i18n.t(read_preference_key(Some(m))))],
        )
    });
    let hint = options.hint.as_ref().map(|hint| {
        let name = match hint {
            HintOption::Name(name) => name.clone(),
            HintOption::Keys(keys) => keys_text(keys),
        };
        ("aggregate.options.status.hint", vec![("name", name)])
    });
    mode.into_iter().chain(hint).collect()
}

/// The label keys of the options a confirmed run carries: everything but the read preference,
/// which a run never sends (it always reads and writes on the primary).
pub fn run_option_labels(options: &PipelineOptions) -> Vec<&'static str> {
    [
        (
            options.collation.is_some(),
            "aggregate.options.collation.enabled",
        ),
        (options.hint.is_some(), "aggregate.options.hint.label"),
        (
            options.allow_disk_use.is_some(),
            "aggregate.options.allow_disk_use.label",
        ),
        (
            options.max_time_ms.is_some(),
            "aggregate.options.max_time.label",
        ),
        (options.comment.is_some(), "aggregate.options.comment.label"),
    ]
    .into_iter()
    .filter_map(|(set, key)| set.then_some(key))
    .collect()
}

/// The label key of a read preference choice (`None` is the connection's own).
pub fn read_preference_key(mode: Option<ReadPreferenceMode>) -> &'static str {
    match mode {
        None => "aggregate.options.read_preference.value.default",
        Some(ReadPreferenceMode::Primary) => "aggregate.options.read_preference.value.primary",
        Some(ReadPreferenceMode::PrimaryPreferred) => {
            "aggregate.options.read_preference.value.primary_preferred"
        }
        Some(ReadPreferenceMode::Secondary) => "aggregate.options.read_preference.value.secondary",
        Some(ReadPreferenceMode::SecondaryPreferred) => {
            "aggregate.options.read_preference.value.secondary_preferred"
        }
        Some(ReadPreferenceMode::Nearest) => "aggregate.options.read_preference.value.nearest",
    }
}

/// The key of the message for a timeout of `op`: the person's own limit has words of its own
/// when it lowered the operation's bound (an explain has no last result to keep), otherwise
/// `default_key`.
pub fn timeout_key(
    options: &PipelineOptions,
    op: Operation,
    default_key: &'static str,
) -> &'static str {
    match options.max_time_effect(op) {
        MaxTimeEffect::Lowered if op == Operation::Explain => {
            "aggregate.options.explain_timeout_set"
        }
        MaxTimeEffect::Lowered => "aggregate.options.timeout_set",
        _ => default_key,
    }
}

#[cfg(test)]
#[path = "aggregate_options_view_tests.rs"]
mod tests;
