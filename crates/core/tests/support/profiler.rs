//! Helpers for the profiler tests against a real server (tier 1): a workload client marked with
//! its own `appName`, the profiler level guard, raw reads of `system.profile`, and the scrubbing
//! capture that writes real profile documents to `src/profile/fixtures/real_<name>.json`.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use super::{LocalDbs, local_mongo};
use bson::{Bson, DateTime, Document, doc};
use curlybrace_core::ejson::doc_to_editor_json;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::profile::ProfileEntry;
use curlybrace_core::store::{DocumentStore, ProfileRead};
use futures::TryStreamExt;
use mongodb::Client;
use mongodb::{Collection, IndexModel};

pub const CAPTURE_ENV: &str = "CURLYBRACE_CAPTURE";
/// The `appName` of the workload client: not ours, so its traffic is what the poller records.
pub const APP: &str = "myapp";

/// Held for the whole body of every test in a profiler file: the tests change per-database
/// profiler levels and read server-global state, so they run one at a time.
pub static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

/// A raw driver client with the given `appName` (the workload that the profiler observes).
pub async fn app_client(app_name: &str) -> Client {
    let uri = curlybrace_testkit::local_uri();
    let uri = if uri.contains('?') {
        format!("{uri}&appName={app_name}")
    } else {
        format!("{}/?appName={app_name}", uri.trim_end_matches('/'))
    };
    Client::with_uri_str(&uri).await.expect("workload client")
}

/// Sets the profiler level of `db` through a raw command.
pub async fn set_level(client: &Client, db: &str, level: i32) -> Document {
    client
        .database(db)
        .run_command(doc! { "profile": level })
        .await
        .expect("profile command")
}

/// Resets the profiler of the test's own database to 0 when dropped, also after a failed
/// assertion (best effort, on a helper thread because Drop can run inside an async runtime).
pub struct ProfileReset {
    pub db: String,
}

impl Drop for ProfileReset {
    fn drop(&mut self) {
        let (uri, db) = (curlybrace_testkit::local_uri(), self.db.clone());
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Runtime::new() else {
                return;
            };
            rt.block_on(async {
                let Ok(client) = Client::with_uri_str(&uri).await else {
                    return;
                };
                let _ = client
                    .database(&db)
                    .run_command(doc! { "profile": 0 })
                    .await;
                client.shutdown().await;
            });
        })
        .join();
    }
}

/// Milliseconds since the epoch now, after a short pause so every later entry is strictly newer.
pub async fn start_mark() -> i64 {
    let now = DateTime::now().timestamp_millis();
    tokio::time::sleep(Duration::from_millis(5)).await;
    now
}

/// Every `system.profile` document of `db` newer than `since`, oldest first, minus reads of
/// `system.profile` itself.
pub async fn raw_profile(client: &Client, db: &str, since: i64) -> Vec<Document> {
    let docs: Vec<Document> = client
        .database(db)
        .collection::<Document>("system.profile")
        .find(doc! { "ts": { "$gt": DateTime::from_millis(since) } })
        .sort(doc! { "ts": 1 })
        .await
        .expect("read system.profile")
        .try_collect()
        .await
        .expect("collect system.profile");
    docs.into_iter()
        .filter(|d| {
            !d.get_str("ns")
                .unwrap_or_default()
                .ends_with(".system.profile")
        })
        .collect()
}

/// The first key of an entry's `command` (the verb), or "".
pub fn verb(entry: &Document) -> String {
    entry
        .get_document("command")
        .ok()
        .and_then(|c| c.keys().next().cloned())
        .unwrap_or_default()
}

/// The first raw entry whose command verb is `verb_name` and that satisfies `pred`.
pub fn find_entry<'a>(
    docs: &'a [Document],
    verb_name: &str,
    pred: impl Fn(&Document) -> bool,
) -> Option<&'a Document> {
    docs.iter().find(|d| verb(d) == verb_name && pred(d))
}

/// `n` documents with `a` cycling through 0..10, `b` unique and `c` unique-ish.
pub fn docs(n: i32) -> Vec<Document> {
    (0..n)
        .map(|i| doc! { "a": i % 10, "b": i, "c": (i * 7919) % n, "pad": "x" })
        .collect()
}

/// Replaces `cbit_<12 hex>` database names with `test`.
pub fn scrub_text(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("cbit_") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 5..];
        let hex = tail
            .chars()
            .take(12)
            .take_while(char::is_ascii_hexdigit)
            .count();
        if hex == 12 {
            out.push_str("test");
            rest = &tail[12..];
        } else {
            out.push_str("cbit_");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// Session fields, and the bulky per-stage statistics and lock/queue counters that nothing in
/// the profile code reads (`execStats` also repeats the query's literal values).
const DROPPED_KEYS: [&str; 9] = [
    "lsid",
    "$clusterTime",
    "txnNumber",
    "$readPreference",
    "execStats",
    "queues",
    "locks",
    "flowControl",
    "allUsers",
];

/// Removes what identifies the machine, the client or the throwaway database from a captured
/// profile document: host names, ports and addresses, session fields, the user and the OS.
pub fn scrub(value: Bson) -> Bson {
    match value {
        Bson::String(s) => Bson::String(scrub_text(&s)),
        Bson::Array(items) => Bson::Array(items.into_iter().map(scrub).collect()),
        Bson::Document(d) => {
            let mut out = Document::new();
            for (k, v) in d {
                if DROPPED_KEYS.contains(&k.as_str()) {
                    continue;
                }
                let v = match (k.as_str(), &v) {
                    ("client", Bson::String(_)) => Bson::String("127.0.0.1".into()),
                    ("user", Bson::String(_)) => Bson::String(String::new()),
                    ("host" | "me", Bson::String(_)) => Bson::String("localhost:27017".into()),
                    ("os", Bson::Document(_)) => Bson::Document(
                        doc! { "type": "Darwin", "name": "macOS", "architecture": "arm64", "version": "0" },
                    ),
                    _ => scrub(v),
                };
                out.insert(k, v);
            }
            Bson::Document(out)
        }
        other => other,
    }
}

/// Writes the scrubbed profile document to the fixtures directory when capturing is switched on.
pub fn capture(name: &str, entry: &Document) {
    if std::env::var(CAPTURE_ENV).as_deref() != Ok("1") {
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/profile/fixtures")
        .join(format!("real_{name}.json"));
    let Bson::Document(scrubbed) = scrub(Bson::Document(entry.clone())) else {
        unreachable!("scrub keeps a document a document")
    };
    let mut text = doc_to_editor_json(&scrubbed);
    text.push('\n');
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// A throwaway database with a workload client (`appName` "myapp") beside the marked store.
pub struct Env {
    /// Declared first so the profiler is switched off before the database is dropped.
    pub _reset: ProfileReset,
    pub dbs: LocalDbs,
    pub client: Client,
    pub store: MongoStore,
    pub app: Client,
}

impl Env {
    pub async fn new() -> Self {
        let (dbs, client, store) = local_mongo().await;
        let _reset = ProfileReset { db: dbs.db.clone() };
        Self {
            _reset,
            app: app_client(APP).await,
            dbs,
            client,
            store,
        }
    }

    pub fn db(&self) -> &str {
        &self.dbs.db
    }

    pub fn coll(&self, name: &str) -> Collection<Document> {
        self.app.database(self.db()).collection(name)
    }

    pub async fn seed(&self, name: &str, n: i32) {
        self.coll(name).insert_many(docs(n)).await.unwrap();
    }

    pub async fn index(&self, name: &str, keys: Document) {
        let model = IndexModel::builder().keys(keys).build();
        self.coll(name).create_index(model).await.unwrap();
    }

    pub async fn level(&self, level: i32) {
        set_level(&self.app, self.db(), level).await;
    }

    pub async fn read(&self, since: i64) -> ProfileRead {
        self.store
            .read_profile(self.db(), since)
            .await
            .expect("read_profile")
    }
}

/// The entries of the workload (reads of `system.profile` left out).
pub fn workload(read: &ProfileRead) -> Vec<&ProfileEntry> {
    read.entries
        .iter()
        .filter(|e| !e.ns.ends_with(".system.profile"))
        .collect()
}

pub fn verb_of(e: &ProfileEntry) -> String {
    e.command.keys().next().cloned().unwrap_or_default()
}

/// The first entry of `ns` whose command verb is `verb_name` and whose `command` satisfies `pred`.
pub fn entry_of<'a>(
    read: &'a ProfileRead,
    ns: &str,
    verb_name: &str,
    pred: impl Fn(&Document) -> bool,
) -> &'a ProfileEntry {
    read.entries
        .iter()
        .find(|e| e.ns == ns && verb_of(e) == verb_name && pred(&e.command))
        .unwrap_or_else(|| panic!("no {verb_name} entry for {ns} in the profile"))
}

/// The raw document behind [`entry_of`].
pub fn raw_of<'a>(
    raw: &'a [Document],
    ns: &str,
    verb_name: &str,
    pred: impl Fn(&Document) -> bool,
) -> &'a Document {
    find_entry(raw, verb_name, |d| {
        d.get_str("ns") == Ok(ns) && d.get_document("command").is_ok_and(&pred)
    })
    .unwrap_or_else(|| panic!("no raw {verb_name} entry for {ns}"))
}

pub fn filter_is(filter: Document) -> impl Fn(&Document) -> bool {
    move |c| c.get_document("filter").ok() == Some(&filter)
}

pub async fn collect(cursor: mongodb::Cursor<Document>) -> Vec<Document> {
    futures::TryStreamExt::try_collect(cursor).await.unwrap()
}
