use bson::doc;

use crate::error::CoreError;
use crate::indexes::IndexSpec;
use crate::pipeline_options::{HintOption, PipelineOptions};
use crate::query_options::FindOperation;
use crate::store::{DocumentStore, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_index_spec, mock_namespace};

fn hinted(hint: HintOption) -> Query {
    Query {
        options: PipelineOptions {
            hint: Some(hint),
            ..PipelineOptions::default()
        },
        ..Query::default()
    }
}

fn scripted() -> InMemoryStore {
    InMemoryStore::new().with_explain(&mock_namespace(), doc! { "queryPlanner": {} })
}

async fn with_index(store: &InMemoryStore, spec: &IndexSpec) {
    store.create_index(&mock_namespace(), spec).await.unwrap();
}

#[tokio::test]
async fn the_store_records_what_each_find_received() {
    let store = InMemoryStore::new();
    let first = Query::default();
    let second = Query {
        skip: 5,
        options: PipelineOptions {
            comment: Some("why".into()),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    store.find(&mock_namespace(), &first).await.unwrap();
    store.find(&mock_namespace(), &second).await.unwrap();
    assert_eq!(
        store.received_queries(),
        vec![(FindOperation::Find, first), (FindOperation::Find, second)]
    );
}

#[tokio::test]
async fn a_hint_naming_no_index_is_hint_not_found() {
    let store = scripted();
    let q = hinted(HintOption::Name("missing_1".into()));
    let ns = mock_namespace();
    assert_eq!(store.find(&ns, &q).await, Err(CoreError::HintNotFound));
    assert_eq!(
        store.explain_find(&ns, &q).await.unwrap_err(),
        CoreError::HintNotFound
    );
    assert_eq!(
        store.explain_plan(&ns, &q).await.unwrap_err(),
        CoreError::HintNotFound
    );
}

#[tokio::test]
async fn a_listed_hint_and_a_key_hint_pass() {
    let store = scripted();
    let spec = mock_index_spec();
    with_index(&store, &spec).await;
    let ns = mock_namespace();
    let by_name = hinted(HintOption::Name(spec.name.clone()));
    assert!(store.find(&ns, &by_name).await.is_ok());
    let by_id = hinted(HintOption::Name("_id_".into()));
    assert!(store.find(&ns, &by_id).await.is_ok());
    let by_keys = hinted(HintOption::Keys(doc! { "nothing": 1 }));
    assert!(store.find(&ns, &by_keys).await.is_ok());
}

#[tokio::test]
async fn explain_and_plan_record_their_operation() {
    let store = scripted();
    let ns = mock_namespace();
    let q = Query::default();
    store.explain_find(&ns, &q).await.unwrap();
    let plan = store.explain_plan(&ns, &q).await.unwrap();
    assert!(plan.plan_only);
    let ops: Vec<FindOperation> = store.received_queries().iter().map(|(op, _)| *op).collect();
    assert_eq!(ops, vec![FindOperation::Explain, FindOperation::PlanCheck]);
}

#[tokio::test]
async fn a_refused_hint_is_still_recorded_as_received() {
    let store = scripted();
    let q = hinted(HintOption::Name("missing_1".into()));
    let _ = store.find(&mock_namespace(), &q).await;
    assert_eq!(store.received_queries().len(), 1);
}

#[tokio::test]
async fn a_scripted_find_error_answers_every_find() {
    let store = InMemoryStore::new().with_find_error(CoreError::ReadPreferenceUnavailable);
    assert_eq!(
        store.find(&mock_namespace(), &Query::default()).await,
        Err(CoreError::ReadPreferenceUnavailable)
    );
}
