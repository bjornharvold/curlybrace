//! Tier 1 (local server): a compare of two real databases through the runtime. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use bson::{Document, doc};
use curlybrace_core::compare::key::KeySpec;
use curlybrace_core::compare::protocol::{CompareCommand, CompareEvent, PairOutcome, PairSpec};
use curlybrace_core::compare::results::Category;
use curlybrace_core::compare::rules::CompareRules;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use support::{local_enabled, local_runtime, next_compare};

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn two_real_databases_compare_end_to_end() {
    if !local_enabled("two_real_databases_compare_end_to_end") {
        return;
    }
    let (dbs, client, mut runtime, config) = local_runtime().await;
    let (a, b) = (dbs.ns("left"), dbs.ns("right"));
    let seed = |ns: &Namespace, docs: Vec<Document>| {
        let c = client
            .database(&ns.db)
            .collection::<Document>(&ns.collection);
        async move { c.insert_many(docs).await.unwrap() }
    };
    seed(
        &a,
        vec![
            doc! { "_id": 1, "v": 1 },
            doc! { "_id": 2, "v": 2 },
            doc! { "_id": 3, "v": 3 },
        ],
    )
    .await;
    seed(
        &b,
        vec![
            doc! { "_id": 2, "v": 2 },
            doc! { "_id": 3, "v": 30 },
            doc! { "_id": 4, "v": 4 },
        ],
    )
    .await;
    let open = CompareCommand::Open {
        session: 1,
        source: config.clone(),
        target: config,
    };
    runtime.tx.send(Command::Compare(open)).unwrap();
    assert_eq!(
        next_compare(&mut runtime).await,
        CompareEvent::Opened { session: 1 }
    );
    let key = KeySpec::id();
    let spec = PairSpec {
        id: 1,
        source: a,
        target: b,
        source_filter: doc! {},
        target_filter: doc! {},
        rules: CompareRules::for_key(&key),
        key,
    };
    let start = CompareCommand::Start {
        session: 1,
        run: 1,
        pairs: vec![spec],
    };
    runtime.tx.send(Command::Compare(start)).unwrap();
    let mut completed = false;
    loop {
        match next_compare(&mut runtime).await {
            CompareEvent::PairDone {
                outcome: PairOutcome::Completed(s),
                ..
            } => {
                assert_eq!(
                    (s.equal, s.only_in_source, s.only_in_target, s.different),
                    (1, 1, 1, 1)
                );
                completed = true;
            }
            CompareEvent::RunDone { .. } => break,
            CompareEvent::Progress { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    assert!(completed, "the pair completed before the run ended");
    check_rows_and_detail(&mut runtime).await;
    runtime
        .tx
        .send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
}

/// The one differing row is key 3; its detail re-reads both documents from the server.
async fn check_rows_and_detail(runtime: &mut curlybrace_core::runtime::RuntimeHandle) {
    let rows = CompareCommand::Rows {
        session: 1,
        pair: 1,
        category: Category::Different,
        offset: 0,
        limit: 10,
    };
    runtime.tx.send(Command::Compare(rows)).unwrap();
    let key = match next_compare(runtime).await {
        CompareEvent::Rows {
            total: 1, mut rows, ..
        } => rows.remove(0).key,
        other => panic!("{other:?}"),
    };
    let detail = CompareCommand::Detail {
        session: 1,
        pair: 1,
        key,
    };
    runtime.tx.send(Command::Compare(detail)).unwrap();
    match next_compare(runtime).await {
        CompareEvent::Detail {
            changes,
            source,
            target,
            ..
        } => {
            assert!(source.is_some() && target.is_some());
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].path, "v");
        }
        other => panic!("{other:?}"),
    }
}
