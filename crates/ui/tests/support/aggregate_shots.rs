//! Fixed states for the Aggregate tab screenshots and layout tests: a three-stage pipeline on
//! `crm.customers` (`$match`, `$group`, `$sort`) with stage 2 selected and its preview shown.
use super::shots::browsing;
use bson::{Document, doc};
use curlybrace_core::pipeline::RunTarget;
use curlybrace_core::saved_pipeline::{SavedPipeline, SavedStage};
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate_run::RunDialog;
use curlybrace_ui::viewmodel::{CollectionTab, ViewMode};
use uuid::Uuid;

/// The stage texts, as the person typed them (JSON data, not labels).
const MATCH: &str = "{\n  \"$match\": { \"status\": \"active\" }\n}";
const GROUP: &str = "{\n  \"$group\": { \"_id\": \"$city\", \"customers\": { \"$sum\": 1 } }\n}";
const SORT: &str = "{\n  \"$sort\": { \"customers\": -1 }\n}";

/// What `$group` returns for the sample: one document per city.
pub fn grouped_docs() -> Vec<Document> {
    vec![
        doc! { "_id": "Berlin", "customers": 12_i32 },
        doc! { "_id": "Lisbon", "customers": 9_i32 },
        doc! { "_id": "Oslo", "customers": 4_i32 },
        doc! { "_id": "Austin", "customers": 2_i32 },
    ]
}

/// `$match`, `$group`, `$sort` with stage 2 selected and its four documents shown.
pub fn three_stage_draft() -> AggregateState {
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$group")
        .add_stage("$sort")
        .edit(0, MATCH)
        .edit(1, GROUP)
        .edit(2, SORT)
        .select(1);
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("the fixture pipeline is valid");
    draft.arrived(request, Ok(grouped_docs()))
}

/// The Aggregate tab of a connected browser on `crm.customers` showing [`three_stage_draft`].
pub fn aggregate_tab(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    state.tab = CollectionTab::Aggregate;
    state.aggregate = Box::new(three_stage_draft());
}

/// The name the opened-pipeline fixture was saved under.
pub const OPENED_NAME: &str = "Customers per city";

/// [`three_stage_draft`] as a saved pipeline, so a draft can be opened from it.
pub fn saved_three_stages() -> SavedPipeline {
    let stages = three_stage_draft()
        .stages
        .iter()
        .map(|s| SavedStage {
            text: s.text.clone(),
            enabled: s.enabled,
        })
        .collect();
    SavedPipeline {
        id: Uuid::from_u128(7),
        name: OPENED_NAME.into(),
        ns_hint: Some("crm.customers".into()),
        saved_unix: 1_700_000_000,
        stages,
    }
}

/// [`three_stage_draft`] opened from its saved pipeline and then edited (the `$sort` direction),
/// so the header shows the name and the Modified mark; stage 3 previewed.
pub fn opened_modified_draft() -> AggregateState {
    let draft = AggregateState::default()
        .opened_from(&saved_three_stages())
        .edit(2, "{\n  \"$sort\": { \"customers\": 1 }\n}");
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("the fixture pipeline is valid");
    draft.arrived(request, Ok(grouped_docs()))
}

/// The Aggregate tab of a connected browser on `crm.customers` showing [`opened_modified_draft`].
pub fn aggregate_opened_tab(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    state.tab = CollectionTab::Aggregate;
    state.aggregate = Box::new(opened_modified_draft());
}

/// `$match`, `$group`, `$out` (to `city_totals`) with the confirmation dialog open on a
/// production connection, the target's count known and the name half typed.
pub fn aggregate_run_dialog_tab(state: &mut AppState) {
    super::production_shots::production_root(state);
    state.tab = CollectionTab::Aggregate;
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$group")
        .add_stage("$out")
        .edit(0, MATCH)
        .edit(1, GROUP)
        .edit(2, "{\n  \"$out\": \"city_totals\"\n}")
        .select(1);
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("the fixture pipeline is valid");
    let dialog = RunDialog {
        target: RunTarget {
            db: None,
            collection: "city_totals".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("crm", "customers"),
        typed: "city_tot".into(),
        target_count: Some(1_204),
    };
    let draft = draft.arrived(request, Ok(grouped_docs()));
    let run = draft.run.clone().opened(dialog);
    state.aggregate = Box::new(draft.with_run(run));
}
