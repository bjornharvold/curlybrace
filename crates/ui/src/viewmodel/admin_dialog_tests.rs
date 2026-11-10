//! The administration dialogs: what each needs before it can be sent, and what it sends.

use super::{AdminDialog, AdminKind, AdminProblem, AdminRequest};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn open(kind: AdminKind, db: &str, collection: Option<&str>) -> AdminDialog {
    AdminDialog::open(AdminRequest {
        kind,
        db: db.into(),
        collection: collection.map(str::to_owned),
    })
}

#[test]
fn a_drop_is_ready_only_once_the_exact_name_was_typed() {
    let d = open(AdminKind::DropCollection, "shop", Some("orders"));
    assert_eq!(d.required_text(), Some("orders"));
    assert_eq!(d.problem(), Some(AdminProblem::ConfirmMismatch));
    assert!(!d.ready());
    let near = d.clone().with_typed("Orders".into());
    assert!(!near.ready(), "case matters");
    let exact = d.with_typed("orders".into());
    assert!(exact.ready());
}

#[test]
fn a_database_drop_asks_for_the_database_name() {
    let d = open(AdminKind::DropDatabase, "shop", None);
    assert_eq!(d.required_text(), Some("shop"));
    assert!(!d.ready());
    assert!(d.with_typed("shop".into()).ready());
}

#[test]
fn a_blank_typed_text_never_confirms() {
    let d = open(AdminKind::DropCollection, "shop", Some("orders"));
    assert!(!d.with_typed("   ".into()).ready());
}

#[test]
fn a_rename_needs_a_valid_new_name_that_differs() {
    let d = open(AdminKind::Rename, "shop", Some("orders"));
    assert_eq!(d.problem(), Some(AdminProblem::SameName));
    assert_eq!(
        d.clone().with_name("".into()).problem(),
        Some(AdminProblem::Required)
    );
    assert_eq!(
        d.clone().with_name("a$b".into()).problem(),
        Some(AdminProblem::InvalidName)
    );
    assert_eq!(
        d.clone().with_name("system.x".into()).problem(),
        Some(AdminProblem::InvalidName)
    );
    assert!(d.with_name("orders_2024".into()).ready());
}

#[test]
fn a_duplicate_starts_with_a_copy_name_and_checks_it_like_any_new_name() {
    let d = open(AdminKind::Duplicate, "shop", Some("orders"));
    assert_eq!(d.name, "orders_copy");
    assert!(d.ready());
    assert_eq!(
        d.with_name("bad$".into()).problem(),
        Some(AdminProblem::InvalidName)
    );
}

#[test]
fn a_view_needs_a_name_a_source_and_a_pipeline_that_parses() {
    let d = open(AdminKind::CreateView, "shop", None);
    assert_eq!(d.problem(), Some(AdminProblem::Required));
    let named = d.with_name("big".into());
    assert_eq!(
        named.problem(),
        Some(AdminProblem::Required),
        "no source yet"
    );
    let sourced = named.with_source("orders".into());
    assert!(sourced.ready(), "an empty pipeline mirrors the source");
    assert_eq!(
        sourced.clone().with_pipeline("{".into()).problem(),
        Some(AdminProblem::Pipeline)
    );
    assert_eq!(
        sourced.with_pipeline("{\"a\":1}".into()).problem(),
        Some(AdminProblem::Pipeline),
        "an object is not an array of stages"
    );
}

#[test]
fn a_view_opened_from_a_collection_row_has_that_source() {
    let d = open(AdminKind::CreateView, "shop", Some("orders"));
    assert_eq!(d.source, "orders");
}

#[test]
fn each_dialog_builds_the_command_for_its_kind() {
    let session = Uuid::nil();
    let ns = Namespace::new("shop", "orders");
    let drop = open(AdminKind::DropCollection, "shop", Some("orders")).with_typed("orders".into());
    assert!(matches!(
        drop.command(session),
        Some(Command::DropCollection { ns: n, .. }) if n == ns
    ));
    let db = open(AdminKind::DropDatabase, "shop", None).with_typed("shop".into());
    assert!(matches!(
        db.command(session),
        Some(Command::DropDatabase { db, .. }) if db == "shop"
    ));
    let rename = open(AdminKind::Rename, "shop", Some("orders"))
        .with_name("o2".into())
        .with_drop_target(true);
    assert!(matches!(
        rename.command(session),
        Some(Command::RenameCollection { ns: n, new_name, drop_target: true, .. })
            if n == ns && new_name == "o2"
    ));
    let dup = open(AdminKind::Duplicate, "shop", Some("orders"));
    assert!(matches!(
        dup.command(session),
        Some(Command::DuplicateCollection { ns: n, target, .. })
            if n == ns && target == "orders_copy"
    ));
    let view = open(AdminKind::CreateView, "shop", Some("orders")).with_name("v".into());
    assert!(matches!(
        view.command(session),
        Some(Command::CreateView { ns: n, source, pipeline_json, .. })
            if n == Namespace::new("shop", "v") && source == "orders" && pipeline_json == "[]"
    ));
}

#[test]
fn a_dialog_that_is_not_ready_builds_no_command() {
    let d = open(AdminKind::DropCollection, "shop", Some("orders"));
    assert!(d.command(Uuid::nil()).is_none());
}

#[test]
fn editing_clears_the_last_refusal() {
    let d = open(AdminKind::Rename, "shop", Some("orders"))
        .with_error(Some(curlybrace_core::error::CoreError::NamespaceExists));
    assert!(d.error.is_some());
    assert!(d.with_name("x".into()).error.is_none());
}

#[test]
fn the_pending_namespace_is_the_one_the_write_acts_on() {
    assert_eq!(
        open(AdminKind::DropDatabase, "shop", None).target_ns(),
        Namespace::new("shop", "")
    );
    assert_eq!(
        open(AdminKind::Rename, "shop", Some("orders")).target_ns(),
        Namespace::new("shop", "orders")
    );
    assert_eq!(
        open(AdminKind::CreateView, "shop", None)
            .with_name("v".into())
            .target_ns(),
        Namespace::new("shop", "v")
    );
}

#[test]
fn the_view_source_is_sent_without_surrounding_spaces() {
    let view = open(AdminKind::CreateView, "shop", None)
        .with_name("v".into())
        .with_source("  orders ".into());
    assert!(matches!(
        view.command(Uuid::nil()),
        Some(Command::CreateView { source, .. }) if source == "orders"
    ));
}
