use super::*;
use bson::oid::ObjectId;
use curlybrace_core::runtime::SessionId;

fn sid(n: u128) -> SessionId {
    SessionId::from_u128(n)
}

const A: u128 = 1;
const B: u128 = 2;

/// Two sessions, `A` and `B`; `closed` and `read_only` list the ones that are not usable.
struct Fixture {
    closed: Vec<SessionId>,
    read_only: Vec<SessionId>,
    names: Vec<(SessionId, String, Vec<String>)>,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            closed: vec![],
            read_only: vec![],
            names: vec![],
        }
    }

    fn with_names(mut self, session: u128, scope: &str, names: &[&str]) -> Self {
        self.names.push((
            sid(session),
            scope.to_string(),
            names.iter().map(|n| n.to_string()).collect(),
        ));
        self
    }

    fn paste(&self, clip: &CopyClipboard, target: &PasteTarget) -> Result<CopyRequest, PasteBlock> {
        let open = |s: SessionId| !self.closed.contains(&s);
        let read_only = |s: SessionId| self.read_only.contains(&s);
        let existing = |s: SessionId, scope: &str| {
            self.names
                .iter()
                .find(|(ns, sc, _)| *ns == s && sc == scope)
                .map(|(_, _, v)| v.clone())
                .unwrap_or_default()
        };
        let ctx = PasteContext {
            open: &open,
            read_only: &read_only,
            existing_names: &existing,
        };
        clip.paste_onto(target, &ctx)
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn collections(session: u128, db: &str, list: &[&str]) -> CopyClipboard {
    CopyClipboard::Collections {
        session: sid(session),
        db: db.into(),
        names: names(list),
    }
}

fn database(session: u128, db: &str) -> CopyClipboard {
    CopyClipboard::Database {
        session: sid(session),
        db: db.into(),
    }
}

fn documents(session: u128, db: &str, coll: &str, selection: DocSelection) -> CopyClipboard {
    CopyClipboard::Documents {
        session: sid(session),
        ns: Namespace::new(db, coll),
        selection,
    }
}

fn db_target(session: u128, db: &str) -> PasteTarget {
    PasteTarget::Database {
        session: sid(session),
        db: db.into(),
    }
}

fn coll_target(session: u128, db: &str, coll: &str) -> PasteTarget {
    PasteTarget::Collection {
        session: sid(session),
        ns: Namespace::new(db, coll),
    }
}

fn conn_target(session: u128) -> PasteTarget {
    PasteTarget::Connection {
        session: sid(session),
    }
}

fn item(src: (&str, &str), dst: (&str, &str), filter: Document) -> CopyItem {
    CopyItem {
        source: Namespace::new(src.0, src.1),
        target: Namespace::new(dst.0, dst.1),
        filter,
    }
}

#[test]
fn source_session_closed_is_refused() {
    let mut f = Fixture::new();
    f.closed.push(sid(A));
    let r = f.paste(&collections(A, "shop", &["orders"]), &db_target(B, "x"));
    assert_eq!(r, Err(PasteBlock::SourceClosed));
}

#[test]
fn target_session_closed_is_refused() {
    let mut f = Fixture::new();
    f.closed.push(sid(B));
    let r = f.paste(&collections(A, "shop", &["orders"]), &db_target(B, "x"));
    assert_eq!(r, Err(PasteBlock::TargetClosed));
}

#[test]
fn read_only_target_is_refused() {
    let mut f = Fixture::new();
    f.read_only.push(sid(B));
    let r = f.paste(&collections(A, "shop", &["orders"]), &db_target(B, "x"));
    assert_eq!(r, Err(PasteBlock::ReadOnlyTarget));
}

#[test]
fn a_read_only_source_is_fine() {
    let mut f = Fixture::new();
    f.read_only.push(sid(A));
    assert!(
        f.paste(&collections(A, "shop", &["orders"]), &db_target(B, "x"))
            .is_ok()
    );
}

#[test]
fn closed_source_wins_over_closed_target_and_read_only() {
    let mut f = Fixture::new();
    f.closed.extend([sid(A), sid(B)]);
    f.read_only.push(sid(B));
    let r = f.paste(&database(A, "shop"), &db_target(B, "x"));
    assert_eq!(r, Err(PasteBlock::SourceClosed));
    f.closed.remove(0);
    assert_eq!(
        f.paste(&database(A, "shop"), &conn_target(B)),
        Err(PasteBlock::TargetClosed)
    );
}

#[test]
fn database_onto_database_is_refused() {
    let r = Fixture::new().paste(&database(A, "shop"), &db_target(B, "x"));
    assert_eq!(r, Err(PasteBlock::DatabaseOntoDatabase));
}

#[test]
fn documents_onto_database_is_refused() {
    let sel = DocSelection::ids(vec![Bson::Int32(1)]).unwrap();
    let clip = documents(A, "shop", "orders", sel);
    let f = Fixture::new();
    assert_eq!(
        f.paste(&clip, &db_target(B, "x")),
        Err(PasteBlock::DocumentsOntoDatabase)
    );
    assert_eq!(
        f.paste(&clip, &conn_target(B)),
        Err(PasteBlock::DocumentsOntoDatabase)
    );
}

#[test]
fn collections_onto_collection_is_refused() {
    let r = Fixture::new().paste(
        &collections(A, "shop", &["orders"]),
        &coll_target(B, "x", "y"),
    );
    assert_eq!(r, Err(PasteBlock::CollectionsOntoCollection));
}

#[test]
fn collections_onto_a_connection_is_refused() {
    let r = Fixture::new().paste(&collections(A, "shop", &["orders"]), &conn_target(B));
    assert_eq!(r, Err(PasteBlock::CollectionsOntoConnection));
}

#[test]
fn database_onto_collection_is_refused() {
    let r = Fixture::new().paste(&database(A, "shop"), &coll_target(B, "x", "y"));
    assert_eq!(r, Err(PasteBlock::DatabaseOntoCollection));
}

#[test]
fn documents_onto_their_own_collection_are_refused() {
    let clip = documents(
        A,
        "shop",
        "orders",
        DocSelection::Query {
            filter: doc! {},
            sort: None,
        },
    );
    let f = Fixture::new();
    assert_eq!(
        f.paste(&clip, &coll_target(A, "shop", "orders")),
        Err(PasteBlock::SameNamespaceDocuments)
    );
    // The same names on another connection are a different collection.
    assert!(f.paste(&clip, &coll_target(B, "shop", "orders")).is_ok());
    assert!(f.paste(&clip, &coll_target(A, "shop", "archive")).is_ok());
}

#[test]
fn excluded_databases_cannot_be_copied_or_pasted_into() {
    let f = Fixture::new();
    let sel = DocSelection::Query {
        filter: doc! {},
        sort: None,
    };
    assert_eq!(
        f.paste(&collections(A, "admin", &["x"]), &db_target(B, "y")),
        Err(PasteBlock::ExcludedDatabase)
    );
    assert_eq!(
        f.paste(&database(A, "config"), &conn_target(B)),
        Err(PasteBlock::ExcludedDatabase)
    );
    assert_eq!(
        f.paste(&collections(A, "shop", &["x"]), &db_target(B, "local")),
        Err(PasteBlock::ExcludedDatabase)
    );
    assert_eq!(
        f.paste(
            &documents(A, "shop", "x", sel),
            &coll_target(B, "admin", "x")
        ),
        Err(PasteBlock::ExcludedDatabase)
    );
}

#[test]
fn clipboards_of_excluded_databases_cannot_be_created() {
    assert_eq!(
        CopyClipboard::database(sid(A), "local"),
        Err(PasteBlock::ExcludedDatabase)
    );
    assert_eq!(
        CopyClipboard::collections(sid(A), "admin", names(&["x"])),
        Err(PasteBlock::ExcludedDatabase)
    );
}

#[test]
fn system_collections_are_dropped_from_a_collections_clipboard() {
    let clip =
        CopyClipboard::collections(sid(A), "shop", names(&["orders", "system.views"])).unwrap();
    assert_eq!(clip, collections(A, "shop", &["orders"]));
}

#[test]
fn ids_selection_over_10000_is_refused() {
    let many: Vec<Bson> = (0..10_001).map(Bson::Int32).collect();
    assert_eq!(DocSelection::ids(many), Err(PasteBlock::TooManyDocuments));
    let cap: Vec<Bson> = (0..10_000).map(Bson::Int32).collect();
    assert!(DocSelection::ids(cap).is_ok());
}

#[test]
fn an_oversized_ids_clipboard_built_by_hand_is_refused_at_paste() {
    let many: Vec<Bson> = (0..10_001).map(Bson::Int32).collect();
    let clip = documents(A, "shop", "orders", DocSelection::Ids(many));
    assert_eq!(
        Fixture::new().paste(&clip, &coll_target(B, "shop", "orders")),
        Err(PasteBlock::TooManyDocuments)
    );
}

#[test]
fn collections_onto_database_builds_a_collections_request() {
    let r = Fixture::new()
        .paste(
            &collections(A, "shop", &["orders", "users"]),
            &db_target(B, "backup"),
        )
        .unwrap();
    assert_eq!(r.source, sid(A));
    assert_eq!(r.target, sid(B));
    assert_eq!(r.policy, ConflictPolicy::default());
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![
            item(("shop", "orders"), ("backup", "orders"), doc! {}),
            item(("shop", "users"), ("backup", "users"), doc! {}),
        ])
    );
}

#[test]
fn same_database_same_connection_suggests_unique_copy_names() {
    let f = Fixture::new().with_names(A, "shop", &["orders", "orders_copy", "users"]);
    let r = f
        .paste(
            &collections(A, "shop", &["orders", "users"]),
            &db_target(A, "shop"),
        )
        .unwrap();
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![
            item(("shop", "orders"), ("shop", "orders_copy2"), doc! {}),
            item(("shop", "users"), ("shop", "users_copy"), doc! {}),
        ])
    );
}

#[test]
fn another_database_on_the_same_connection_keeps_the_names() {
    let f = Fixture::new().with_names(A, "backup", &["orders"]);
    let r = f
        .paste(
            &collections(A, "shop", &["orders"]),
            &db_target(A, "backup"),
        )
        .unwrap();
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![item(
            ("shop", "orders"),
            ("backup", "orders"),
            doc! {}
        )])
    );
}

#[test]
fn the_same_database_name_on_another_connection_keeps_the_names() {
    let f = Fixture::new().with_names(A, "shop", &["orders"]);
    let r = f
        .paste(&collections(A, "shop", &["orders"]), &db_target(B, "shop"))
        .unwrap();
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![item(("shop", "orders"), ("shop", "orders"), doc! {})])
    );
}

#[test]
fn a_suggested_name_that_is_too_long_is_invalid() {
    let long = "c".repeat(250);
    let r = Fixture::new().paste(
        &collections(A, "shop", &[long.as_str()]),
        &db_target(A, "shop"),
    );
    assert_eq!(r, Err(PasteBlock::InvalidName(NameProblem::TooLong)));
}

#[test]
fn database_onto_connection_builds_a_database_request() {
    let r = Fixture::new()
        .paste(&database(A, "shop"), &conn_target(B))
        .unwrap();
    assert_eq!(r.source, sid(A));
    assert_eq!(r.target, sid(B));
    assert_eq!(
        r.scope,
        CopyScope::Database {
            source_db: "shop".into(),
            target_db: "shop".into()
        }
    );
}

#[test]
fn database_onto_its_own_connection_suggests_a_new_database_name() {
    let f = Fixture::new().with_names(A, "", &["shop", "shop_copy", "other"]);
    let r = f.paste(&database(A, "shop"), &conn_target(A)).unwrap();
    assert_eq!(
        r.scope,
        CopyScope::Database {
            source_db: "shop".into(),
            target_db: "shop_copy2".into()
        }
    );
}

#[test]
fn a_suggested_database_name_that_is_too_long_is_invalid() {
    let long = "d".repeat(60);
    let r = Fixture::new().paste(&database(A, &long), &conn_target(A));
    assert_eq!(r, Err(PasteBlock::InvalidName(NameProblem::TooLong)));
}

#[test]
fn documents_onto_collection_builds_an_ids_or_query_filter() {
    let id = ObjectId::new();
    let ids = DocSelection::ids(vec![Bson::ObjectId(id), Bson::String("k".into())]).unwrap();
    let r = Fixture::new()
        .paste(
            &documents(A, "shop", "orders", ids),
            &coll_target(B, "backup", "orders2"),
        )
        .unwrap();
    let filter = doc! { "_id": { "$in": [Bson::ObjectId(id), Bson::String("k".into())] } };
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![item(
            ("shop", "orders"),
            ("backup", "orders2"),
            filter
        )])
    );

    let query = DocSelection::Query {
        filter: doc! { "status": "open" },
        sort: Some(doc! { "n": 1 }),
    };
    let r = Fixture::new()
        .paste(
            &documents(A, "shop", "orders", query),
            &coll_target(B, "shop", "orders"),
        )
        .unwrap();
    assert_eq!(
        r.scope,
        CopyScope::Collections(vec![item(
            ("shop", "orders"),
            ("shop", "orders"),
            doc! { "status": "open" }
        )])
    );
}

#[test]
fn a_target_collection_that_is_not_valid_is_refused() {
    let clip = documents(
        A,
        "shop",
        "orders",
        DocSelection::Query {
            filter: doc! {},
            sort: None,
        },
    );
    let r = Fixture::new().paste(&clip, &coll_target(B, "backup", "system.x"));
    assert_eq!(r, Err(PasteBlock::InvalidName(NameProblem::SystemPrefix)));
}

#[test]
fn menu_label_keys_carry_the_count() {
    let sel = DocSelection::ids(vec![Bson::Int32(1), Bson::Int32(2)]).unwrap();
    let one = DocSelection::ids(vec![Bson::Int32(1)]).unwrap();
    let q = DocSelection::Query {
        filter: doc! {},
        sort: None,
    };
    assert_eq!(database(A, "s").menu_label_key(), (MENU_COPY_DATABASE, 1));
    assert_eq!(
        collections(A, "s", &["a", "b", "c"]).menu_label_key(),
        (MENU_COPY_COLLECTIONS, 3)
    );
    assert_eq!(
        collections(A, "s", &["a"]).menu_label_key(),
        (MENU_COPY_COLLECTION, 1)
    );
    assert_eq!(
        documents(A, "s", "c", sel).menu_label_key(),
        (MENU_COPY_DOCUMENTS, 2)
    );
    assert_eq!(
        documents(A, "s", "c", one).menu_label_key(),
        (MENU_COPY_DOCUMENT, 1)
    );
    assert_eq!(
        documents(A, "s", "c", q).menu_label_key(),
        (MENU_COPY_QUERY_RESULT, 0)
    );
}

#[test]
fn an_empty_ids_selection_is_refused_instead_of_copying_nothing() {
    assert_eq!(DocSelection::ids(vec![]), Err(PasteBlock::EmptySelection));
    let clip = documents(A, "shop", "orders", DocSelection::Ids(vec![]));
    assert_eq!(
        Fixture::new().paste(&clip, &coll_target(B, "shop", "orders")),
        Err(PasteBlock::EmptySelection)
    );
}

#[test]
fn documents_from_a_system_collection_are_refused() {
    let clip = documents(
        A,
        "shop",
        "system.views",
        DocSelection::Ids(vec![Bson::Int32(1)]),
    );
    assert_eq!(
        Fixture::new().paste(&clip, &coll_target(B, "crm", "views")),
        Err(PasteBlock::SystemSource)
    );
}
