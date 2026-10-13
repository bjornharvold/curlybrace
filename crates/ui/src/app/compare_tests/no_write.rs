//! The write gate: no sequence of user actions or events sends `Apply` except `ConfirmApply`
//! from a review whose confirmation passes.
use super::confirm::{
    WORD, applies, confirmed, key, review_action, running_apply, tab, with_delete_pending,
    with_pending,
};
use super::*;
use crate::viewmodel::compare::{ApplyUi, ExportKind, RowAction};
use curlybrace_core::compare::apply::{ApplyReport, IdPolicy};
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::protocol::ExportFormat;
use std::path::PathBuf;
use uuid::Uuid;

/// The instance of the variant that follows `action`, `None` after the last one. The match has
/// one arm per variant and no wildcard, so a new `CompareAction` does not compile until it is
/// given an arm here; `all_actions` walks the chain from the first variant.
fn successor(action: &CompareAction) -> Option<CompareAction> {
    use CompareAction as A;
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    Some(match action {
        A::Show => A::Hide,
        A::Hide => A::Open {
            source: a,
            target: b,
        },
        A::Open { .. } => A::CloseSession,
        A::CloseSession => A::PickSide(Side::Source, Pick::Database("a".into())),
        A::PickSide(..) => A::SetKey("_id".into()),
        A::SetKey(_) => A::SetFilter(Side::Source, "{}".into()),
        A::SetFilter(..) => A::SetRules(RulesForm::default()),
        A::SetRules(_) => A::AddPair,
        A::AddPair => A::AddAllSameNamed,
        A::AddAllSameNamed => A::AddUnmatched {
            side: Side::Source,
            name: "x".into(),
            as_name: "y".into(),
        },
        A::AddUnmatched { .. } => A::RemovePair(ENTRY),
        A::RemovePair(_) => A::IncludePair(ENTRY, true),
        A::IncludePair(..) => A::EditPair(Some(ENTRY)),
        A::EditPair(_) => A::SetPairOverrides(ENTRY, PairOverrides::default()),
        A::SetPairOverrides(..) => A::ToggleSystemCollections(true),
        A::ToggleSystemCollections(_) => A::RunCheck,
        A::RunCheck => A::Start,
        A::Start => A::StartPassing,
        A::StartPassing => A::Cancel,
        A::Cancel => A::SelectCategory(Category::Different),
        A::SelectCategory(_) => A::Page(1),
        A::Page(_) => A::Select(key(0)),
        A::Select(_) => A::OpenPair(ENTRY),
        A::OpenPair(_) => A::BackToOverview,
        A::BackToOverview => A::DiscardPair(ENTRY),
        A::DiscardPair(_) => A::Rerun(ENTRY),
        A::Rerun(_) => A::Back,
        A::Back => A::Reload(Side::Source),
        A::Reload(_) => A::RetryRows,
        A::RetryRows => A::FindDuplicates(ENTRY),
        A::FindDuplicates(_) => A::ToggleRow(key(0)),
        A::ToggleRow(_) => A::SelectPage,
        A::SelectPage => A::SelectAllInCategory,
        A::SelectAllInCategory => A::ClearSelection,
        A::ClearSelection => A::AddToPending(RowAction::CopyToTarget),
        A::AddToPending(_) => A::AddKeyToPending(key(2), RowAction::CopyToSource),
        A::AddKeyToPending(..) => A::RemovePending(0),
        A::RemovePending(_) => A::ClearPending,
        A::ClearPending => A::ToggleField(vec![PathSegment::Field("a".into())]),
        A::ToggleField(_) => A::ClearFieldSelection,
        A::ClearFieldSelection => A::AddFieldsToPending(RowAction::CopyFieldsToTarget),
        A::AddFieldsToPending(_) => A::ReviewApply { word: WORD.into() },
        A::ReviewApply { .. } => A::SetConfirmText(String::new()),
        A::SetConfirmText(_) => A::SetIdPolicy(IdPolicy::Generate),
        A::SetIdPolicy(_) => A::ConfirmApply,
        A::ConfirmApply => A::CancelReview,
        A::CancelReview => A::CancelApply,
        A::CancelApply => A::DismissApplyResult,
        A::DismissApplyResult => A::OpenExport(ExportKind::Plan),
        A::OpenExport(_) => A::SetExportFormat(ExportFormat::JsonPlan),
        A::SetExportFormat(_) => A::SetExportLabel("label".into()),
        A::SetExportLabel(_) => A::SetExportIdPolicy(IdPolicy::Generate),
        A::SetExportIdPolicy(_) => A::ToggleExportPair(ENTRY),
        A::ToggleExportPair(_) => A::SetExportIncludePaths(true),
        A::SetExportIncludePaths(_) => A::ChooseExportFile,
        A::ChooseExportFile => A::ExportPathChosen(PathBuf::from("/exports/out")),
        A::ExportPathChosen(_) => A::ExportPathCancelled,
        A::ExportPathCancelled => A::CopyReport,
        A::CopyReport => A::CancelExport,
        A::CancelExport => A::CloseExport,
        A::CloseExport => return None,
    })
}

/// The position of a variant: one arm per variant, no wildcard. The walk must visit exactly the
/// slots `0..=max`, so a mislinked `successor` or an unreachable new variant fails.
fn slot(action: &CompareAction) -> usize {
    use CompareAction as A;
    match action {
        A::Show => 0,
        A::Hide => 1,
        A::Open { .. } => 2,
        A::CloseSession => 3,
        A::PickSide(..) => 4,
        A::SetKey(_) => 5,
        A::SetFilter(..) => 6,
        A::SetRules(_) => 7,
        A::AddPair => 8,
        A::AddAllSameNamed => 9,
        A::AddUnmatched { .. } => 10,
        A::RemovePair(_) => 11,
        A::IncludePair(..) => 12,
        A::EditPair(_) => 13,
        A::SetPairOverrides(..) => 14,
        A::ToggleSystemCollections(_) => 15,
        A::RunCheck => 16,
        A::Start => 17,
        A::StartPassing => 18,
        A::Cancel => 19,
        A::SelectCategory(_) => 20,
        A::Page(_) => 21,
        A::Select(_) => 22,
        A::OpenPair(_) => 23,
        A::BackToOverview => 24,
        A::DiscardPair(_) => 25,
        A::Rerun(_) => 26,
        A::Back => 27,
        A::Reload(_) => 28,
        A::RetryRows => 29,
        A::FindDuplicates(_) => 30,
        A::ToggleRow(_) => 31,
        A::SelectPage => 32,
        A::SelectAllInCategory => 33,
        A::ClearSelection => 34,
        A::AddToPending(_) => 35,
        A::AddKeyToPending(..) => 36,
        A::RemovePending(_) => 37,
        A::ClearPending => 38,
        A::ReviewApply { .. } => 39,
        A::SetConfirmText(_) => 40,
        A::SetIdPolicy(_) => 41,
        A::ConfirmApply => 42,
        A::CancelReview => 43,
        A::CancelApply => 44,
        A::DismissApplyResult => 45,
        A::ToggleField(_) => 46,
        A::ClearFieldSelection => 47,
        A::AddFieldsToPending(_) => 48,
        A::OpenExport(_) => 49,
        A::SetExportFormat(_) => 50,
        A::SetExportLabel(_) => 51,
        A::SetExportIdPolicy(_) => 52,
        A::ToggleExportPair(_) => 53,
        A::SetExportIncludePaths(_) => 54,
        A::ChooseExportFile => 55,
        A::ExportPathChosen(_) => 56,
        A::ExportPathCancelled => 57,
        A::CopyReport => 58,
        A::CancelExport => 59,
        A::CloseExport => 60,
    }
}

pub(super) fn all_actions() -> Vec<CompareAction> {
    std::iter::successors(Some(CompareAction::Show), successor).collect()
}

fn finished_apply() -> AppState {
    let report = ApplyReport {
        applied: 2,
        ..ApplyReport::default()
    };
    let pair_done = ev(CompareEvent::ApplyPairDone {
        session: 1,
        run: 2,
        pair: RUN_PAIR,
        report,
    });
    let (state, _) = update(running_apply(), pair_done);
    update(state, ev(CompareEvent::ApplyDone { session: 1, run: 2 })).0
}

/// A review (from `confirmed`) shown while a compare run is active.
fn reviewing_during_a_run() -> AppState {
    let review = confirmed().compare.apply;
    let mut state = with_pending_during_run();
    state.compare.apply = review;
    state
}

fn with_pending_during_run() -> AppState {
    let ticked = super::confirm::toggled(tab(Category::Different, 3, false), &[0, 1]);
    let queue = CompareAction::AddToPending(RowAction::CopyToTarget);
    let (state, _) = act(ticked, queue);
    assert!(state.compare.is_running() && !state.compare.pending.is_empty());
    state
}

/// A review whose session is still opening.
fn reviewing_while_opening() -> AppState {
    let mut state = confirmed();
    state.compare.session = Some(SessionPhase::Opening(1));
    state
}

/// (name, builder, whether `ConfirmApply` may send there)
type StartState = (&'static str, fn() -> AppState, bool);

fn states() -> Vec<StartState> {
    vec![
        ("idle with pending", with_pending, false),
        ("reviewing, confirmation passes", confirmed, true),
        (
            "reviewing, confirmation not typed",
            || super::confirm::reviewing(with_delete_pending()),
            false,
        ),
        ("reviewing during a run", reviewing_during_a_run, false),
        ("reviewing while opening", reviewing_while_opening, false),
        ("running", running_apply, false),
        ("finished", finished_apply, false),
        ("idle, nothing pending", checked_state, false),
        ("detail open, fields ticked", super::fields::ticked, false),
        (
            "reviewing a fields item",
            super::fields::reviewing_queued,
            true,
        ),
    ]
}

#[test]
fn the_walk_visits_every_variant_once() {
    let all = all_actions();
    let mut slots: Vec<usize> = all.iter().map(slot).collect();
    slots.sort_unstable();
    let max = *slots.last().unwrap();
    assert_eq!(
        slots,
        (0..=max).collect::<Vec<_>>(),
        "gap or repeat in the walk"
    );
    for (i, a) in all.iter().enumerate() {
        assert!(!all[..i].contains(a), "{a:?} twice");
    }
    assert_eq!(all.first(), Some(&CompareAction::Show));
    assert!(all.contains(&CompareAction::ConfirmApply));
    assert_eq!(all.last(), Some(&CompareAction::CloseExport));
}

#[test]
fn only_a_passing_confirm_from_a_review_sends_apply_for_every_action_in_every_state() {
    for (name, build, passing) in states() {
        for action in all_actions() {
            let (state, cmds) = act(build(), action.clone());
            let sent = applies(&cmds).len();
            let allowed = action == CompareAction::ConfirmApply && passing;
            assert_eq!(sent, usize::from(allowed), "{action:?} in {name}");
            if allowed {
                assert!(state.compare.apply.is_running());
            }
        }
    }
}

#[test]
fn no_sequence_of_actions_without_confirm_sends_apply() {
    let forward: Vec<CompareAction> = all_actions()
        .into_iter()
        .filter(|a| *a != CompareAction::ConfirmApply)
        .collect();
    let backward: Vec<CompareAction> = forward.iter().rev().cloned().collect();
    for (name, build, _) in states() {
        for order in [&forward, &backward] {
            let mut state = build();
            for action in order.iter() {
                let (next, cmds) = act(state, action.clone());
                assert!(applies(&cmds).is_empty(), "{action:?} after {name}");
                state = next;
            }
        }
    }
}

/// The walk above closes the detail before the field actions run, so they also run here, with a
/// loaded detail and ticks, in several orders and from states where a write is under review.
#[test]
fn the_field_actions_with_a_loaded_detail_never_send_apply() {
    let a = || PathSegment::Field("a".into());
    let toggle = || CompareAction::ToggleField(vec![a()]);
    let queue_t = || CompareAction::AddFieldsToPending(RowAction::CopyFieldsToTarget);
    let queue_s = || CompareAction::AddFieldsToPending(RowAction::CopyFieldsToSource);
    let orders = [
        vec![
            toggle(),
            queue_t(),
            toggle(),
            queue_s(),
            CompareAction::ClearFieldSelection,
        ],
        vec![
            queue_s(),
            CompareAction::ClearFieldSelection,
            toggle(),
            queue_t(),
            toggle(),
        ],
    ];
    let starts: [fn() -> AppState; 2] = [super::fields::ticked, super::fields::reviewing_queued];
    for build in starts {
        for order in &orders {
            let mut state = build();
            for action in order {
                let (next, cmds) = act(state, action.clone());
                assert!(applies(&cmds).is_empty(), "{action:?}");
                state = next;
            }
        }
    }
}

#[test]
fn no_runtime_event_sends_apply() {
    let report = ApplyReport {
        applied: 1,
        ..ApplyReport::default()
    };
    let events = vec![
        CompareEvent::ApplyProgress {
            session: 1,
            run: 2,
            pair: RUN_PAIR,
            done: 1,
            total: 2,
        },
        CompareEvent::ApplyPairDone {
            session: 1,
            run: 2,
            pair: RUN_PAIR,
            report,
        },
        CompareEvent::ApplyDone { session: 1, run: 2 },
        CompareEvent::Failed {
            session: 1,
            context: "compare_apply",
            error: CoreError::NotFound,
        },
        CompareEvent::RunDone { session: 1, run: 1 },
        CompareEvent::Opened { session: 1 },
    ];
    for (name, build, _) in states() {
        for event in &events {
            let (_, cmds) = update(build(), ev(event.clone()));
            assert!(applies(&cmds).is_empty(), "{event:?} in {name}");
        }
    }
}

#[test]
fn a_review_that_was_cancelled_cannot_be_confirmed_later() {
    let (state, _) = act(confirmed(), CompareAction::CancelReview);
    let (state, cmds) = act(state, CompareAction::ConfirmApply);
    assert!(cmds.is_empty() && state.compare.apply == ApplyUi::Idle);
    let (state, _) = act(state, review_action());
    let (_, cmds) = act(state, CompareAction::ConfirmApply);
    assert_eq!(applies(&cmds).len(), 1, "a fresh review confirms again");
}
