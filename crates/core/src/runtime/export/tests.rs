//! The export job end to end: a runtime over an in-memory store, a temp directory, the events.

use std::path::{Path, PathBuf};

use bson::{Bson, Document, doc};

use super::super::copy_support::{Point, TestStore, next};
use super::super::*;
use crate::compare::protocol::RunId;
use crate::error::CoreError;
use crate::export::protocol::{
    DocExportOutcome, DocExportRequest, DocFormat, ExportCommand, ExportEvent, JsonMode,
};
use crate::runtime::export_file::test_hooks;
use crate::test_support::mock_objects::mock_numbered_docs;

fn ns() -> Namespace {
    Namespace::new("s", "orders")
}

fn seeded(docs: Vec<Document>) -> TestStore {
    TestStore::new(&[("s", "orders", docs)])
}

/// Hands every connection the one store.
struct Fixed(Arc<TestStore>);

#[async_trait::async_trait]
impl Connector for Fixed {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> crate::error::Result<Arc<dyn crate::store::DocumentStore>> {
        Ok(self.0.clone())
    }
}

struct Rig {
    h: RuntimeHandle,
    store: Arc<TestStore>,
    session: SessionId,
    dir: tempfile::TempDir,
}

async fn rig(store: TestStore) -> Rig {
    rig_with(store, RuntimeConfig::default()).await
}

async fn rig_with(store: TestStore, config: RuntimeConfig) -> Rig {
    let store = Arc::new(store);
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(Fixed(store.clone())),
        Arc::new(|| {}),
        config,
    );
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    Rig {
        h,
        store,
        session: cfg.id,
        dir: tempfile::tempdir().unwrap(),
    }
}

impl Rig {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn request(&self, path: &Path, format: DocFormat) -> DocExportRequest {
        DocExportRequest {
            session: self.session,
            ns: ns(),
            filter: Document::new(),
            format,
            mode: JsonMode::Canonical,
            path: path.to_path_buf(),
        }
    }

    fn start(&self, run: RunId, request: DocExportRequest) {
        self.h
            .tx
            .send(Command::Export(ExportCommand::Start { run, request }))
            .unwrap();
    }

    async fn until_terminal(&mut self, run: RunId) -> Vec<ExportEvent> {
        let mut seen = Vec::new();
        loop {
            if let Event::Export(event) = next(&mut self.h).await {
                let terminal = matches!(
                    &event,
                    ExportEvent::Done { run: r, .. } | ExportEvent::Failed { run: r, .. } if *r == run
                );
                seen.push(event);
                if terminal {
                    return seen;
                }
            }
        }
    }

    async fn export(&mut self, run: RunId, request: DocExportRequest) -> Vec<ExportEvent> {
        self.start(run, request);
        self.until_terminal(run).await
    }
}

fn outcome(events: &[ExportEvent]) -> DocExportOutcome {
    match events.last() {
        Some(ExportEvent::Done { outcome, .. }) => *outcome,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn failure(events: &[ExportEvent]) -> CoreError {
    match events.last() {
        Some(ExportEvent::Failed { error, .. }) => error.clone(),
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn exports_ndjson_to_the_chosen_file_with_progress_and_done() {
    let docs = mock_numbered_docs(3);
    let mut r = rig(seeded(docs.clone())).await;
    let path = r.path("out.ndjson");
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    let done = outcome(&events);
    assert_eq!(
        (done.written, done.cancelled, done.file_left),
        (3, false, false)
    );
    assert_eq!(done.bytes, std::fs::metadata(&path).unwrap().len());
    assert!(matches!(
        events.first(),
        Some(ExportEvent::Progress {
            done: 3,
            total: Some(3),
            ..
        })
    ));
    let lines = read_lines(&path);
    assert_eq!(lines.len(), 3);
    for (line, original) in lines.iter().zip(&docs) {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let back = Bson::try_from(value).unwrap();
        assert_eq!(back.as_document().unwrap(), original);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[tokio::test]
async fn exports_a_json_array_that_parses_as_json() {
    let mut r = rig(seeded(mock_numbered_docs(4))).await;
    let path = r.path("out.json");
    let events = r.export(1, r.request(&path, DocFormat::JsonArray)).await;
    assert_eq!(outcome(&events).written, 4);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn the_filter_limits_what_is_written() {
    let docs: Vec<Document> = (0..5)
        .map(|n| doc! { "_id": n, "tag": if n % 2 == 0 { "even" } else { "odd" } })
        .collect();
    let mut r = rig(seeded(docs)).await;
    let path = r.path("some.ndjson");
    let request = DocExportRequest {
        filter: doc! { "tag": "even" },
        ..r.request(&path, DocFormat::Ndjson)
    };
    let events = r.export(1, request).await;
    assert_eq!(outcome(&events).written, 3);
    assert_eq!(read_lines(&path).len(), 3);
    // The in-memory store cannot count a filtered export, so its total is unknown.
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, ExportEvent::Progress { total: Some(_), .. }))
    );
}

#[tokio::test]
async fn an_empty_collection_writes_a_valid_empty_file_and_reports_zero() {
    let mut r = rig(seeded(vec![])).await;
    let nd = r.path("empty.ndjson");
    let events = r.export(1, r.request(&nd, DocFormat::Ndjson)).await;
    assert_eq!(outcome(&events).written, 0);
    assert_eq!(std::fs::metadata(&nd).unwrap().len(), 0);
    let arr = r.path("empty.json");
    let events = r.export(2, r.request(&arr, DocFormat::JsonArray)).await;
    assert_eq!(outcome(&events).written, 0);
    assert_eq!(std::fs::read_to_string(&arr).unwrap(), "[]\n");
}

#[tokio::test]
async fn cancel_removes_the_file_and_frees_the_slot_for_the_next_export() {
    let mut r = rig(seeded(mock_numbered_docs(30))).await;
    let path = r.path("cancelled.ndjson");
    let cancel = Command::Export(ExportCommand::Cancel { run: 1 });
    r.store.arm_at(&r.h.tx, Point::DuringScan, 2, vec![cancel]);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    let done = outcome(&events);
    assert!(done.cancelled && !done.file_left && done.file_created);
    assert!(!path.exists());
    let again = r.path("again.ndjson");
    let events = r.export(2, r.request(&again, DocFormat::Ndjson)).await;
    assert_eq!(outcome(&events).written, 30);
    assert!(again.exists());
}

#[tokio::test]
async fn cancel_during_a_hung_count_ends_the_run_cancelled_and_frees_the_slot() {
    let mut r = rig(seeded(mock_numbered_docs(3)).hanging_count()).await;
    let path = r.path("hung.ndjson");
    let cancel = Command::Export(ExportCommand::Cancel { run: 1 });
    r.store.arm_at(&r.h.tx, Point::DuringCount, 1, vec![cancel]);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    let done = outcome(&events);
    assert!(done.cancelled && !done.file_left);
    assert!(!path.exists());
}

#[tokio::test]
async fn a_write_failure_removes_the_file_and_fails_with_export_write_failed() {
    let mut r = rig(seeded(mock_numbered_docs(3))).await;
    let path = r.path("broken.ndjson");
    test_hooks::fail_writes_after(&path, 0);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(failure(&events), CoreError::ExportWriteFailed);
    assert!(!path.exists());
}

#[tokio::test]
async fn an_unusable_id_aborts_removes_the_file_and_fails_with_a_typed_error() {
    let docs = vec![doc! { "_id": 1 }, doc! { "_id": [1, 2] }];
    let mut r = rig(seeded(docs)).await;
    let path = r.path("bad.ndjson");
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(failure(&events), CoreError::CompareKeyInvalid);
    assert!(!path.exists());
}

#[tokio::test]
async fn a_closed_session_ends_with_failed_and_no_file() {
    let mut r = rig(seeded(mock_numbered_docs(3))).await;
    let path = r.path("closed.ndjson");
    let close = Command::Disconnect { session: r.session };
    r.store.arm_at(&r.h.tx, Point::DuringScan, 1, vec![close]);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(failure(&events), CoreError::Superseded);
    assert!(!path.exists());
}

#[tokio::test]
async fn a_second_export_while_one_runs_is_refused() {
    let mut r = rig(seeded(mock_numbered_docs(3))).await;
    let (first, second) = (r.path("first.ndjson"), r.path("second.ndjson"));
    let request = r.request(&second, DocFormat::Ndjson);
    let start = Command::Export(ExportCommand::Start { run: 2, request });
    r.store.arm_at(&r.h.tx, Point::DuringScan, 1, vec![start]);
    r.start(1, r.request(&first, DocFormat::Ndjson));
    let refused = r.until_terminal(2).await;
    assert!(matches!(
        refused.last(),
        Some(ExportEvent::Failed { run: 2, .. })
    ));
    assert!(!second.exists());
    let events = r.until_terminal(1).await;
    assert_eq!(outcome(&events).written, 3);
}

#[tokio::test]
async fn an_existing_file_is_overwritten() {
    let mut r = rig(seeded(mock_numbered_docs(2))).await;
    let path = r.path("old.ndjson");
    std::fs::write(&path, "x".repeat(10_000)).unwrap();
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(outcome(&events).written, 2);
    assert_eq!(read_lines(&path).len(), 2);
}

#[tokio::test]
async fn an_invalid_path_is_refused_before_anything_is_created() {
    let mut r = rig(seeded(mock_numbered_docs(2))).await;
    let missing_parent = r.path("nowhere").join("out.ndjson");
    let events = r
        .export(1, r.request(&missing_parent, DocFormat::Ndjson))
        .await;
    assert_eq!(events.len(), 1);
    assert!(matches!(failure(&events), CoreError::Unsupported(_)));
    assert!(!r.path("nowhere").exists());
    let relative = Path::new("relative.ndjson");
    let events = r.export(2, r.request(relative, DocFormat::Ndjson)).await;
    assert!(matches!(failure(&events), CoreError::Unsupported(_)));
    assert!(!relative.exists());
}

const GOOD: &str = "previous good export\n";

fn with_previous_export(r: &Rig, name: &str) -> PathBuf {
    let path = r.path(name);
    std::fs::write(&path, GOOD).unwrap();
    path
}

#[tokio::test]
async fn a_failing_first_read_leaves_an_existing_file_untouched() {
    let store = seeded(mock_numbered_docs(3)).failing_scan("orders", CoreError::AuthFailed);
    let mut r = rig(store).await;
    let path = with_previous_export(&r, "keep.ndjson");
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(failure(&events), CoreError::AuthFailed);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), GOOD);
}

#[tokio::test]
async fn cancel_during_the_first_read_leaves_an_existing_file_untouched() {
    let mut r = rig(seeded(mock_numbered_docs(3))).await;
    let path = with_previous_export(&r, "keep.ndjson");
    let cancel = Command::Export(ExportCommand::Cancel { run: 1 });
    r.store.arm_at(&r.h.tx, Point::DuringScan, 1, vec![cancel]);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    let done = outcome(&events);
    assert!(done.cancelled && !done.file_left && !done.file_created);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), GOOD);
}

#[tokio::test]
async fn cancel_during_a_hung_count_leaves_an_existing_file_untouched() {
    let mut r = rig(seeded(mock_numbered_docs(3)).hanging_count()).await;
    let path = with_previous_export(&r, "keep.ndjson");
    let cancel = Command::Export(ExportCommand::Cancel { run: 1 });
    r.store.arm_at(&r.h.tx, Point::DuringCount, 1, vec![cancel]);
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    let done = outcome(&events);
    assert!(done.cancelled && !done.file_created);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), GOOD);
}

#[tokio::test]
async fn an_empty_source_replaces_an_existing_file_with_the_valid_empty_file() {
    let mut r = rig(seeded(vec![])).await;
    let path = with_previous_export(&r, "old.json");
    let events = r.export(1, r.request(&path, DocFormat::JsonArray)).await;
    assert_eq!(outcome(&events).written, 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]\n");
}

#[tokio::test]
async fn a_hung_whole_collection_count_times_out_and_the_export_goes_on_without_a_total() {
    let config = RuntimeConfig {
        command_timeout: std::time::Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let mut r = rig_with(seeded(mock_numbered_docs(3)).hanging_count(), config).await;
    let path = r.path("nototal.ndjson");
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(outcome(&events).written, 3);
    assert!(events.iter().any(|e| matches!(
        e,
        ExportEvent::Progress {
            done: 3,
            total: None,
            ..
        }
    )));
}

#[tokio::test]
async fn progress_reports_the_real_count_and_drops_a_total_that_was_passed() {
    let mut r = rig(seeded(mock_numbered_docs(30)).with_estimated_count(2)).await;
    let path = r.path("stale.ndjson");
    let events = r.export(1, r.request(&path, DocFormat::Ndjson)).await;
    assert_eq!(outcome(&events).written, 30);
    let progress: Vec<(u64, Option<u64>)> = events
        .iter()
        .filter_map(|e| match e {
            ExportEvent::Progress { done, total, .. } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(progress.last(), Some(&(30, None)));
    assert!(
        progress
            .iter()
            .all(|(done, total)| total.is_none_or(|t| *done <= t))
    );
    assert!(progress.windows(2).all(|w| w[0].0 < w[1].0));
}
