//! The write gate: no sequence of user actions or events sends `Apply` except `ConfirmApply`
//! from a review whose confirmation passes.
use super::confirm::{
    WORD, applies, confirmed, key, review_action, running_apply, with_delete_pending, with_pending,
};
use super::*;
use crate::viewmodel::compare::{ApplyUi, RowAction};
use curlybrace_core::compare::apply::{ApplyReport, IdPolicy};

/// One arm per variant and no wildcard: a new `CompareAction` must be added here, and to
/// `all_actions`, before this file compiles and passes.
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
    }
}

const VARIANTS: usize = 46;

fn all_actions() -> Vec<CompareAction> {
    use CompareAction as A;
    let (a, b) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
    vec![
        A::Show,
        A::Hide,
        A::Open {
            source: a,
            target: b,
        },
        A::CloseSession,
        A::PickSide(Side::Source, Pick::Database("a".into())),
        A::SetKey("_id".into()),
        A::SetFilter(Side::Source, "{}".into()),
        A::SetRules(RulesForm::default()),
        A::AddPair,
        A::AddAllSameNamed,
        A::AddUnmatched {
            side: Side::Source,
            name: "x".into(),
            as_name: "y".into(),
        },
        A::RemovePair(ENTRY),
        A::IncludePair(ENTRY, true),
        A::EditPair(Some(ENTRY)),
        A::SetPairOverrides(ENTRY, PairOverrides::default()),
        A::ToggleSystemCollections(true),
        A::RunCheck,
        A::Start,
        A::StartPassing,
        A::Cancel,
        A::SelectCategory(Category::Different),
        A::Page(1),
        A::Select(key(0)),
        A::OpenPair(ENTRY),
        A::BackToOverview,
        A::DiscardPair(ENTRY),
        A::Rerun(ENTRY),
        A::Back,
        A::Reload(Side::Source),
        A::RetryRows,
        A::FindDuplicates(ENTRY),
        A::ToggleRow(key(0)),
        A::SelectPage,
        A::SelectAllInCategory,
        A::ClearSelection,
        A::AddToPending(RowAction::CopyToTarget),
        A::AddKeyToPending(key(2), RowAction::CopyToSource),
        A::RemovePending(0),
        A::ClearPending,
        A::ReviewApply { word: WORD.into() },
        A::SetConfirmText(String::new()),
        A::SetIdPolicy(IdPolicy::Generate),
        A::ConfirmApply,
        A::CancelReview,
        A::CancelApply,
        A::DismissApplyResult,
    ]
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

/// A named state to start from; `passing` tells whether `ConfirmApply` may send there.
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
        ("running", running_apply, false),
        ("finished", finished_apply, false),
        ("idle, nothing pending", || checked_state(), false),
    ]
}

#[test]
fn every_action_has_a_slot_and_an_instance() {
    let slots: std::collections::BTreeSet<usize> = all_actions().iter().map(slot).collect();
    assert_eq!(all_actions().len(), VARIANTS);
    assert_eq!(slots.len(), VARIANTS, "one instance per variant");
    assert_eq!(slots.into_iter().max(), Some(VARIANTS - 1));
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
