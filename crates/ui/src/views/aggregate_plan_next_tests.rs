//! The Plan tab's follow-ups: named controls, the index list asked for once, off controls with
//! their reasons, Copy command on the clipboard, Create index... off on a read-only connection.
use super::*;
use crate::theme;
use crate::viewmodel::aggregate_explain_block::explain_request;
use crate::viewmodel::aggregate_hint_list::HintListUi;
use bson::{Document, doc};
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::pipeline_options::{HintOption, PipelineOptions};
use curlybrace_core::store::{Explain, Namespace};
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    draft: AggregateState,
    context: RunContext,
    seen: Vec<AggregateAction>,
    copied: Vec<String>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn fixture(name: &str) -> Document {
    let text = match name {
        "collscan" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_collscan.json")
        }
        _ => include_str!("../../../core/tests/fixtures/explain_aggregate/stages_ixscan.json"),
    };
    editor_json_to_doc(text).expect("fixture parses")
}

/// A draft with something to explain: two stages, the selected one the last.
fn base() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

fn pipeline() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$sort": { "total": -1_i32 } },
    ]
}

/// A draft whose Plan tab holds the plan of `fixture`; the index list is `list`.
fn explained(name: &str, options: PipelineOptions, list: HintListUi) -> AggregateState {
    let draft = base();
    let (ui, request, _) = draft.explain.clone().begun(pipeline(), &options);
    let ui = ui.arrived(
        request,
        Ok(Explain {
            document: fixture(name),
            plan_only: true,
        }),
    );
    let mut draft = draft;
    draft.explain = ui;
    draft.options = options;
    draft.options_ui.hint = list;
    draft
}

fn names() -> HintListUi {
    HintListUi::default().from_names(vec!["status_1".into(), "total_-1".into()])
}

fn context(read_only: bool) -> RunContext {
    RunContext {
        read_only,
        ns: Some(Namespace::new("shop", "orders")),
        ..RunContext::default()
    }
}

fn harness(draft: AggregateState, context: RunContext) -> Harness<'static, Frame> {
    let frame = Frame {
        draft,
        context,
        seen: Vec::new(),
        copied: Vec::new(),
    };
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 500.0))
        .build_ui_state(
            move |ui, f: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_plan_next(
                    ui,
                    &f.draft,
                    (&explain_request(&f.draft, &f.draft.stages), &f.context),
                    &I18n::english(),
                );
                let copied = ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| match c {
                            egui::OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                f.copied.extend(copied);
                f.seen.extend(actions);
            },
            frame,
        );
    h.run_steps(3);
    h
}

fn scan() -> AggregateState {
    explained("collscan", PipelineOptions::default(), names())
}

#[test]
fn a_collection_scan_plan_shows_try_an_index_and_the_suggestion() {
    let h = harness(scan(), context(false));
    h.get_by_role_and_label(Role::ComboBox, &t("aggregate.options.explain.try_index"));
    h.get_by_label(&t("aggregate.options.explain.suggested"));
    h.get_by_role_and_label(
        Role::Button,
        &t("aggregate.options.explain.copy_create_index"),
    );
    h.get_by_role_and_label(Role::Button, &t("aggregate.options.explain.create_index"));
    h.get_by_label_contains("createIndex({ \"status\": 1, \"total\": -1 })");
    assert!(
        h.query_by_label(&t("aggregate.options.explain.clear_hint"))
            .is_none(),
        "no hint, nothing to clear"
    );
}

#[test]
fn an_index_plan_without_a_hint_shows_nothing() {
    let h = harness(
        explained("ixscan", PipelineOptions::default(), names()),
        context(false),
    );
    assert!(
        h.query_by_label(&t("aggregate.options.explain.try_index"))
            .is_none()
    );
    assert!(
        h.query_by_label(&t("aggregate.options.explain.suggested"))
            .is_none()
    );
}

#[test]
fn picking_an_index_emits_try_index() {
    let mut h = harness(scan(), context(false));
    h.get_by_role_and_label(Role::ComboBox, &t("aggregate.options.explain.try_index"))
        .click();
    h.run_steps(3);
    h.get_by_label("total_-1").click();
    h.run_steps(3);
    assert_eq!(
        h.state().seen,
        [AggregateAction::Explain(ExplainAction::TryIndex(
            "total_-1".into()
        ))]
    );
}

#[test]
fn clear_hint_shows_only_with_a_hint_and_emits_clear_hint() {
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        ..PipelineOptions::default()
    };
    let mut h = harness(explained("collscan", hinted, names()), context(false));
    h.get_by_role_and_label(Role::Button, &t("aggregate.options.explain.clear_hint"))
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().seen,
        [AggregateAction::Explain(ExplainAction::ClearHint)]
    );
}

#[test]
fn the_picker_asks_for_the_index_list_once() {
    let asking = harness(
        explained(
            "collscan",
            PipelineOptions::default(),
            HintListUi::default(),
        ),
        context(false),
    );
    let wanted = AggregateAction::Options(OptionsAction::HintListWanted);
    assert!(asking.state().seen.contains(&wanted));
    let loaded = harness(scan(), context(false));
    assert!(!loaded.state().seen.contains(&wanted));
}

#[test]
fn loading_and_failed_lists_say_so() {
    let loading = harness(
        explained(
            "collscan",
            PipelineOptions::default(),
            HintListUi::default(),
        ),
        context(false),
    );
    loading.get_by_label(&t("aggregate.options.hint.loading"));
    let (waiting, request) = HintListUi::default().requested();
    let failed = waiting.failed(request);
    let h = harness(
        explained("collscan", PipelineOptions::default(), failed),
        context(false),
    );
    h.get_by_label(&t("aggregate.options.hint.list_failed"));
}

#[test]
fn a_failed_list_offers_a_retry_that_asks_again() {
    let (waiting, request) = HintListUi::default().requested();
    let failed = waiting.failed(request);
    let mut h = harness(
        explained("collscan", PipelineOptions::default(), failed),
        context(false),
    );
    h.get_by_role_and_label(Role::Button, &t("common.retry"))
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().seen,
        [AggregateAction::Options(OptionsAction::HintListRetry)]
    );
}

#[test]
fn try_is_off_with_the_reason_while_an_explain_runs() {
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        ..PipelineOptions::default()
    };
    let done = explained("collscan", hinted.clone(), names());
    // The next explain is out: the plan is replaced by the wait, the hint stays clearable (off).
    let (running, _, _) = done.explain.clone().begun(pipeline(), &hinted);
    let mut draft = done;
    draft.explain = running;
    let mut h = harness(draft, context(false));
    let reason = t("aggregate.explain.disabled.running");
    let label = t("aggregate.options.explain.clear_hint");
    let clear = h.get_by_role_and_label(Role::Button, &label);
    assert!(clear.accesskit_node().is_disabled());
    assert_eq!(clear.accesskit_node().description(), Some(reason));
    clear.click();
    h.run_steps(2);
    assert!(h.state().seen.is_empty());
}

#[test]
fn clear_hint_is_off_with_the_reason_while_disconnected() {
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        ..PipelineOptions::default()
    };
    let lost = RunContext {
        disconnected: true,
        ..context(false)
    };
    let mut h = harness(explained("collscan", hinted, names()), lost);
    let label = t("aggregate.options.explain.clear_hint");
    let clear = h.get_by_role_and_label(Role::Button, &label);
    assert!(clear.accesskit_node().is_disabled());
    assert_eq!(
        clear.accesskit_node().description(),
        Some(t("aggregate.explain.disabled.no_connection"))
    );
    clear.click();
    h.run_steps(2);
    assert!(h.state().seen.is_empty());
}

#[test]
fn copy_puts_the_command_on_the_clipboard() {
    let mut h = harness(scan(), context(false));
    h.get_by_role_and_label(
        Role::Button,
        &t("aggregate.options.explain.copy_create_index"),
    )
    .click();
    h.run_steps(3);
    let expected = "db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1, \"total\": -1 })";
    assert_eq!(h.state().copied, [expected]);
    assert_eq!(
        h.state().seen,
        [AggregateAction::Explain(ExplainAction::CopyCreateIndex)]
    );
}

#[test]
fn create_index_is_off_with_the_reason_on_a_read_only_connection() {
    let mut h = harness(scan(), context(true));
    let label = t("aggregate.options.explain.create_index");
    let create = h.get_by_role_and_label(Role::Button, &label);
    assert!(create.accesskit_node().is_disabled());
    assert_eq!(
        create.accesskit_node().description(),
        Some(t("aggregate.options.explain.create_index_read_only"))
    );
    create.click();
    h.run_steps(2);
    assert!(h.state().seen.is_empty());
}

#[test]
fn create_index_emits_create_index() {
    let mut h = harness(scan(), context(false));
    h.get_by_role_and_label(Role::Button, &t("aggregate.options.explain.create_index"))
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().seen,
        [AggregateAction::Explain(ExplainAction::CreateIndex)]
    );
}
