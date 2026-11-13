//! Fixed states for the Aggregate tab screenshots and layout tests: a three-stage pipeline on
//! `crm.customers` (`$match`, `$group`, `$sort`) with stage 2 selected and its preview shown.
use super::shots::browsing;
use bson::{Document, doc};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::{CollectionTab, ViewMode};

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
