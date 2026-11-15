//! The Plan tab's tree: a summary line, the row of the input (the index used, or a collection
//! scan), one row per stage with its counters and time, the hints in words, and one collapsible
//! group per shard when the server answered per shard. Every text is an i18n key; counters the
//! server did not report are left out rather than shown as zero.
use crate::I18n;
use curlybrace_core::explain::{AggHint, AggPlan, AggStage, InputPlan};
use eframe::egui;

/// The figures of one row; `None` is not reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Figures {
    pub returned: Option<u64>,
    pub keys: Option<u64>,
    pub docs: Option<u64>,
    pub millis: Option<u64>,
}

impl Figures {
    fn of_stage(stage: &AggStage) -> Self {
        Self {
            returned: stage.n_returned,
            keys: stage.keys_examined,
            docs: stage.docs_examined,
            millis: stage.millis,
        }
    }

    fn of_input(input: &InputPlan) -> Self {
        Self {
            returned: input.n_returned,
            keys: input.keys_examined,
            docs: input.docs_examined,
            millis: None,
        }
    }
}

/// The reported figures in words, in a fixed order; empty when none was reported.
pub(super) fn figures_text(figures: Figures, i18n: &I18n) -> String {
    let part =
        |key: &str, value: Option<u64>| value.map(|n| i18n.t_with(key, &[("n", &n.to_string())]));
    let parts: Vec<String> = [
        part("aggregate.explain.figure.returned", figures.returned),
        part("aggregate.explain.figure.keys", figures.keys),
        part("aggregate.explain.figure.docs", figures.docs),
        part("aggregate.explain.figure.millis", figures.millis),
    ]
    .into_iter()
    .flatten()
    .collect();
    parts.join(&i18n.t("aggregate.explain.figure.separator"))
}

/// The input row: the index read, a collection scan, or the plan's own stage name.
pub(super) fn input_text(input: &InputPlan, i18n: &I18n) -> String {
    match (&input.index, input.stage_kind.as_str()) {
        (Some(index), _) => i18n.t_with("aggregate.explain.input.index", &[("index", index)]),
        (None, "COLLSCAN") => i18n.t("aggregate.explain.input.collscan"),
        (None, kind) => i18n.t_with("aggregate.explain.input.other", &[("kind", kind)]),
    }
}

/// The stage's name, with its derived detail (field names) when it has one.
pub(super) fn stage_title(stage: &AggStage, i18n: &I18n) -> String {
    match &stage.detail {
        Some(detail) => i18n.t_with(
            "aggregate.explain.stage_with_detail",
            &[("name", &stage.name), ("detail", detail)],
        ),
        None => stage.name.clone(),
    }
}

/// The plan's summary line: its totals in words, or that nothing was run.
pub(super) fn summary_text(plan: &AggPlan, i18n: &I18n) -> String {
    let summary = &plan.summary;
    let figures = figures_text(
        Figures {
            returned: summary.n_returned,
            keys: summary.keys_examined,
            docs: summary.docs_examined,
            millis: summary.total_millis,
        },
        i18n,
    );
    match figures.is_empty() {
        true => i18n.t("aggregate.explain.summary_none"),
        false => i18n.t_with("aggregate.explain.summary", &[("figures", &figures)]),
    }
}

/// Draws `plan`; a sharded plan is one collapsible group per shard (open by default).
pub(super) fn show_plan(ui: &mut egui::Ui, plan: &AggPlan, i18n: &I18n) {
    ui.label(summary_text(plan, i18n));
    for (name, shard) in &plan.shards {
        let title = i18n.t_with("aggregate.explain.group.shard", &[("name", name)]);
        egui::CollapsingHeader::new(title)
            .id_salt(("explain_shard", name))
            .default_open(true)
            .show(ui, |ui| show_body(ui, shard, false, i18n));
    }
    show_body(ui, plan, true, i18n);
}

/// The input, the stages and, with `hints`, the hints of one plan (a shard's, or the whole: a
/// sharded plan names its hints once, after the shards).
fn show_body(ui: &mut egui::Ui, plan: &AggPlan, hints: bool, i18n: &I18n) {
    if let Some(input) = &plan.input {
        show_input(ui, input, i18n);
    }
    for stage in &plan.stages {
        show_row(
            ui,
            &stage_title(stage, i18n),
            &figures_text(Figures::of_stage(stage), i18n),
        );
    }
    if hints {
        show_hints(ui, plan, i18n);
    }
}

fn show_input(ui: &mut egui::Ui, input: &InputPlan, i18n: &I18n) {
    show_row(
        ui,
        &input_text(input, i18n),
        &figures_text(Figures::of_input(input), i18n),
    );
    if input.plan_stages.len() > 1 {
        let stages = input.plan_stages.join(", ");
        ui.weak(i18n.t_with("aggregate.explain.input.plan", &[("stages", &stages)]));
    }
}

/// One row: the title, then its figures in a weaker colour. A plan stage's own sub-plan is a
/// shard's, shown once by `show_plan`; nothing here nests.
fn show_row(ui: &mut egui::Ui, title: &str, figures: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(title);
        if !figures.is_empty() {
            ui.weak(figures);
        }
    });
}

/// The i18n key of a hint's sentence. Listed one by one (not built from `AggHint::key`), so a new
/// hint in core is a compile error here until it has its sentence.
pub(super) fn hint_key(hint: AggHint) -> &'static str {
    match hint {
        AggHint::CollectionScan => "aggregate.explain.hint.collection_scan",
        AggHint::InMemorySort => "aggregate.explain.hint.in_memory_sort",
        AggHint::DiskSort => "aggregate.explain.hint.disk_sort",
        AggHint::ExaminedMuchMoreThanReturned => {
            "aggregate.explain.hint.examined_much_more_than_returned"
        }
        AggHint::LookupCollectionScan => "aggregate.explain.hint.lookup_collection_scan",
    }
}

fn show_hints(ui: &mut egui::Ui, plan: &AggPlan, i18n: &I18n) {
    if plan.hints.is_empty() {
        return;
    }
    ui.add_space(ui.spacing().item_spacing.y);
    ui.strong(i18n.t("aggregate.explain.hints.heading"));
    for hint in &plan.hints {
        ui.label(i18n.t(hint_key(*hint)));
    }
}

#[cfg(test)]
#[path = "aggregate_plan_tests.rs"]
mod tests;
