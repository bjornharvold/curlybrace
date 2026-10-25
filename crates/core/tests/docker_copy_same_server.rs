//! Copy between databases of ONE real server, through the runtime with one open session.
//! These belong to tier 1 in spirit (one server), but no local mongod runs on the machines that
//! run them, so they are tier 2 (docker): own `cb-it-*` container per test; run with
//! `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test docker_copy_same_server -- --ignored`.

mod support;

use bson::doc;
use curlybrace_core::copy::protocol::CopyItem;
use curlybrace_core::copy::{ConflictPolicy, CopyEvent, CopyOutcome, CopyScope};
use curlybrace_core::error::{CopyRefusal, CoreError};
use curlybrace_core::store::Namespace;
use support::copy_flow::{
    all_docs, collection_names, connect, db_names, has_planned, id_v, item, numbered_range,
    reports, request, run_copy, runtime, seed,
};
use support::{config, docker_enabled, docker_mongo};

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn copies_a_collection_to_another_database_on_one_server() {
    if !docker_enabled("copies_a_collection_to_another_database_on_one_server") {
        return;
    }
    let (c, client, _store) = docker_mongo().await;
    let src = Namespace::new("cbit_src", "people");
    let dst = Namespace::new("cbit_dst", "people_copy");
    seed(&client, &src, numbered_range(1, 750, "a")).await;

    let mut rt = runtime();
    let session = connect(&mut rt, &config(&c)).await;
    let scope = CopyScope::Collections(vec![item(&src, &dst)]);
    let events = run_copy(
        &mut rt,
        1,
        request(session, session, scope, ConflictPolicy::Skip),
    )
    .await;

    assert!(matches!(
        events.last(),
        Some(CopyEvent::Done {
            cancelled: false,
            ..
        })
    ));
    let report = &reports(&events)[0];
    assert_eq!(report.outcome, CopyOutcome::Completed);
    assert_eq!(
        (report.progress.scanned, report.progress.written),
        (750, 750)
    );
    let copied = all_docs(&client, &dst).await;
    assert_eq!(copied.len(), 750);
    assert!(id_v(&copied).iter().all(|(_, v)| v == "a"));
    assert_eq!(all_docs(&client, &src).await.len(), 750, "source untouched");
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn copies_a_database_under_a_new_name_on_one_server() {
    if !docker_enabled("copies_a_database_under_a_new_name_on_one_server") {
        return;
    }
    let (c, client, _store) = docker_mongo().await;
    for (name, n) in [("alpha", 3), ("beta", 600), ("gamma", 1)] {
        seed(
            &client,
            &Namespace::new("cbit_whole", name),
            numbered_range(1, n, name),
        )
        .await;
    }
    let mut rt = runtime();
    let session = connect(&mut rt, &config(&c)).await;
    let scope = CopyScope::Database {
        source_db: "cbit_whole".into(),
        target_db: "cbit_renamed".into(),
    };
    let events = run_copy(
        &mut rt,
        1,
        request(session, session, scope, ConflictPolicy::Skip),
    )
    .await;

    assert!(matches!(
        events.last(),
        Some(CopyEvent::Done {
            cancelled: false,
            ..
        })
    ));
    assert_eq!(reports(&events).len(), 3);
    assert_eq!(
        collection_names(&client, "cbit_renamed").await,
        ["alpha", "beta", "gamma"]
    );
    for (name, n) in [("alpha", 3), ("beta", 600), ("gamma", 1)] {
        let from = all_docs(&client, &Namespace::new("cbit_whole", name)).await;
        let to = all_docs(&client, &Namespace::new("cbit_renamed", name)).await;
        assert_eq!(id_v(&to), id_v(&from), "{name}");
        assert_eq!(to.len(), n as usize);
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn system_databases_are_never_copied() {
    if !docker_enabled("system_databases_are_never_copied") {
        return;
    }
    let (c, client, _store) = docker_mongo().await;
    seed(
        &client,
        &Namespace::new("cbit_plain", "c"),
        numbered_range(1, 3, "a"),
    )
    .await;
    let before: Vec<_> = {
        let mut v = Vec::new();
        for db in ["admin", "local", "config"] {
            v.push(collection_names(&client, db).await);
        }
        v
    };
    let dbs_before = db_names(&client).await;

    let mut rt = runtime();
    let session = connect(&mut rt, &config(&c)).await;
    let mut run = 0;
    for sys in ["admin", "local", "config"] {
        // A system database as the source and as the target, by namespace and by database.
        let scopes = [
            CopyScope::Collections(vec![item(
                &Namespace::new(sys, "system.version"),
                &Namespace::new("cbit_out", "x"),
            )]),
            CopyScope::Collections(vec![item(
                &Namespace::new("cbit_plain", "c"),
                &Namespace::new(sys, "cbit_injected"),
            )]),
            CopyScope::Database {
                source_db: sys.into(),
                target_db: "cbit_out".into(),
            },
            CopyScope::Database {
                source_db: "cbit_plain".into(),
                target_db: sys.into(),
            },
        ];
        for scope in scopes {
            run += 1;
            let events = run_copy(
                &mut rt,
                run,
                request(session, session, scope, ConflictPolicy::Upsert),
            )
            .await;
            assert!(!has_planned(&events), "{sys}: refused before planning");
            assert!(
                matches!(
                    events.as_slice(),
                    [CopyEvent::Failed {
                        error: CoreError::CopyRefused(CopyRefusal::ServerOwned),
                        ..
                    }]
                ),
                "{sys}: a single typed refusal"
            );
        }
    }
    for (i, db) in ["admin", "local", "config"].iter().enumerate() {
        assert_eq!(collection_names(&client, db).await, before[i], "{db}");
    }
    assert_eq!(db_names(&client).await, dbs_before, "no database appeared");
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_database_copy_leaves_out_views() {
    if !docker_enabled("a_database_copy_leaves_out_views") {
        return;
    }
    let (c, client, _store) = docker_mongo().await;
    let src = Namespace::new("cbit_withview", "base");
    seed(&client, &src, numbered_range(1, 5, "a")).await;
    // A view is not a plain collection, so a database copy leaves it out.
    client
        .database("cbit_withview")
        .run_command(doc! { "create": "a_view", "viewOn": "base", "pipeline": [] })
        .await
        .unwrap();
    let mut rt = runtime();
    let session = connect(&mut rt, &config(&c)).await;
    let scope = CopyScope::Database {
        source_db: "cbit_withview".into(),
        target_db: "cbit_viewless".into(),
    };
    let events = run_copy(
        &mut rt,
        1,
        request(session, session, scope, ConflictPolicy::Skip),
    )
    .await;
    assert!(matches!(
        events.last(),
        Some(CopyEvent::Done {
            cancelled: false,
            ..
        })
    ));
    assert_eq!(collection_names(&client, "cbit_viewless").await, ["base"]);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn copies_selected_documents_by_id_between_collections() {
    if !docker_enabled("copies_selected_documents_by_id_between_collections") {
        return;
    }
    let (c, client, _store) = docker_mongo().await;
    let src = Namespace::new("cbit_pick", "orders");
    let dst = Namespace::new("cbit_pick", "picked");
    let mut docs = numbered_range(1, 20, "a");
    docs.push(doc! { "_id": "k", "v": "text-id" });
    seed(&client, &src, docs).await;

    let mut rt = runtime();
    let session = connect(&mut rt, &config(&c)).await;
    // What the documents pane copies by hand: the picked `_id`s, of any BSON type.
    let picked = CopyItem {
        filter: doc! { "_id": { "$in": [bson::Bson::Int32(3), bson::Bson::Int32(7), bson::Bson::String("k".into())] } },
        ..item(&src, &dst)
    };
    let scope = CopyScope::Collections(vec![picked]);
    let events = run_copy(
        &mut rt,
        1,
        request(session, session, scope, ConflictPolicy::Skip),
    )
    .await;

    assert!(matches!(
        events.last(),
        Some(CopyEvent::Done {
            cancelled: false,
            ..
        })
    ));
    let copied = all_docs(&client, &dst).await;
    let mut ids: Vec<String> = copied
        .iter()
        .map(|d| d.get("_id").unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, ["\"k\"", "3", "7"]);
    assert_eq!(all_docs(&client, &src).await.len(), 21, "source untouched");
}
