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
    let mut draft = three_stage_draft();
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
