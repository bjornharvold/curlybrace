use super::*;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_counts_plan::count_plan;
use curlybrace_core::error::CoreError;

fn text(count: StageCount) -> (String, bool) {
    count_text(&count, &I18n::english())
}

#[test]
fn counts_are_worded_in_singular_plural_zero_and_capped() {
    assert_eq!(text(StageCount::Exact(1)), ("1 document".into(), false));
    assert_eq!(text(StageCount::Exact(0)), ("0 documents".into(), false));
    assert_eq!(text(StageCount::Exact(42)), ("42 documents".into(), false));
    assert_eq!(
        text(StageCount::AtLeast(100_000)),
        ("at least 100000 documents".into(), false)
    );
}

#[test]
fn errors_and_timeouts_are_worded_as_errors() {
    let (failed, error) = text(StageCount::Failed(CoreError::Unreachable));
    assert!(error);
    assert!(failed.starts_with("Could not count: "), "{failed}");
    let (timed_out, error) = text(StageCount::TimedOut);
    assert!(error);
    assert_eq!(timed_out, "Timed out: the server stopped counting.");
}

#[test]
fn every_reason_a_stage_is_not_counted_has_its_own_words() {
    let reasons = [
        NotCounted::Invalid,
        NotCounted::AfterInvalid,
        NotCounted::WriteStage,
        NotCounted::NestedWrite,
    ];
    let words: Vec<String> = reasons
        .into_iter()
        .map(|r| text(StageCount::NotCounted(r)).0)
        .collect();
    assert!(words.iter().all(|w| w.starts_with("Not counted: ")));
    let mut unique = words.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), words.len());
}

fn draft() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

#[test]
fn the_button_is_off_with_the_previews_reason_when_nothing_can_be_counted() {
    let i18n = I18n::english();
    let view = AggregateView::default();
    let none = AggregateState::default();
    assert_eq!(
        counts_blocked(&none, &none.stages, &view, &i18n),
        Some(i18n.t("aggregate.error.empty"))
    );
    let broken = draft().edit(0, "{ nope").edit(1, "{ nope");
    assert_eq!(
        counts_blocked(&broken, &broken.stages, &view, &i18n),
        Some(i18n.t("aggregate.counts.disabled.nothing"))
    );
    let text = draft().to_text().with_text("[]");
    assert_eq!(
        counts_blocked(&text, &text.stages, &view, &i18n),
        Some(i18n.t("aggregate.counts.disabled.text_not_applied"))
    );
    let applied = draft().to_text();
    assert_eq!(
        counts_blocked(&applied, &applied.stages, &view, &i18n),
        Some(i18n.t("aggregate.counts.disabled.text_mode"))
    );
    let ok = draft();
    assert_eq!(counts_blocked(&ok, &ok.stages, &view, &i18n), None);
}

#[test]
fn a_card_says_counting_or_waiting_while_the_sequence_runs_then_its_result() {
    let i18n = I18n::english();
    let d = draft();
    let plan = count_plan(&d, &d.stages).expect("a plan");
    let (ui, first) = CountsUi::default().begun(plan, &d.stages, &d.keys);
    let first = first.expect("sent");
    let line = |ui: &CountsUi, i: usize| line_text(ui, (i, d.keys[i]), &i18n).map(|(t, _)| t);
    assert_eq!(line(&ui, 0), Some("Counting...".into()));
    assert_eq!(line(&ui, 1), Some("Waiting".into()));
    let (ui, _) = ui.answered(first.request, 0, Ok(3));
    assert_eq!(line(&ui, 0), Some("3 documents".into()));
    assert_eq!(line(&ui, 1), Some("Counting...".into()));
}
