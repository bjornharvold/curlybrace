//! Fixtures for the apply tests, and the review and confirmation of an apply.
use super::*;
use crate::viewmodel::compare::{ApplyUi, RequiredText, RowAction, Scope};
use curlybrace_core::compare::apply::{ApplyKind, Direction, IdPolicy};

/// The localized confirmation word the fixtures' view hands in (never the English default, so a
/// reducer that ignored it would show).
pub(super) const WORD: &str = "wipe-it";

pub(super) fn key(i: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(i)])
}

pub(super) fn summary_of(cat: Category, n: u64) -> Summary {
    match cat {
        Category::OnlyInSource => Summary {
            only_in_source: n,
            ..Summary::default()
        },
        Category::OnlyInTarget => Summary {
            only_in_target: n,
            ..Summary::default()
        },
        Category::Different => Summary {
            different: n,
            ..Summary::default()
        },
    }
}

pub(super) fn rows_of(cat: Category, n: i32) -> Input {
    ev(CompareEvent::Rows {
        session: 1,
        pair: RUN_PAIR,
        category: cat,
        offset: 0,
        total: n as usize,
        rows: (0..n)
            .map(|i| RowView {
                key: key(i),
                has_source: cat != Category::OnlyInTarget,
                has_target: cat != Category::OnlyInSource,
            })
            .collect(),
    })
}

/// A finished run with `n` rows of `cat` loaded and open. The run is still active when
/// `run_done_too` is false.
pub(super) fn tab(cat: Category, n: i32, run_done_too: bool) -> AppState {
    let (state, _) = update(running_state(), pair_done(1, summary_of(cat, n as u64)));
    let (state, _) = update(state, rows_of(cat, n));
    match run_done_too {
        true => update(state, run_done()).0,
        false => state,
    }
}

pub(super) fn tab_of(cat: Category) -> AppState {
    tab(cat, 3, true)
}

pub(super) fn toggled(state: AppState, keys: &[i32]) -> AppState {
    keys.iter()
        .fold(state, |s, k| act(s, CompareAction::ToggleRow(key(*k))).0)
}

/// Rows 0 and 1 of the `cat` tab ticked and queued with `action`.
pub(super) fn pending_for(cat: Category, action: RowAction) -> AppState {
    let state = toggled(tab_of(cat), &[0, 1]);
    let (state, cmds) = act(state, CompareAction::AddToPending(action));
    assert!(cmds.is_empty());
    assert_eq!(state.compare.pending.items().len(), 1, "queued");
    state
}

/// Different rows 0 and 1 queued to copy to the target (a replace: nothing to type).
pub(super) fn with_pending() -> AppState {
    pending_for(Category::Different, RowAction::CopyToTarget)
}

/// Only-in-source rows 0 and 1 queued for deletion from the source (`a.users`).
pub(super) fn with_delete_pending() -> AppState {
    pending_for(Category::OnlyInSource, RowAction::DeleteFromSource)
}

pub(super) fn review_action() -> CompareAction {
    CompareAction::ReviewApply { word: WORD.into() }
}

pub(super) fn reviewing(state: AppState) -> AppState {
    let (state, cmds) = act(state, review_action());
    assert!(cmds.is_empty(), "opening a review writes nothing");
    assert!(state.compare.apply.review().is_some(), "review is open");
    state
}

pub(super) fn applies(cmds: &[Command]) -> Vec<&CompareCommand> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Compare(c @ CompareCommand::Apply { .. }) => Some(c),
            _ => None,
        })
        .collect()
}

/// The review of `with_pending`; a replace needs no typing.
pub(super) fn confirmed() -> AppState {
    reviewing(with_pending())
}

/// A confirmed review that has been sent: the apply of run 2 is running.
pub(super) fn running_apply() -> AppState {
    let (state, cmds) = act(confirmed(), CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1);
    state
}

/// Both only-in-source and only-in-target rows exist; all of each are queued for deletion,
/// which writes to two databases.
fn wide_deletes() -> AppState {
    let both = Summary {
        only_in_source: 3,
        only_in_target: 3,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, both));
    let (state, _) = update(state, rows_of(Category::OnlyInSource, 3));
    let (state, _) = update(state, run_done());
    let (state, _) = act(state, CompareAction::SelectAllInCategory);
    let delete_source = CompareAction::AddToPending(RowAction::DeleteFromSource);
    let (state, _) = act(state, delete_source);
    let (state, _) = act(state, CompareAction::SelectCategory(Category::OnlyInTarget));
    let (state, _) = update(state, rows_of(Category::OnlyInTarget, 3));
    let (state, _) = act(state, CompareAction::SelectAllInCategory);
    let delete_target = CompareAction::AddToPending(RowAction::DeleteFromTarget);
    let (state, _) = act(state, delete_target);
    assert_eq!(state.compare.pending.items().len(), 2);
    state
}

#[test]
fn review_is_a_snapshot_of_names_counts_and_warnings() {
    let state = reviewing(with_pending());
    let review = state.compare.apply.review().unwrap();
    assert_eq!(review.pairs.len(), 1);
    assert_eq!(review.pairs[0].pair, RUN_PAIR);
    assert_eq!(review.pairs[0].counts.replace, 2);
    let (action, dest) = &review.pairs[0].destinations[0];
    assert_eq!(*action, RowAction::CopyToTarget);
    assert_eq!(dest.connection, "b");
    assert_eq!(dest.database, "b");
    assert_eq!(dest.collection, "users");
    assert_eq!(review.id_policy, IdPolicy::Keep);
    assert!(review.required.is_none());
    assert!(review.confirmation_passes(), "a replace needs no typing");
    assert_eq!(review.session, 1);
    assert_eq!(review.word, WORD);
}

#[test]
fn deletes_need_the_destination_collection_typed() {
    let state = reviewing(with_delete_pending());
    let review = state.compare.apply.review().unwrap();
    assert_eq!(review.pairs[0].counts.delete, 2);
    assert_eq!(review.required_text(&review.word), Some("users"));
    assert!(!review.confirmation_passes());
    let (state, _) = act(state, CompareAction::SetConfirmText(" users ".into()));
    assert!(state.compare.apply.review().unwrap().confirmation_passes());
    let (state, _) = act(state, CompareAction::SetConfirmText("Users".into()));
    assert!(!state.compare.apply.review().unwrap().confirmation_passes());
}

#[test]
fn deletes_across_databases_need_the_word_the_view_handed_in() {
    let state = reviewing(wide_deletes());
    let review = state.compare.apply.review().unwrap();
    assert_eq!(review.required, Some(RequiredText::Word));
    assert!(!review.confirmation_passes());
    let (state, _) = act(state, CompareAction::SetConfirmText("delete".into()));
    let (_, cmds) = act(copy_of(&state), CompareAction::ConfirmApply);
    assert!(
        applies(&cmds).is_empty(),
        "the default English is not the view's word"
    );
    let (state, _) = act(state, CompareAction::SetConfirmText(WORD.into()));
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1);
}

#[test]
fn an_empty_word_can_never_confirm_a_word_delete() {
    let state = wide_deletes();
    let (state, _) = act(
        state,
        CompareAction::ReviewApply {
            word: String::new(),
        },
    );
    let (state, _) = act(state, CompareAction::SetConfirmText(String::new()));
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty());
}

#[test]
fn review_is_refused_when_the_gates_fail() {
    // A run still active.
    let active = toggled(tab(Category::Different, 3, false), &[0]);
    let (active, _) = act(active, CompareAction::AddToPending(RowAction::CopyToTarget));
    assert!(active.compare.is_running() && !active.compare.pending.is_empty());
    let (after, cmds) = act(active, review_action());
    assert!(cmds.is_empty() && after.compare.apply == ApplyUi::Idle);
    // A check in flight.
    let (checking, _) = act(with_pending(), CompareAction::RunCheck);
    assert!(checking.compare.check_in_flight());
    let (after, _) = act(checking, review_action());
    assert_eq!(after.compare.apply, ApplyUi::Idle);
    // Nothing pending.
    let (after, _) = act(tab_of(Category::Different), review_action());
    assert_eq!(after.compare.apply, ApplyUi::Idle);
    // A pair an earlier apply touched.
    let mut stale = with_pending();
    stale.compare.overview = stale.compare.overview.clone().marked_applied(RUN_PAIR);
    let (after, _) = act(stale, review_action());
    assert_eq!(after.compare.apply, ApplyUi::Idle);
    // No session.
    let (closed, _) = act(with_pending(), CompareAction::CloseSession);
    let (after, _) = act(closed, review_action());
    assert_eq!(after.compare.apply, ApplyUi::Idle);
}

#[test]
fn review_of_a_gone_connection_is_refused_with_a_banner() {
    let mut state = with_pending();
    state.conn.connections.truncate(1);
    let (state, cmds) = act(state, review_action());
    assert!(cmds.is_empty());
    assert_eq!(state.compare.apply, ApplyUi::Idle);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_apply",
            ..
        })
    ));
}

#[test]
fn id_policy_and_typing_only_work_while_reviewing() {
    let generate = CompareAction::SetIdPolicy(IdPolicy::Generate);
    let (idle, _) = act(with_pending(), generate.clone());
    assert_eq!(idle.compare.apply, ApplyUi::Idle);
    let (idle, _) = act(idle, CompareAction::SetConfirmText("x".into()));
    assert_eq!(idle.compare.apply, ApplyUi::Idle);
    let (state, _) = act(confirmed(), generate);
    let review = state.compare.apply.review().unwrap();
    assert_eq!(review.id_policy, IdPolicy::Generate);
}

#[test]
fn confirm_sends_one_apply_with_the_pending_list_and_a_fresh_run() {
    let (state, cmds) = act(confirmed(), CompareAction::ConfirmApply);
    assert_eq!(cmds.len(), 1);
    let [
        CompareCommand::Apply {
            session,
            run,
            ops,
            bulk,
            id_policy,
        },
    ] = applies(&cmds)[..]
    else {
        panic!("one apply")
    };
    assert_eq!((*session, *run, *id_policy), (1, 2, IdPolicy::Keep));
    assert!(bulk.is_empty());
    let want: Vec<_> = (0..2)
        .map(|i| {
            (
                RUN_PAIR,
                key(i),
                ApplyKind::Replace,
                Direction::SourceToTarget,
            )
        })
        .collect();
    let got: Vec<_> = ops
        .iter()
        .map(|o| (o.pair, o.key.clone(), o.kind, o.direction))
        .collect();
    assert_eq!(got, want);
    assert_eq!(state.compare.next_run, 3);
    assert!(matches!(
        state.compare.apply,
        ApplyUi::Running {
            run: 2,
            total: 2,
            cancel_requested: false,
            ..
        }
    ));
    assert!(!state.compare.pending.is_empty(), "kept until ApplyDone");
}

#[test]
fn a_whole_category_goes_out_as_a_bulk_item_and_the_id_policy_is_sent() {
    let (state, _) = act(
        tab_of(Category::OnlyInSource),
        CompareAction::SelectAllInCategory,
    );
    let copy = CompareAction::AddToPending(RowAction::CopyToTarget);
    let (state, _) = act(state, copy);
    let (state, _) = act(
        reviewing(state),
        CompareAction::SetIdPolicy(IdPolicy::Generate),
    );
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    let [
        CompareCommand::Apply {
            ops,
            bulk,
            id_policy,
            ..
        },
    ] = applies(&cmds)[..]
    else {
        panic!("one apply")
    };
    assert!(ops.is_empty());
    assert_eq!(*id_policy, IdPolicy::Generate);
    assert_eq!(bulk.len(), 1);
    assert_eq!(bulk[0].kind, ApplyKind::Insert);
    assert_eq!(bulk[0].category, Category::OnlyInSource);
}

#[test]
fn a_double_confirm_sends_exactly_one_apply() {
    let (state, first) = act(confirmed(), CompareAction::ConfirmApply);
    let (state, second) = act(state, CompareAction::ConfirmApply);
    assert_eq!(applies(&first).len(), 1);
    assert!(second.is_empty());
    assert_eq!(state.compare.next_run, 3, "no second run id taken");
}

#[test]
fn wrong_or_missing_typed_text_sends_nothing() {
    let state = reviewing(with_delete_pending());
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(cmds.is_empty(), "nothing typed");
    assert!(state.compare.apply.review().is_some(), "dialog stays open");
    let (state, _) = act(state, CompareAction::SetConfirmText("user".into()));
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(cmds.is_empty());
    let (state, _) = act(state, CompareAction::SetConfirmText("users".into()));
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1);
}

#[test]
fn typing_alone_never_sends() {
    let state = reviewing(with_delete_pending());
    let (_, cmds) = act(state, CompareAction::SetConfirmText("users".into()));
    assert!(cmds.is_empty());
}

#[test]
fn confirm_after_the_world_changed_sends_nothing_and_goes_idle() {
    let changes: [fn(&mut AppState); 5] = [
        |s| s.compare.pending = s.compare.pending.cleared(),
        |s| s.compare.overview = s.compare.overview.clone().marked_applied(RUN_PAIR),
        |s| s.compare.overview = s.compare.overview.clone().discarded(RUN_PAIR),
        |s| s.compare.session = Some(SessionPhase::Ready(9)),
        |s| s.compare.session = Some(SessionPhase::Opening(1)),
    ];
    for change in changes {
        let mut state = confirmed();
        change(&mut state);
        let (state, cmds) = act(state, CompareAction::ConfirmApply);
        assert!(applies(&cmds).is_empty());
        assert_eq!(state.compare.apply, ApplyUi::Idle);
    }
}

#[test]
fn confirm_after_the_counts_changed_sends_nothing() {
    let mut state = confirmed();
    let (pending, _) = state.compare.pending.with_action(
        RUN_PAIR,
        Category::Different,
        RowAction::CopyToSource,
        Scope::Keys(vec![key(2)]),
    );
    state.compare.pending = pending;
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(cmds.is_empty());
    assert_eq!(state.compare.apply, ApplyUi::Idle);
}

#[test]
fn confirm_after_a_session_replacement_sends_nothing() {
    let state = confirmed();
    let (a, b) = state.compare.connections.unwrap();
    let (state, _) = act(state, CompareAction::CloseSession);
    assert_eq!(state.compare.apply, ApplyUi::Idle);
    let (state, _) = act(state, open_action(a, b));
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(cmds.is_empty());
}

#[test]
fn a_review_bound_to_another_session_is_not_confirmed() {
    let mut state = confirmed();
    let ApplyUi::Reviewing(review) = state.compare.apply.clone() else {
        panic!("reviewing")
    };
    state.compare.apply = ApplyUi::Reviewing(review.bound(7, WORD));
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(applies(&cmds).is_empty());
    assert_eq!(state.compare.apply, ApplyUi::Idle);
}

#[test]
fn cancel_review_and_back_work() {
    let (state, cmds) = act(confirmed(), CompareAction::CancelReview);
    assert!(cmds.is_empty() && state.compare.apply == ApplyUi::Idle);
    let (state, _) = act(confirmed(), CompareAction::Back);
    assert!(
        state.compare.apply.review().is_some(),
        "Back leaves the dialog"
    );
}

#[test]
fn cancel_apply_sends_one_cancel_for_the_running_apply() {
    let (running, cmds) = act(running_apply(), CompareAction::CancelApply);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Cancel {
            session: 1,
            run: 2
        })]
    ));
    let (running, again) = act(running, CompareAction::CancelApply);
    assert!(again.is_empty());
    assert!(matches!(
        running.compare.apply,
        ApplyUi::Running {
            cancel_requested: true,
            ..
        }
    ));
    let (_, none) = act(confirmed(), CompareAction::CancelApply);
    assert!(none.is_empty(), "nothing runs while reviewing");
}
