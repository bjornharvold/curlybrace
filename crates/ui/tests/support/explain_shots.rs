//! A canned explain for the Plan tab's screenshots and layout tests: the pipeline of
//! [`super::aggregate_shots::three_stage_draft`] up to its selected `$group`, planned as a
//! collection scan with measured figures, so the tree shows an input row, a stage row, hints and
//! the cost note.
use super::aggregate_shots::three_stage_draft;
use super::shots::browsing;
use bson::Document;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::explain::ExplainVerbosity;
use curlybrace_core::pipeline::{PipelineOptions, explain_pipeline};
use curlybrace_core::store::Explain;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::aggregate_explain::ResultTab;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;
use curlybrace_ui::viewmodel::{AggregateState, CollectionTab, ViewMode};

/// The `executionStats` explain of `$match` + `$group` on `crm.customers`, as a server of the old
/// `stages` shape would send it.
pub fn explain_document() -> Document {
    editor_json_to_doc(
        r#"{
  "explainVersion": "1",
  "stages": [
    {
      "$cursor": {
        "queryPlanner": {
          "namespace": "crm.customers",
          "winningPlan": {
            "stage": "COLLSCAN",
            "filter": { "status": { "$eq": "active" } },
            "direction": "forward"
          },
          "rejectedPlans": []
        },
        "executionStats": {
          "executionSuccess": true,
          "nReturned": 8400,
          "executionTimeMillis": 63,
          "totalKeysExamined": 0,
          "totalDocsExamined": 120000,
          "executionStages": {
            "stage": "COLLSCAN",
            "nReturned": 8400,
            "executionTimeMillisEstimate": 41,
            "docsExamined": 120000
          }
        }
      },
      "nReturned": 8400,
      "executionTimeMillisEstimate": 58
    },
    {
      "$group": { "_id": "$city", "customers": { "$sum": 1 } },
      "maxAccumulatorMemoryUsageBytes": { "customers": 2400 },
      "totalOutputDataSizeBytes": 4100,
      "usedDisk": false,
      "spills": 0,
      "nReturned": 4,
      "executionTimeMillisEstimate": 63
    }
  ],
  "ok": 1
}"#,
    )
    .expect("the canned explain parses")
}

/// [`three_stage_draft`] with its explain answered and the Plan tab in view, measured.
pub fn explained_draft() -> AggregateState {
    // The collection's indexes are known, as after the Options section listed them.
    let mut draft = three_stage_draft()
        .hint_edited(|list| list.from_names(vec!["status_1".into(), "city_1".into()]));
    let pipeline = explain_pipeline(&draft.stages, draft.selected).expect("valid pipeline");
    let (ui, request, _) = draft
        .explain
        .clone()
        .with_verbosity(ExplainVerbosity::ExecutionStats)
        .begun(pipeline, &PipelineOptions::default());
    draft.explain = ui
        .arrived(
            request,
            Ok(Explain {
                document: explain_document(),
                plan_only: false,
            }),
        )
        .with_tab(ResultTab::Plan);
    draft
}

/// The Aggregate tab of a connected browser on `crm.customers` showing [`explained_draft`].
pub fn aggregate_plan_tab(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    state.tab = CollectionTab::Aggregate;
    state.aggregate = Box::new(explained_draft());
}

/// A plan that sorts in memory after a collection scan, as a server without an index on the
/// filter and sort fields would answer (plan only).
fn sort_document() -> Document {
    editor_json_to_doc(
        r#"{
  "explainVersion": "1",
  "queryPlanner": {
    "namespace": "crm.customers",
    "winningPlan": {
      "stage": "SORT",
      "sortPattern": { "total": -1 },
      "inputStage": {
        "stage": "COLLSCAN",
        "filter": { "status": { "$eq": "A" } },
        "direction": "forward"
      }
    },
    "rejectedPlans": []
  },
  "ok": 1
}"#,
    )
    .expect("the canned explain parses")
}

/// `$match` + `$sort` explained as a scan with an in-memory sort, with the hint `status_1` set
/// and the collection's indexes listed: the Plan tab shows Try an index, Clear hint and the
/// suggested index.
pub fn plan_next_draft() -> AggregateState {
    let draft = AggregateState::default()
        .add_stage("$match")
        .edit(0, r#"{ "$match": { "status": "A" } }"#)
        .add_stage("$sort")
        .edit(1, r#"{ "$sort": { "total": -1 } }"#)
        .options_edited(OptionsEdit::HintPicked("status_1".into()))
        .options_applied()
        .0
        .hint_edited(|list| list.from_names(vec!["status_1".into(), "total_-1".into()]));
    let options = draft.options.clone();
    let pipeline = explain_pipeline(&draft.stages, draft.selected).expect("valid pipeline");
    let (ui, request, _) = draft.explain.clone().begun(pipeline, &options);
    let ui = ui
        .arrived(
            request,
            Ok(Explain {
                document: sort_document(),
                plan_only: true,
            }),
        )
        .with_tab(ResultTab::Plan);
    let mut draft = draft;
    draft.explain = ui;
    draft
}

/// The Aggregate tab of a connected browser on `crm.customers` showing [`plan_next_draft`].
pub fn aggregate_plan_next_tab(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    state.tab = CollectionTab::Aggregate;
    state.aggregate = Box::new(plan_next_draft());
}
