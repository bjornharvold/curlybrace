//! The list of saved undo copies: opening reads the listing, a deletion asks first and only
//! removes the copy that was asked about, and a copy still listed afterwards is reported.
use super::*;
use crate::app::reducer::{Input, update};
use crate::app::state::Banner;
use curlybrace_core::runtime::Event;
use curlybrace_core::snapshot::SnapshotOp;
use curlybrace_core::store::Namespace;
use std::path::PathBuf;

fn run(n: u128) -> BulkRunId {
    BulkRunId(uuid::Uuid::from_u128(n))
}

fn info(n: u128) -> SnapshotInfo {
    SnapshotInfo {
        run: run(n),
        ns: Namespace::new("shop", "orders"),
        op: SnapshotOp::Delete,
        created_unix: 1_700_000_000,
        bytes: 2048,
    }
}

fn with_dir() -> AppState {
    AppState {
        snapshot_dir: Some(PathBuf::from("/snapshots")),
        ..AppState::default()
    }
}

fn act(s: AppState, a: CopiesAction) -> Out {
    update(s, Input::Copies(a))
}

fn listing(s: AppState, request: u64, rows: Vec<SnapshotInfo>) -> AppState {
    update(
        s,
        Input::Event(Event::Snapshots {
            request,
            snapshots: rows,
        }),
    )
    .0
}

/// An open list that has been read.
fn listed(rows: Vec<SnapshotInfo>) -> AppState {
    let (s, cmds) = act(with_dir(), CopiesAction::Open);
    let request = match cmds[..] {
        [Command::SnapshotList { request }] => request,
        _ => panic!("expected one SnapshotList"),
    };
    listing(s, request, rows)
}

fn rows(s: &AppState) -> Vec<BulkRunId> {
    let list = s.copies.as_ref().and_then(|c| c.list.clone());
    list.unwrap_or_default().iter().map(|i| i.run).collect()
}

#[test]
fn opening_reads_the_listing_and_shows_it_when_it_arrives() {
    let (s, cmds) = act(with_dir(), CopiesAction::Open);
    assert!(matches!(cmds[..], [Command::SnapshotList { .. }]));
    assert_eq!(s.copies.as_ref().unwrap().list, None, "loading");
    let request = s.copies.as_ref().unwrap().awaiting.unwrap().0;
    let s = listing(s, request, vec![info(1), info(2)]);
    assert_eq!(rows(&s), vec![run(1), run(2)]);
    assert!(!s.copies.unwrap().busy());
}

#[test]
fn without_a_folder_for_the_copies_there_is_nothing_to_open() {
    let (s, cmds) = act(AppState::default(), CopiesAction::Open);
    assert!(s.copies.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn a_listing_nobody_asked_for_is_ignored() {
    let (s, _) = act(with_dir(), CopiesAction::Open);
    let request = s.copies.as_ref().unwrap().awaiting.unwrap().0;
    let s = listing(s, request + 1, vec![info(1)]);
    assert_eq!(s.copies.as_ref().unwrap().list, None);
    assert!(s.copies.unwrap().busy());
}

#[test]
fn a_delete_without_the_question_removes_nothing() {
    let s = listed(vec![info(1)]);
    let (s, cmds) = act(s, CopiesAction::Delete(run(1)));
    assert!(cmds.is_empty());
    assert_eq!(rows(&s), vec![run(1)]);
}

#[test]
fn only_the_copy_that_was_asked_about_is_deleted() {
    let s = listed(vec![info(1), info(2)]);
    let (s, _) = act(s, CopiesAction::Ask(run(1)));
    let (s, cmds) = act(s, CopiesAction::Delete(run(2)));
    assert!(cmds.is_empty(), "the question was about another copy");
    let (s, cmds) = act(s, CopiesAction::Delete(run(1)));
    assert!(matches!(cmds[..], [Command::SnapshotDelete { run: r, .. }] if r == run(1)));
    assert!(s.copies.as_ref().unwrap().busy());
    assert_eq!(s.copies.unwrap().asking, None);
}

#[test]
fn keeping_it_drops_the_question_and_sends_nothing() {
    let s = listed(vec![info(1)]);
    let (s, _) = act(s, CopiesAction::Ask(run(1)));
    let (s, cmds) = act(s, CopiesAction::Keep);
    assert!(cmds.is_empty());
    assert_eq!(s.copies.unwrap().asking, None);
}

#[test]
fn a_deleted_copy_leaves_the_list_without_a_banner() {
    let s = listed(vec![info(1), info(2)]);
    let (s, _) = act(s, CopiesAction::Ask(run(1)));
    let (s, cmds) = act(s, CopiesAction::Delete(run(1)));
    let request = match cmds[..] {
        [Command::SnapshotDelete { request, .. }] => request,
        _ => panic!("expected SnapshotDelete"),
    };
    let s = listing(s, request, vec![info(2)]);
    assert_eq!(rows(&s), vec![run(2)]);
    assert!(s.banner.is_none());
}

#[test]
fn a_copy_that_is_still_listed_after_its_deletion_is_reported() {
    let s = listed(vec![info(1)]);
    let (s, _) = act(s, CopiesAction::Ask(run(1)));
    let (s, cmds) = act(s, CopiesAction::Delete(run(1)));
    let request = match cmds[..] {
        [Command::SnapshotDelete { request, .. }] => request,
        _ => panic!("expected SnapshotDelete"),
    };
    let s = listing(s, request, vec![info(1)]);
    assert_eq!(
        rows(&s),
        vec![run(1)],
        "the list shows what is really there"
    );
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "snapshot_delete",
            ..
        })
    ));
}

#[test]
fn nothing_is_asked_or_deleted_while_an_answer_is_awaited() {
    let (s, _) = act(with_dir(), CopiesAction::Open);
    let (s, _) = act(s, CopiesAction::Ask(run(1)));
    assert_eq!(s.copies.unwrap().asking, None);
}

#[test]
fn closing_removes_the_list() {
    let s = listed(vec![info(1)]);
    let (s, _) = act(s, CopiesAction::Close);
    assert!(s.copies.is_none());
}
