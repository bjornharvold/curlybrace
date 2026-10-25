//! Copy between two REAL servers, through the runtime with two open sessions. Each test owns two
//! `cb-it-*` containers (removed by their guards even when the test fails). The same-server cases
//! are in `docker_copy_same_server.rs`: tier 2 as well, because no local mongod runs where these
//! are run. Run with
//! `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test docker_copy_between_servers -- --ignored`.
//! Document values and `_id`s are never put in a message beyond what an assertion needs.

mod support;

use bson::{Bson, Document, doc, oid::ObjectId};
use curlybrace_core::copy::{ConflictPolicy, CopyEvent, CopyOutcome, CopyScope};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, WriteFailure};
use support::copy_flow::{
    all_docs, by_id, cancel, collect, connect, count, db_names, has_planned, id_v, item, next_copy,
    numbered_range, reports, request, run_copy, runtime, seed, start,
};
use support::{config, docker_enabled, docker_mongo};

/// Runs one copy of `ns` (same names on both sides) from session `a` to session `b`.
macro_rules! copy_ns {
    ($rt:expr, $run:expr, $a:expr, $b:expr, $ns:expr, $policy:expr) => {{
        let scope = CopyScope::Collections(vec![item($ns, $ns)]);
        run_copy($rt, $run, request($a, $b, scope, $policy)).await
    }};
}

fn assert_done(events: &[CopyEvent]) {
    assert!(
        matches!(
            events.last(),
            Some(CopyEvent::Done {
                cancelled: false,
                ..
            })
        ),
        "the run ends with Done"
    );
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn copies_between_two_containers_with_skip_overwrite_and_upsert() {
    if !docker_enabled("copies_between_two_containers_with_skip_overwrite_and_upsert") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    // Source holds 1..=6 ("src"); each target collection holds 4..=9 ("tgt"): 4..=6 collide.
    let cases = [
        (ConflictPolicy::Skip, "skip", 3),
        (ConflictPolicy::Overwrite, "overwrite", 3),
        (ConflictPolicy::Upsert, "upsert", 6),
    ];
    for (run, (policy, name, written)) in cases.into_iter().enumerate() {
        let ns = Namespace::new("cbit_policies", name);
        seed(&client_a, &ns, numbered_range(1, 6, "src")).await;
        seed(&client_b, &ns, numbered_range(4, 9, "tgt")).await;
        let events = copy_ns!(&mut rt, run as u64 + 1, sa, sb, &ns, policy);
        assert_done(&events);
        let report = &reports(&events)[0];
        assert_eq!(report.outcome, CopyOutcome::Completed, "{name}");
        assert_eq!(report.progress.scanned, 6, "{name}");
        assert_eq!(report.progress.written, written, "{name}");
        assert_eq!(report.progress.failed, 0, "{name}");

        let got = id_v(&all_docs(&client_b, &ns).await);
        let v = |id: i32| got.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_str());
        // Nothing is ever deleted: 7..=9 are always still there, untouched.
        assert!(
            (7..=9).all(|id| v(id) == Some("tgt")),
            "{name}: target-only kept"
        );
        match policy {
            ConflictPolicy::Skip => {
                assert_eq!(got.len(), 9);
                assert!((1..=3).all(|id| v(id) == Some("src")));
                assert!(
                    (4..=6).all(|id| v(id) == Some("tgt")),
                    "skip keeps collisions"
                );
            }
            ConflictPolicy::Overwrite => {
                assert_eq!(got.len(), 6, "overwrite never inserts");
                assert!((1..=3).all(|id| v(id).is_none()));
                assert!((4..=6).all(|id| v(id) == Some("src")));
            }
            ConflictPolicy::Upsert => {
                assert_eq!(got.len(), 9);
                assert!((1..=6).all(|id| v(id) == Some("src")));
            }
        }
        assert_eq!(count(&client_a, &ns).await, 6, "{name}: source untouched");
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn read_only_target_container_is_refused_and_untouched() {
    if !docker_enabled("read_only_target_container_is_refused_and_untouched") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let ns = Namespace::new("cbit_ro", "c");
    seed(&client_a, &ns, numbered_range(1, 5, "src")).await;
    let dbs_before = db_names(&client_b).await;

    let mut rt = runtime();
    let sa = connect(&mut rt, &config(&a)).await;
    let mut cfg_b = config(&b);
    cfg_b.read_only = true;
    let sb = connect(&mut rt, &cfg_b).await;
    let events = copy_ns!(&mut rt, 1, sa, sb, &ns, ConflictPolicy::Upsert);

    assert!(!has_planned(&events), "refused before planning");
    assert!(
        matches!(
            events.as_slice(),
            [CopyEvent::Failed {
                error: CoreError::ReadOnly,
                ..
            }]
        ),
        "a single Failed(ReadOnly)"
    );
    assert_eq!(
        db_names(&client_b).await,
        dbs_before,
        "no database appeared"
    );
    assert_eq!(count(&client_b, &ns).await, 0, "no document, no collection");
    assert!(
        support::copy_flow::collection_names(&client_b, "cbit_ro")
            .await
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn cancel_mid_copy_leaves_target_with_exactly_the_reported_documents() {
    if !docker_enabled("cancel_mid_copy_leaves_target_with_exactly_the_reported_documents") {
        return;
    }
    const TOTAL: i32 = 20_000;
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let ns = Namespace::new("cbit_cancel", "big");
    for chunk in 0..(TOTAL / 5_000) {
        seed(
            &client_a,
            &ns,
            numbered_range(chunk * 5_000 + 1, (chunk + 1) * 5_000, "x"),
        )
        .await;
    }
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    let scope = CopyScope::Collections(vec![item(&ns, &ns)]);
    start(&rt, 1, request(sa, sb, scope, ConflictPolicy::Skip));

    // Cancel on the very first Progress (one batch of 500 of 20,000 is written by then).
    loop {
        match next_copy(&mut rt).await {
            CopyEvent::Progress { .. } => break,
            CopyEvent::Planned { .. } => {}
            other => panic!("unexpected event before the first Progress: {other:?}"),
        }
    }
    cancel(&rt, 1);
    let events = collect(&mut rt).await;

    let report = &reports(&events)[0];
    let actual = count(&client_b, &ns).await;
    assert_eq!(report.progress.written, actual, "reported equals actual");
    assert!(actual > 0 && actual <= TOTAL as u64);
    match (&report.outcome, events.last()) {
        (
            CopyOutcome::Cancelled,
            Some(CopyEvent::Done {
                cancelled: true, ..
            }),
        ) => {
            assert!(actual < TOTAL as u64, "a cancelled copy did not finish");
        }
        (
            CopyOutcome::Completed,
            Some(CopyEvent::Done {
                cancelled: false, ..
            }),
        ) => {
            assert_eq!(actual, TOTAL as u64, "a finished copy wrote everything");
        }
        other => panic!("inconsistent end of run: {other:?}"),
    }
    eprintln!(
        "cancel test: {actual} of {TOTAL} written, outcome {:?}",
        report.outcome
    );
}

/// `numeric_id_type_mismatch_is_reported_per_document_or_replaced`.
/// Observed on `mongo:7` (image mongo:7, mongod 7.0.45): the server ACCEPTS the replace.
/// Int32(1) and Int64(1) are the same `_id`, so the copy plans a Replace (Overwrite) or a Replace
/// (Upsert) carrying the source `_id`, the server replaces the document in place and the stored
/// `_id` becomes Int32(1). One document remains, nothing duplicated, nothing failed.
#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn numeric_id_type_mismatch_is_reported_per_document_or_replaced() {
    if !docker_enabled("numeric_id_type_mismatch_is_reported_per_document_or_replaced") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    for (run, (policy, name)) in [
        (ConflictPolicy::Overwrite, "ow"),
        (ConflictPolicy::Upsert, "up"),
    ]
    .into_iter()
    .enumerate()
    {
        let ns = Namespace::new("cbit_numeric", name);
        seed(&client_a, &ns, vec![doc! { "_id": 1_i32, "v": "new" }]).await;
        seed(&client_b, &ns, vec![doc! { "_id": 1_i64, "v": "old" }]).await;
        let events = copy_ns!(&mut rt, run as u64 + 1, sa, sb, &ns, policy);
        assert_done(&events);
        let report = &reports(&events)[0];
        assert_eq!(report.outcome, CopyOutcome::Completed, "{name}");
        assert_eq!(report.progress.written, 1, "{name}");
        assert_eq!(report.progress.failed, 0, "{name}");
        assert!(report.failures.is_empty(), "{name}");

        let after = all_docs(&client_b, &ns).await;
        assert_eq!(after.len(), 1, "{name}: no duplicate");
        assert_eq!(after[0].get_str("v"), Ok("new"), "{name}: replaced");
        assert_eq!(
            after[0].get("_id"),
            Some(&Bson::Int32(1)),
            "{name}: source _id"
        );
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn mixed_id_types_copy_completely_between_servers() {
    if !docker_enabled("mixed_id_types_copy_completely_between_servers") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let ns = Namespace::new("cbit_mixed", "c");
    // 1,200 documents (three batches of 500) with four `_id` types interleaved.
    let docs: Vec<Document> = (0..300_i32)
        .flat_map(|i| {
            [
                doc! { "_id": ObjectId::new(), "i": i },
                doc! { "_id": format!("s{i}"), "i": i },
                doc! { "_id": 1_000_000 + i, "i": i },
                doc! { "_id": { "a": i, "b": "k" }, "i": i },
            ]
        })
        .collect();
    seed(&client_a, &ns, docs.clone()).await;
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    let events = copy_ns!(&mut rt, 1, sa, sb, &ns, ConflictPolicy::Skip);
    assert_done(&events);
    let report = &reports(&events)[0];
    assert_eq!(
        (report.progress.scanned, report.progress.written),
        (1200, 1200)
    );
    assert_eq!(report.progress.failed, 0);
    assert_eq!(count(&client_b, &ns).await, 1200);
    for d in &docs {
        let got = by_id(&client_b, &ns, d.get("_id").unwrap()).await;
        assert_eq!(got.as_ref(), Some(d), "a document differs after the copy");
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn copies_documents_with_dollar_and_dotted_field_names_faithfully() {
    if !docker_enabled("copies_documents_with_dollar_and_dotted_field_names_faithfully") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let ns = Namespace::new("cbit_names", "c");
    let docs = vec![
        doc! { "_id": 1, "a.b": "dotted", "nested": { "$x": 1, "c.d": 2 } },
        doc! { "_id": 2, "": "empty name", "arr": [ { "$in": 1 }, { "e.f": 2 } ] },
    ];
    seed(&client_a, &ns, docs.clone()).await;
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    // Insert (Skip), make the target documents differ, then Overwrite: the replace path must
    // really write both documents (`written == 2`), not skip them as equal.
    for (run, policy) in [(1, ConflictPolicy::Skip), (2, ConflictPolicy::Overwrite)] {
        if run == 2 {
            stale_target(&client_b, &ns, docs.len()).await;
        }
        let events = copy_ns!(&mut rt, run, sa, sb, &ns, policy);
        assert_done(&events);
        let progress = reports(&events)[0].progress;
        assert_eq!(progress.failed, 0, "{policy:?}");
        assert_eq!(progress.written, docs.len() as u64, "{policy:?}: written");
        let mut got = all_docs(&client_b, &ns).await;
        got.sort_by_key(|d| d.get_i32("_id").unwrap());
        assert_eq!(got, docs, "{policy:?}: documents are identical");
    }
}

/// Replaces documents `1..=n` of `ns` with different, plain content.
async fn stale_target(client: &mongodb::Client, ns: &Namespace, n: usize) {
    let coll = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection);
    for id in 1..=n as i32 {
        coll.replace_one(doc! { "_id": id }, doc! { "_id": id, "stale": true })
            .await
            .expect("make the target document differ");
    }
}

/// Observed on image mongo:7 (mongod 7.0.45): a document with a TOP-LEVEL `$`-prefixed field is
/// inserted faithfully, but REPLACING an existing document with it is refused by the server
/// (error 52, DollarPrefixedFieldName). The copy reports that as a bounded per-document failure
/// (`failed` counts it, `Rejected(52)` in the report) and leaves the target document untouched.
/// Nothing is corrupted, duplicated or lost; Overwrite/Upsert simply cannot update such documents.
#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn top_level_dollar_field_inserts_but_its_replace_is_rejected_per_document() {
    if !docker_enabled("top_level_dollar_field_inserts_but_its_replace_is_rejected_per_document") {
        return;
    }
    let (a, client_a, _sa) = docker_mongo().await;
    let (b, client_b, _sb) = docker_mongo().await;
    let ns = Namespace::new("cbit_dollar_top", "c");
    let docs = vec![
        doc! { "_id": 1, "$price": 5 },
        doc! { "_id": 2, "$": 1, "ok": true },
    ];
    seed(&client_a, &ns, docs.clone()).await;
    let mut rt = runtime();
    let (sa, sb) = (
        connect(&mut rt, &config(&a)).await,
        connect(&mut rt, &config(&b)).await,
    );
    let events = copy_ns!(&mut rt, 1, sa, sb, &ns, ConflictPolicy::Skip);
    assert_done(&events);
    assert_eq!(reports(&events)[0].progress.written, 2, "insert path");
    let mut got = all_docs(&client_b, &ns).await;
    got.sort_by_key(|d| d.get_i32("_id").unwrap());
    assert_eq!(got, docs, "inserted faithfully");

    stale_target(&client_b, &ns, docs.len()).await;
    let events = copy_ns!(&mut rt, 2, sa, sb, &ns, ConflictPolicy::Overwrite);
    let progress = reports(&events)[0].progress;
    assert_eq!((progress.written, progress.failed), (0, 2), "replace path");
    assert!(
        reports(&events)[0]
            .failures
            .iter()
            .all(|f| f.reason == WriteFailure::Rejected(52)),
        "each refusal is reported as the server's error 52"
    );
    assert_eq!(count(&client_b, &ns).await, 2, "nothing lost or duplicated");
    assert!(
        all_docs(&client_b, &ns)
            .await
            .iter()
            .all(|d| d.get_bool("stale") == Ok(true))
    );
}
