//! Tier 1 (local server): the mongosh code Copy as code writes for an aggregate with every option
//! runs in the real `mongosh`, and the read preference it carries is honoured. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`; the read preference
//! test needs a replica set member.

mod support;

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use bson::{Document, doc};
use curlybrace_core::codegen::{CodeOptions, CodeRequest, Language, generate};
use curlybrace_core::pipeline::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};
use curlybrace_core::store::Namespace;
use mongodb::{Client, IndexModel};
use support::shell::mongosh;
use support::{insert_labelled, local_enabled, local_mongo, replica_set_or_skip};

fn stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$sort": { "total": -1_i32 } },
    ]
}

fn every_option(read_preference: ReadPreferenceMode) -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Tertiary),
            case_level: Some(false),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: Some(Alternate::Shifted),
        }),
        hint: Some(HintOption::Name("status_1_total_-1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly \"report\" 'q' \\ $x ${y} #{z} %s {0} end".into()),
        read_preference: Some(read_preference),
    }
}

fn code_for(ns: &Namespace, options: PipelineOptions) -> String {
    let request = CodeRequest::Aggregate {
        ns: ns.clone(),
        stages: stages(),
        options,
    };
    generate(Language::Mongosh, &request, &CodeOptions::default()).expect("generates")
}

/// A temp file holding `code`, removed when dropped.
struct Script(PathBuf);

impl Script {
    fn write(code: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cbit_codegen_{}_{n}.js", std::process::id()));
        std::fs::write(&path, code).expect("write the script");
        Self(path)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_mongosh(exe: &std::path::Path, uri: &str, script: &Script) -> Output {
    Command::new(exe)
        .arg(uri)
        .arg("--quiet")
        .arg("--file")
        .arg(&script.0)
        .output()
        .expect("run mongosh")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

async fn orders(client: &Client, ns: &Namespace) {
    let coll = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection);
    coll.create_index(
        IndexModel::builder()
            .keys(doc! { "status": 1_i32, "total": -1_i32 })
            .build(),
    )
    .await
    .expect("create the index");
    insert_labelled(
        client,
        ns,
        &[
            doc! { "_id": "o1", "status": "A", "total": 10_i32 },
            doc! { "_id": "o2", "status": "B", "total": 99_i32 },
            doc! { "_id": "o3", "status": "A", "total": 30_i32 },
        ],
    )
    .await;
}

fn with_params(uri: &str, params: &str) -> String {
    match (uri.contains('?'), uri.ends_with('/') || uri.contains("/?")) {
        (true, _) => format!("{uri}&{params}"),
        (false, true) => format!("{uri}?{params}"),
        (false, false) => format!("{uri}/?{params}"),
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB and mongosh; see scripts/integration-tests.sh"]
async fn the_mongosh_code_with_every_option_runs() {
    let name = "the_mongosh_code_with_every_option_runs";
    if !local_enabled(name) {
        return;
    }
    let Some(exe) = mongosh(name) else {
        return;
    };
    let (dbs, client, _) = local_mongo().await;
    let ns = dbs.ns("orders");
    orders(&client, &ns).await;
    let script = Script::write(&code_for(
        &ns,
        every_option(ReadPreferenceMode::SecondaryPreferred),
    ));
    let output = run_mongosh(&exe, &curlybrace_testkit::local_uri(), &script);
    let shown = text(&output);
    assert!(output.status.success(), "mongosh failed: {shown}");
    let (first, second) = (shown.find("o3"), shown.find("o1"));
    assert!(first.is_some() && second.is_some(), "{shown}");
    assert!(first < second, "sorted by total, descending: {shown}");
    assert!(!shown.contains("o2"), "only the A orders: {shown}");
}

#[tokio::test]
#[ignore = "needs a local replica set and mongosh; see scripts/integration-tests.sh"]
async fn the_mongosh_read_preference_option_is_honoured() {
    let name = "the_mongosh_read_preference_option_is_honoured";
    if !local_enabled(name) {
        return;
    }
    let Some(exe) = mongosh(name) else {
        return;
    };
    let (dbs, client, _) = local_mongo().await;
    if !replica_set_or_skip(&client, name).await {
        return;
    }
    let ns = dbs.ns("orders");
    orders(&client, &ns).await;
    let uri = with_params(
        &curlybrace_testkit::local_uri(),
        "serverSelectionTimeoutMS=2000",
    );
    let only = |mode| PipelineOptions {
        read_preference: Some(mode),
        ..PipelineOptions::default()
    };
    let secondary = Script::write(&code_for(&ns, only(ReadPreferenceMode::Secondary)));
    let refused = run_mongosh(&exe, &uri, &secondary);
    assert!(
        !refused.status.success(),
        "a secondary read on a one-member set must fail: {}",
        text(&refused)
    );
    assert!(
        text(&refused).to_lowercase().contains("server selection"),
        "{}",
        text(&refused)
    );
    let preferred = Script::write(&code_for(&ns, only(ReadPreferenceMode::SecondaryPreferred)));
    let served = run_mongosh(&exe, &uri, &preferred);
    assert!(served.status.success(), "{}", text(&served));
}
