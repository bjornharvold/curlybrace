//! The per-stage counts on the Aggregate tab: the Count stages / Cancel counting button in the
//! header and the count line of each stage card. Draws what it is given and returns what the
//! person did; the reducer decides what happens. The line keeps its height whatever it says, so
//! a count arriving does not move the cards below it.
use super::aggregate::pipeline_error_text;
use crate::I18n;
use crate::app::CountsAction;
use crate::theme::{gate, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_counts::{CountsUi, Progress, StageCount};
use crate::viewmodel::aggregate_counts_plan::{CountsBlock, NotCounted};
use crate::viewmodel::aggregate_view::AggregateView;
use curlybrace_core::pipeline::{COUNT_CAP, Stage};
use eframe::egui;

/// Why Count stages is off, in words; `None` when it is on. `typed` are the stages with the text
/// still being typed applied. The plan is remembered between repaints (see
/// `viewmodel::aggregate_counts_plan`).
pub fn counts_blocked(
    draft: &AggregateState,
    typed: &[Stage],
    view: &AggregateView,
    i18n: &I18n,
) -> Option<String> {
    match &*view.count_plan(draft, typed) {
        Err(CountsBlock::Pipeline(error)) => Some(pipeline_error_text(error, i18n)),
        Err(CountsBlock::TextNotApplied) => {
            Some(i18n.t("aggregate.counts.disabled.text_not_applied"))
        }
        Err(CountsBlock::Nothing) => Some(i18n.t("aggregate.counts.disabled.nothing")),
        Ok(_) => None,
    }
}

/// Count stages, or Cancel counting while a sequence runs; the action when it was clicked while
/// on. `blocked` is the reason Count stages is off, shown as its tooltip and its accessible
/// description.
pub fn show_counts_button(
    ui: &mut egui::Ui,
    running: bool,
    blocked: Option<&str>,
    i18n: &I18n,
) -> Option<CountsAction> {
    if running {
        let cancel = tonal_button(ui, i18n.t("aggregate.counts.cancel"))
            .on_hover_text(i18n.t("aggregate.counts.cancel_hint"));
        return cancel.clicked().then_some(CountsAction::Cancel);
    }
    let label = i18n.t("aggregate.counts.button");
    let response = match blocked {
        None => {
            let cap = COUNT_CAP.to_string();
            let hint = i18n.t_with("aggregate.counts.button_hint", &[("cap", &cap)]);
            tonal_button(ui, label).on_hover_text(hint)
        }
        Some(reason) => {
            let off = gate(ui, false, |ui| tonal_button(ui, label)).on_hover_text(reason);
            let why = reason.to_string();
            ui.ctx()
                .accesskit_node_builder(off.id, |node| node.set_description(why));
            off
        }
    };
    (response.clicked() && blocked.is_none()).then_some(CountsAction::Start)
}

/// A card's count text and whether it is an error.
pub(super) fn count_text(count: &StageCount, i18n: &I18n) -> (String, bool) {
    match count {
        StageCount::Exact(1) => (i18n.t("aggregate.counts.one"), false),
        StageCount::Exact(n) => (
            i18n.t_with("aggregate.counts.many", &[("n", &n.to_string())]),
            false,
        ),
        StageCount::AtLeast(n) => (
            i18n.t_with("aggregate.counts.at_least", &[("n", &n.to_string())]),
            false,
        ),
        StageCount::Failed(error) => (
            i18n.t_with("aggregate.counts.failed", &[("error", &i18n.error(error))]),
            true,
        ),
        StageCount::NotCounted(reason) => (i18n.t(not_counted_key(*reason)), false),
        StageCount::TimedOut => (i18n.t("aggregate.counts.timed_out"), true),
    }
}

fn not_counted_key(reason: NotCounted) -> &'static str {
    match reason {
        NotCounted::Invalid => "aggregate.counts.not_counted.invalid",
        NotCounted::AfterInvalid => "aggregate.counts.not_counted.after_invalid",
        NotCounted::WriteStage => "aggregate.counts.not_counted.write_stage",
        NotCounted::NestedWrite => "aggregate.counts.not_counted.nested_write",
    }
}

/// What one card's count line says: the progress of a running sequence, else the result.
pub(super) fn line_text(
    counts: &CountsUi,
    (index, key): (usize, u64),
    i18n: &I18n,
) -> Option<(String, bool)> {
    match counts.progress_of(index) {
        Some(Progress::Counting) => Some((i18n.t("aggregate.counts.counting"), false)),
        Some(Progress::Waiting) => Some((i18n.t("aggregate.counts.waiting"), false)),
        None => counts.result_of(key).map(|c| count_text(c, i18n)),
    }
}

/// The count line of the card at `index` with identity `key`: nothing for a disabled stage (the
/// height is kept), the count otherwise, marked when the stages changed since it was counted.
pub(super) fn show_count_line(
    ui: &mut egui::Ui,
    counts: &CountsUi,
    (index, key, enabled): (usize, u64, bool),
    stale: bool,
    i18n: &I18n,
) {
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(ui.spacing().interact_size.y);
        let line = enabled
            .then(|| line_text(counts, (index, key), i18n))
            .flatten();
        let Some((text, error)) = line else {
            return;
        };
        match error {
            true => ui.colored_label(ui.visuals().error_fg_color, text),
            false => ui.label(text),
        };
        if stale && !counts.is_running() {
            ui.weak(i18n.t("aggregate.counts.out_of_date"))
                .on_hover_text(i18n.t("aggregate.counts.out_of_date_hint"));
        }
    });
}

#[cfg(test)]
#[path = "aggregate_counts_tests.rs"]
mod tests;
