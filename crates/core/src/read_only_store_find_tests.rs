//! Query options are reads: a locked store passes them through the find and both explains.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bson::doc;

use crate::pipeline_options::{HintOption, PipelineOptions, ReadPreferenceMode};
use crate::query_options::FindOperation;
use crate::read_only_store::ReadOnlyStore;
use crate::store::Query;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

#[tokio::test]
async fn find_options_pass_through_a_read_only_store() {
    let ns = mock_namespace();
    let inner = Arc::new(InMemoryStore::new().with_explain(&ns, doc! { "queryPlanner": {} }));
    let locked = ReadOnlyStore::wrap(inner.clone(), Arc::new(AtomicBool::new(true)));
    let query = Query {
        options: PipelineOptions {
            hint: Some(HintOption::Keys(doc! { "a": 1 })),
            comment: Some("why".into()),
            read_preference: Some(ReadPreferenceMode::Nearest),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    locked.find(&ns, &query).await.expect("a find is a read");
    locked
        .explain_find(&ns, &query)
        .await
        .expect("so is an explain");
    locked
        .explain_plan(&ns, &query)
        .await
        .expect("and a plan check");
    let received = inner.received_queries();
    let ops: Vec<FindOperation> = received.iter().map(|(op, _)| *op).collect();
    assert_eq!(
        ops,
        vec![
            FindOperation::Find,
            FindOperation::Explain,
            FindOperation::PlanCheck
        ]
    );
    assert!(received.iter().all(|(_, q)| *q == query));
}
