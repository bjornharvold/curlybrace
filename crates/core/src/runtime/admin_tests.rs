//! The administration commands: classification, routing to the store, the read-only refusal
//! and the events they answer with.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::Document;

use super::handlers::command_time_limit;
use super::*;
use crate::admin::{CollectionStats, DatabaseStats};
use crate::store::{DocumentStore, Page, Query};
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);

/// Records the administration calls it receives, as readable strings.
#[derive(Default)]
struct AdminStore {
    calls: Mutex<Vec<String>>,
}

impl AdminStore {
    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl DocumentStore for AdminStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Ok(Page {
            docs: vec![],
            skip: 0,
            has_more: false,
        })
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<bson::Bson> {
        Ok(bson::Bson::Null)
    }
    async fn replace(&self, _ns: &Namespace, _id: &bson::Bson, _doc: Document) -> Result<()> {
        Ok(())
    }
    async fn delete(&self, _ns: &Namespace, _id: &bson::Bson) -> Result<()> {
        Ok(())
    }
    async fn drop_collection(&self, ns: &Namespace) -> Result<()> {
        self.note(format!("drop_collection {}.{}", ns.db, ns.collection));
        Ok(())
    }
    async fn drop_database(&self, db: &str) -> Result<()> {
        self.note(format!("drop_database {db}"));
        Ok(())
    }
    async fn rename_collection(&self, ns: &Namespace, to: &str, drop_target: bool) -> Result<()> {
        self.note(format!(
            "rename {}.{} -> {to} drop={drop_target}",
            ns.db, ns.collection
        ));
        Ok(())
    }
    async fn create_view(&self, ns: &Namespace, source: &str, pipeline: &[Document]) -> Result<()> {
        self.note(format!(
            "view {} on {source} stages={}",
            ns.collection,
            pipeline.len()
        ));
        Ok(())
    }
    async fn duplicate_collection(&self, ns: &Namespace, target: &str) -> Result<()> {
        self.note(format!("duplicate {} -> {target}", ns.collection));
        Ok(())
    }
    async fn collection_stats(&self, _ns: &Namespace) -> Result<CollectionStats> {
        Ok(CollectionStats {
            document_count: 9,
            ..CollectionStats::default()
        })
    }
    async fn database_stats(&self, _db: &str) -> Result<DatabaseStats> {
        Ok(DatabaseStats {
            collections: 4,
            ..DatabaseStats::default()
        })
    }
}

fn session() -> SessionId {
    EMBEDDED_SESSION
}

fn writes() -> Vec<Command> {
    let ns = mock_namespace();
    vec![
        Command::DropCollection {
            session: session(),
            ns: ns.clone(),
        },
        Command::DropDatabase {
            session: session(),
            db: ns.db.clone(),
        },
        Command::RenameCollection {
            session: session(),
            ns: ns.clone(),
            new_name: "renamed".into(),
            drop_target: true,
        },
        Command::CreateView {
            session: session(),
            ns: ns.clone(),
            source: "src".into(),
            pipeline_json: "[{\"$match\":{\"a\":1}}]".into(),
        },
        Command::DuplicateCollection {
            session: session(),
            ns,
            target: "copy".into(),
        },
    ]
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn spawn(store: Arc<AdminStore>) -> RuntimeHandle {
    spawn_runtime_with_config(None, Some(store), Arc::new(|| {}), RuntimeConfig::default())
}

#[test]
fn admin_commands_are_classified() {
    let expected = [
        ("drop_collection", true),
        ("drop_database", true),
        ("rename_collection", true),
        ("create_view", true),
        ("duplicate_collection", true),
    ];
    for (command, (context, write)) in writes().iter().zip(expected) {
        assert_eq!(command.context(), context);
        assert_eq!(command.is_write(), write, "{context}");
        assert_eq!(command.session(), Some(session()));
        assert_eq!(command.request(), None);
    }
    let reads = [
        Command::CollectionStats {
            session: session(),
            ns: mock_namespace(),
            request: 3,
        },
        Command::DatabaseStats {
            session: session(),
            db: "d".into(),
            request: 4,
        },
    ];
    for (command, (context, request)) in reads
        .iter()
        .zip([("collection_stats", 3), ("database_stats", 4)])
    {
        assert_eq!(command.context(), context);
        assert!(!command.is_write());
        assert_eq!(command.request(), Some(request));
    }
}

#[tokio::test]
async fn each_write_reaches_the_store_and_answers_admin_changed() {
    let store = Arc::new(AdminStore::default());
    let mut h = spawn(store.clone());
    for command in writes() {
        h.tx.send(command).unwrap();
        let Event::AdminChanged { db, .. } = next(&mut h).await else {
            panic!("expected AdminChanged");
        };
        assert_eq!(db, mock_namespace().db);
    }
    let ns = mock_namespace();
    let (db, c) = (&ns.db, &ns.collection);
    assert_eq!(
        store.calls(),
        vec![
            format!("drop_collection {db}.{c}"),
            format!("drop_database {db}"),
            format!("rename {db}.{c} -> renamed drop=true"),
            format!("view {c} on src stages=1"),
            format!("duplicate {c} -> copy"),
        ]
    );
}

#[tokio::test]
async fn a_bad_pipeline_fails_before_the_store_is_called() {
    let store = Arc::new(AdminStore::default());
    let mut h = spawn(store.clone());
    h.tx.send(Command::CreateView {
        session: session(),
        ns: mock_namespace(),
        source: "src".into(),
        pipeline_json: "{not an array}".into(),
    })
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::Failed { .. }));
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn statistics_are_answered_with_their_request_number() {
    let mut h = spawn(Arc::new(AdminStore::default()));
    h.tx.send(Command::CollectionStats {
        session: session(),
        ns: mock_namespace(),
        request: 7,
    })
    .unwrap();
    let Event::CollectionStats { stats, request, .. } = next(&mut h).await else {
        panic!("expected CollectionStats");
    };
    assert_eq!((stats.document_count, request), (9, 7));
    h.tx.send(Command::DatabaseStats {
        session: session(),
        db: "d".into(),
        request: 8,
    })
    .unwrap();
    let Event::DatabaseStats {
        stats, request, db, ..
    } = next(&mut h).await
    else {
        panic!("expected DatabaseStats");
    };
    assert_eq!((stats.collections, request, db.as_str()), (4, 8, "d"));
}

#[tokio::test]
async fn a_locked_connection_refuses_each_admin_write() {
    let store = Arc::new(AdminStore::default());
    let locked = crate::read_only_store::ReadOnlyStore::wrap(
        store.clone(),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    let mut h = spawn_runtime_with_config(
        None,
        Some(locked),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    for command in writes() {
        h.tx.send(command).unwrap();
        let Event::Failed { error, .. } = next(&mut h).await else {
            panic!("expected Failed");
        };
        assert_eq!(error, CoreError::ReadOnly);
    }
    assert!(store.calls().is_empty());
}

#[test]
fn a_duplicate_runs_under_the_long_bound_and_other_admin_writes_do_not() {
    let config = RuntimeConfig {
        command_timeout: Duration::from_secs(1),
        index_build_timeout: Duration::from_secs(9),
        ..RuntimeConfig::default()
    };
    let all = writes();
    assert_eq!(command_time_limit(&config, &all[4]), Duration::from_secs(9));
    for other in &all[..4] {
        assert_eq!(command_time_limit(&config, other), Duration::from_secs(1));
    }
}
