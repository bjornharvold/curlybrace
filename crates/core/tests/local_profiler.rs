//! Tier 1 (local server): the profiler against a real MongoDB. Writes only to throwaway `cbit_*`
//! databases and only ever changes the profiler level of those (a guard resets it to 0, also
//! after a failed assertion). Run with `scripts/integration-tests.sh`.
//!
//! The unit tests of `core::profile` and `core::shape` use fixtures; these tests prove those
//! fixtures resemble what a real server writes. With `CURLYBRACE_CAPTURE=1` each test writes the
//! raw profile documents it saw to `src/profile/fixtures/real_<name>.json` (scrubbed).
//! Shapes of entries asserted here were captured on mongod 9.0.4.
//!
//! Every test takes [`serial`] first: profiler levels and the capped `system.profile` are
//! per-database, but the reads and the roll-off test are timing sensitive.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bson::{Document, doc};
use curlybrace_core::client::{build_client_options, connect};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::profile::{ProfileEntry, classify_entry, entry_has_filter, is_own_traffic};
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::shape::from_profile_command;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use mongodb::{Client, Collection};
use support::local_enabled;
use support::profiler::{
    APP, Env, capture, collect, entry_of, filter_is, raw_of, raw_profile, serial, start_mark,
    verb_of, workload,
};

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_profiling_level_roundtrip() {
    if !local_enabled("real_profiling_level_roundtrip") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    let before = env.store.profiling_level(env.db()).await.unwrap();
    assert_eq!(before.level, 0);
    assert_eq!(before.slow_ms, 100, "the server's default slowms");

    let on = env.store.set_profiling_level(env.db(), 2).await.unwrap();
    assert_eq!((on.level, on.slow_ms), (2, 100));
    assert_eq!(env.store.profiling_level(env.db()).await.unwrap(), on);

    let off = env.store.set_profiling_level(env.db(), 0).await.unwrap();
    assert_eq!(off.level, 0);
    assert_eq!(env.store.profiling_level(env.db()).await.unwrap().level, 0);

    let refused = env.store.set_profiling_level(env.db(), 3).await;
    assert!(
        matches!(refused, Err(CoreError::Unsupported(_))),
        "{refused:?}"
    );
    assert_eq!(env.store.profiling_level(env.db()).await.unwrap().level, 0);
}

/// Runs one of each operation against `orders` (300 documents, indexes on `c` and `a`).
async fn run_every_op(env: &Env) {
    let orders = env.coll("orders");
    orders.find(doc! { "b": 5 }).await.unwrap();
    let _ =
        futures::TryStreamExt::try_collect::<Vec<_>>(orders.find(doc! { "b": 5 }).await.unwrap())
            .await
            .unwrap();
    let _ =
        futures::TryStreamExt::try_collect::<Vec<_>>(orders.find(doc! { "c": 5 }).await.unwrap())
            .await
            .unwrap();
    let sorted = orders
        .find(doc! { "a": 3 })
        .sort(doc! { "b": -1 })
        .await
        .unwrap();
    let _ = futures::TryStreamExt::try_collect::<Vec<_>>(sorted)
        .await
        .unwrap();
    let db = env.app.database(env.db());
    db.run_command(doc! { "count": "orders", "query": { "a": 3 } })
        .await
        .unwrap();
    orders
        .distinct("a", doc! { "c": { "$gte": 0 } })
        .await
        .unwrap();
    let pipeline = vec![
        doc! { "$match": { "a": 3 } },
        doc! { "$group": { "_id": "$a", "n": { "$sum": 1 } } },
    ];
    let agg = orders.aggregate(pipeline).await.unwrap();
    let _ = futures::TryStreamExt::try_collect::<Vec<_>>(agg)
        .await
        .unwrap();
    orders
        .update_many(doc! { "a": 4 }, doc! { "$set": { "touched": true } })
        .await
        .unwrap();
    orders.delete_one(doc! { "b": 7 }).await.unwrap();
    orders
        .find_one_and_update(doc! { "b": 8 }, doc! { "$set": { "fam": 1 } })
        .await
        .unwrap();
}

async fn orders_with_indexes(env: &Env) {
    env.seed("orders", 300).await;
    env.index("orders", doc! { "c": 1 }).await;
    env.index("orders", doc! { "a": 1 }).await;
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_read_profile_returns_entries_with_the_expected_fields() {
    if !local_enabled("real_read_profile_returns_entries_with_the_expected_fields") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    orders_with_indexes(&env).await;
    let start = start_mark().await;
    env.level(2).await;
    run_every_op(&env).await;
    env.level(0).await;

    let read = env.read(start).await;
    assert_eq!(read.skipped, 0, "every real entry parses");
    assert!(
        read.host.is_empty() || read.host.contains(':'),
        "host {:?}",
        read.host
    );
    let ops = workload(&read);
    for e in &read.entries {
        assert!(e.ts_millis > start, "{e:?}");
        assert_eq!(e.ts_secs, e.ts_millis.div_euclid(1000));
        assert!(e.ns.starts_with(env.db()));
        assert_eq!(
            e.app_name.as_deref(),
            Some(if e.ns.ends_with(".system.profile") {
                "curlybrace"
            } else {
                APP
            })
        );
    }
    let newest = read.entries.iter().map(|e| e.ts_millis).max();
    assert_eq!(read.newest_ts_millis, newest);
    assert_eq!(
        read.oldest_ts_millis,
        read.entries.first().map(|e| e.ts_millis)
    );
    assert!(
        read.entries
            .windows(2)
            .all(|w| w[0].ts_millis <= w[1].ts_millis),
        "ascending"
    );
    let raw = raw_profile(&env.app, env.db(), start).await;
    assert_eq!(
        raw.len(),
        ops.len(),
        "the raw profile and the parsed read agree"
    );
    let ns = format!("{}.orders", env.db());
    for (name, v) in [
        ("count", "count"),
        ("distinct", "distinct"),
        ("aggregate", "aggregate"),
        ("find_and_modify", "findAndModify"),
    ] {
        capture(name, raw_of(&raw, &ns, v, |_| true));
    }
    // Update and delete name their op directly and `command` holds the statement itself.
    for (name, op) in [("update", "update"), ("delete", "remove")] {
        let d = raw.iter().find(|d| d.get_str("op") == Ok(op)).expect(op);
        capture(name, d);
    }
}

/// Verdict of an entry the way the poller computes it.
fn verdict(e: &ProfileEntry) -> VerdictKind {
    classify_entry(e, entry_has_filter(e))
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_entries_classify_as_expected() {
    if !local_enabled("real_entries_classify_as_expected") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("scan", 300).await;
    env.seed("ix", 300).await;
    env.index("ix", doc! { "c": 1 }).await;
    env.seed("srt", 300).await;
    env.index("srt", doc! { "a": 1 }).await;
    // `a` takes 1000 values (6 documents each), `b` is unique: with {b: 1, a: 1} the equality on
    // `a` cannot narrow the scan, so the wide `b` range is read to return 6 matches.
    let many: Vec<Document> = (0..6000)
        .map(|i| doc! { "a": i % 1000, "b": i, "pad": "x" })
        .collect();
    env.coll("wide").insert_many(many).await.unwrap();
    env.index("wide", doc! { "b": 1, "a": 1 }).await;

    let start = start_mark().await;
    env.level(2).await;
    let (scan, ix, srt, wide) = (
        env.coll("scan"),
        env.coll("ix"),
        env.coll("srt"),
        env.coll("wide"),
    );
    collect(scan.find(doc! { "c": 5 }).await.unwrap()).await;
    collect(scan.find(doc! {}).await.unwrap()).await;
    collect(ix.find(doc! { "c": 5 }).await.unwrap()).await;
    collect(
        srt.find(doc! { "a": 3 })
            .sort(doc! { "c": -1 })
            .await
            .unwrap(),
    )
    .await;
    let range = doc! { "a": 3, "b": { "$gte": 0, "$lt": 6000 } };
    collect(wide.find(range.clone()).await.unwrap()).await;
    env.level(0).await;

    let read = env.read(start).await;
    assert_eq!(read.skipped, 0);
    let ns = |c: &str| format!("{}.{c}", env.db());
    let no_filter = |c: &Document| c.get_document("filter").is_ok_and(Document::is_empty);
    let cases = [
        (
            "scan",
            filter_is(doc! { "c": 5 }),
            VerdictKind::CollectionScan,
            "find_collscan",
        ),
        (
            "ix",
            filter_is(doc! { "c": 5 }),
            VerdictKind::IndexUsed,
            "find_ixscan",
        ),
        (
            "srt",
            filter_is(doc! { "a": 3 }),
            VerdictKind::SortNotCovered,
            "find_sort",
        ),
        (
            "wide",
            filter_is(range),
            VerdictKind::InefficientIndex,
            "find_inefficient",
        ),
    ];
    let raw = raw_profile(&env.app, env.db(), start).await;
    for (coll, pred, expected, name) in cases {
        let e = entry_of(&read, &ns(coll), "find", &pred);
        assert_eq!(verdict(e), expected, "{name}: {e:?}");
        capture(name, raw_of(&raw, &ns(coll), "find", &pred));
    }
    let e = entry_of(&read, &ns("scan"), "find", no_filter);
    assert!(!entry_has_filter(e));
    assert_eq!(verdict(e), VerdictKind::NoFilter, "{e:?}");
    capture(
        "find_no_filter",
        raw_of(&raw, &ns("scan"), "find", no_filter),
    );
    // The facts the verdicts stand on.
    let sorted = entry_of(&read, &ns("srt"), "find", filter_is(doc! { "a": 3 }));
    assert!(sorted.has_sort_stage && sorted.plan_summary.contains("IXSCAN"));
    let indexed = entry_of(&read, &ns("ix"), "find", filter_is(doc! { "c": 5 }));
    assert_eq!(
        (
            indexed.keys_examined,
            indexed.docs_examined,
            indexed.returned
        ),
        (1, 1, 1)
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_shapes_are_value_free_and_group_by_shape() {
    if !local_enabled("real_shapes_are_value_free_and_group_by_shape") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("orders", 100).await;
    let start = start_mark().await;
    env.level(2).await;
    let orders = env.coll("orders");
    for i in 0..25_i32 {
        let filter = doc! { "email": format!("zebra-9137-{i}@example.com"), "age": 918_273 + i };
        collect(orders.find(filter).await.unwrap()).await;
    }
    collect(
        orders
            .find(doc! { "email": "quokka-5521@example.com" })
            .await
            .unwrap(),
    )
    .await;
    env.level(0).await;

    let read = env.read(start).await;
    let ns = format!("{}.orders", env.db());
    let finds: Vec<&ProfileEntry> = read
        .entries
        .iter()
        .filter(|e| e.ns == ns && verb_of(e) == "find")
        .collect();
    assert_eq!(finds.len(), 26);
    let shapes: Vec<_> = finds
        .iter()
        .map(|e| from_profile_command(&e.op, &e.command).expect("a find has a shape"))
        .collect();
    let hashes: std::collections::BTreeSet<String> = shapes.iter().map(|s| s.hash()).collect();
    assert_eq!(
        hashes.len(),
        2,
        "25 runs of one shape plus one other shape: {hashes:?}"
    );
    assert_eq!(
        shapes[..25]
            .iter()
            .map(|s| s.hash())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1
    );
    for shape in &shapes {
        let json = shape.json();
        for literal in [
            "zebra",
            "9137",
            "918273",
            "918290",
            "quokka",
            "5521",
            "example.com",
        ] {
            assert!(!json.contains(literal), "{literal} leaked into {json}");
        }
        assert!(json.contains("email"), "field names stay: {json}");
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_profile_commands_map_to_shapes() {
    if !local_enabled("real_profile_commands_map_to_shapes") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    orders_with_indexes(&env).await;
    let start = start_mark().await;
    env.level(2).await;
    run_every_op(&env).await;
    env.level(0).await;

    let read = env.read(start).await;
    let ns = format!("{}.orders", env.db());
    let mut seen = Vec::new();
    for e in read.entries.iter().filter(|e| e.ns == ns) {
        let shape = from_profile_command(&e.op, &e.command);
        seen.push((e.op.clone(), verb_of(e), shape.is_some()));
    }
    eprintln!("op/verb -> shape: {seen:?}");
    // Every operation with a filter yields a shape on this server (mongod 9.0.4); `createIndexes`
    // (run by the setup, before the profiler is on) and other verbs would yield none.
    for (op, verb_name, has_shape) in &seen {
        assert!(
            has_shape,
            "{op}/{verb_name} yielded no shape on this server"
        );
    }
    for expected in [
        "find",
        "count",
        "distinct",
        "aggregate",
        "q",
        "findAndModify",
    ] {
        assert!(
            seen.iter().any(|(_, v, _)| v == expected),
            "{expected} missing from {seen:?}"
        );
    }
}

/// A config for the local server with `app_name` in the URI (or none).
fn cfg_with_app(app_name: Option<&str>) -> ConnectionConfig {
    let uri = curlybrace_testkit::local_uri();
    let uri = match app_name {
        Some(name) => format!("{}/?appName={name}", uri.trim_end_matches('/')),
        None => uri,
    };
    ConnectionConfig::new("it", &uri)
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_own_app_name_marker_is_excluded() {
    if !local_enabled("real_own_app_name_marker_is_excluded") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    let plain = connect(&cfg_with_app(None), None, None).await.unwrap();
    let suffixed = connect(&cfg_with_app(Some("shopapp")), None, None)
        .await
        .unwrap();
    let options = build_client_options(&cfg_with_app(None), None, None)
        .await
        .unwrap();
    assert_eq!(options.app_name.as_deref(), Some("curlybrace"));
    let bare = Client::with_uri_str(curlybrace_testkit::local_uri())
        .await
        .unwrap();
    let writers: [(&str, &Client); 4] = [
        ("own_plain", &plain),
        ("own_suffixed", &suffixed),
        ("app", &env.app),
        ("bare", &bare),
    ];
    for (name, client) in &writers {
        let c: Collection<Document> = client.database(env.db()).collection(name);
        c.insert_one(doc! { "x": 1 }).await.unwrap();
    }
    let start = start_mark().await;
    env.level(2).await;
    for (name, client) in &writers {
        let c: Collection<Document> = client.database(env.db()).collection(name);
        collect(c.find(doc! { "x": 1 }).await.unwrap()).await;
    }
    env.level(0).await;

    let read = env.read(start).await;
    let ns = |c: &str| format!("{}.{c}", env.db());
    let find = |c: &str| entry_of(&read, &ns(c), "find", |_| true);
    assert_eq!(find("own_plain").app_name.as_deref(), Some("curlybrace"));
    assert_eq!(
        find("own_suffixed").app_name.as_deref(),
        Some("shopapp curlybrace")
    );
    assert_eq!(find("app").app_name.as_deref(), Some("myapp"));
    assert_eq!(find("bare").app_name, None);
    assert!(is_own_traffic(find("own_plain")));
    assert!(is_own_traffic(find("own_suffixed")));
    assert!(
        !is_own_traffic(find("app")),
        "a raw driver client named myapp is not ours"
    );
    assert!(
        !is_own_traffic(find("bare")),
        "a client without an appName is not ours"
    );
    let raw = raw_profile(&env.app, env.db(), start).await;
    capture(
        "own_traffic",
        raw_of(&raw, &ns("own_plain"), "find", |_| true),
    );
    capture(
        "own_traffic_prefixed",
        raw_of(&raw, &ns("own_suffixed"), "find", |_| true),
    );
    capture("app_find", raw_of(&raw, &ns("app"), "find", |_| true));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_system_profile_reads_and_explain_are_own_traffic() {
    if !local_enabled("real_system_profile_reads_and_explain_are_own_traffic") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("orders", 50).await;
    env.index("orders", doc! { "c": 1 }).await;
    let start = start_mark().await;
    env.level(2).await;
    // Every request below comes from the "myapp" client, so only the shape of the request can
    // make it ours.
    let app_db = env.app.database(env.db());
    collect(env.coll("system.profile").find(doc! {}).await.unwrap()).await;
    app_db
        .run_command(doc! { "explain": { "find": "orders", "filter": { "c": 5 } }, "verbosity": "executionStats" })
        .await
        .unwrap();
    collect(
        env.coll("orders")
            .aggregate(vec![doc! { "$indexStats": {} }])
            .await
            .unwrap(),
    )
    .await;
    collect(env.coll("orders").find(doc! { "c": 5 }).await.unwrap()).await;
    // The explain that this application itself sends.
    let ns = Namespace::new(env.db(), "orders");
    let q = Query {
        filter: doc! { "c": 5 },
        ..Query::default()
    };
    env.store.explain_find(&ns, &q).await.unwrap();
    env.level(0).await;

    let read = env.read(start).await;
    let orders = format!("{}.orders", env.db());
    let profile_read = read
        .entries
        .iter()
        .find(|e| {
            e.ns.ends_with(".system.profile")
                && verb_of(e) == "find"
                && e.app_name.as_deref() == Some(APP)
        })
        .expect("a system.profile read is profiled");
    assert!(is_own_traffic(profile_read));
    let explain = entry_of(&read, &orders, "explain", |_| true);
    assert_eq!(explain.app_name.as_deref(), Some(APP));
    assert!(is_own_traffic(explain), "{explain:?}");
    let stats = entry_of(&read, &orders, "aggregate", |_| true);
    assert!(is_own_traffic(stats), "{stats:?}");
    let plain = entry_of(&read, &orders, "find", |_| true);
    assert!(
        !is_own_traffic(plain),
        "an ordinary find of the same collection counts"
    );
    let ours = read
        .entries
        .iter()
        .filter(|e| e.app_name.as_deref() == Some("curlybrace") && e.ns == orders)
        .count();
    assert!(ours >= 1, "the application's own explain was profiled");
    assert!(
        read.entries
            .iter()
            .filter(|e| e.app_name.as_deref() == Some("curlybrace"))
            .all(is_own_traffic)
    );
    let raw = raw_profile(&env.app, env.db(), start).await;
    capture("explain", raw_of(&raw, &orders, "explain", |_| true));
    capture(
        "index_stats",
        raw_of(&raw, &orders, "aggregate", |c| {
            c.get_array("pipeline").is_ok_and(|p| {
                p.iter().any(|s| {
                    s.as_document()
                        .is_some_and(|d| d.contains_key("$indexStats"))
                })
            })
        }),
    );
    // The system.profile read is filtered out of `raw_profile`; fetch it directly for the fixture.
    let docs = collect(
        env.coll("system.profile")
            .find(doc! { "ns": format!("{}.system.profile", env.db()) })
            .await
            .unwrap(),
    )
    .await;
    if let Some(d) = docs.iter().find(|d| {
        d.get_document("command")
            .is_ok_and(|c| c.contains_key("find"))
    }) {
        capture("system_profile_read", d);
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_continuations_are_detected() {
    if !local_enabled("real_continuations_are_detected") {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    env.seed("orders", 20).await;
    let start = start_mark().await;
    env.level(2).await;
    let rows = collect(
        env.coll("orders")
            .find(doc! { "a": doc! { "$gte": 0 } })
            .batch_size(1)
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(rows.len(), 20);
    env.level(0).await;

    let read = env.read(start).await;
    let orders = format!("{}.orders", env.db());
    let first = entry_of(&read, &orders, "find", |_| true);
    assert!(!first.is_continuation());
    assert_eq!(verdict(first), VerdictKind::CollectionScan);
    let more: Vec<&ProfileEntry> = read
        .entries
        .iter()
        .filter(|e| e.ns == orders && verb_of(e) == "getMore")
        .collect();
    assert!(more.len() >= 10, "{} getMore entries", more.len());
    for e in &more {
        assert!(e.is_continuation(), "{e:?}");
        assert_eq!(verdict(e), VerdictKind::NoFilter);
        assert!(
            from_profile_command(&e.op, &e.command).is_none(),
            "a getMore has no shape"
        );
    }
    eprintln!("getMore op value: {:?}", more[0].op);
    let raw = raw_profile(&env.app, env.db(), start).await;
    capture("getmore", raw_of(&raw, &orders, "getMore", |_| true));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_set_profiling_level_is_refused_on_a_read_only_store_and_level_unchanged() {
    if !local_enabled(
        "real_set_profiling_level_is_refused_on_a_read_only_store_and_level_unchanged",
    ) {
        return;
    }
    let _serial = serial().await;
    let env = Env::new().await;
    let locked = Arc::new(AtomicBool::new(true));
    let store = ReadOnlyStore::wrap(Arc::new(MongoStore::new(env.client.clone())), locked);
    let refused = store.set_profiling_level(env.db(), 2).await;
    assert!(matches!(refused, Err(CoreError::ReadOnly)), "{refused:?}");
    assert_eq!(
        store.profiling_level(env.db()).await.unwrap().level,
        0,
        "reads stay allowed"
    );
    assert_eq!(
        env.store.profiling_level(env.db()).await.unwrap().level,
        0,
        "level unchanged"
    );
    assert!(
        store.read_profile(env.db(), 0).await.is_ok(),
        "reading the profile is allowed"
    );
}

/// Not feasible on the throwaway standalone: an unauthorised read needs a user whose role lacks
/// `find` on `system.profile`, and creating users is only possible once the server enforces
/// authentication (`--auth`), which would lock every other test out of the shared server. The
/// mapping of `Unauthorized` to a clear error is covered by `mongo_profile_tests` and the poller
/// tests; the docker auth tier (`docker_mongo_auth.rs`) is where a restricted user can be created.
#[tokio::test]
#[ignore = "not feasible without enabling auth server-wide; see the comment"]
async fn real_unauthorised_is_a_clear_error() {
    eprintln!("SKIPPED real_unauthorised_is_a_clear_error: needs a server started with --auth");
}
