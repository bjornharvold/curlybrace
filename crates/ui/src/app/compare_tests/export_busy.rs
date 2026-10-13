//! The busy rules: while the file dialog is open or an export runs, the session and the results
//! are held like during an apply.
use super::confirm::{WORD, key, toggled};
use super::export::{choosing, done_event, plan_modal, run_of, running_clipboard, running_plan};
use super::*;
use crate::viewmodel::compare::{ExportKind, ExportUi};
use curlybrace_core::compare::protocol::ExportOutcome;

/// The actions the apply-busy rule refuses; each can fire from `control_baseline` except those
/// listed in `cannot_fire_in_the_fixture`.
fn refused_while_busy() -> Vec<CompareAction> {
    vec![
        CompareAction::Start,
        CompareAction::StartPassing,
        CompareAction::RunCheck,
        CompareAction::Rerun(ENTRY),
        CompareAction::RemovePair(ENTRY),
        CompareAction::DiscardPair(ENTRY),
        CompareAction::FindDuplicates(ENTRY),
        CompareAction::ReviewApply { word: WORD.into() },
        CompareAction::ToggleRow(key(2)),
        CompareAction::SelectAllInCategory,
        CompareAction::AddToPending(RowAction::CopyToSource),
        CompareAction::RemovePending(0),
        CompareAction::ClearPending,
        CompareAction::CloseSession,
        CompareAction::OpenExport(ExportKind::Plan),
        CompareAction::OpenExport(ExportKind::Report),
    ]
}

/// Never fires in the fixtures: FindDuplicates needs a run that failed on a duplicate key (its
/// own tests cover the apply-busy refusal); the two OpenExport actions are ignored while a
/// modal is open (controlled from the closed baseline below).
fn cannot_fire_in_the_fixture(action: &CompareAction) -> bool {
    matches!(
        action,
        CompareAction::FindDuplicates(_) | CompareAction::OpenExport(_)
    )
}

/// The plan modal with row 2 ticked: nothing is busy, every refused action can act here.
fn control_baseline() -> AppState {
    toggled(plan_modal(), &[2])
}

fn snapshot(state: &AppState) -> String {
    format!("{:?}", state.compare)
}

fn assert_refused(state: AppState, action: CompareAction, what: &str) {
    let before = snapshot(&state);
    let (after, cmds) = act(state, action.clone());
    assert!(cmds.is_empty(), "{action:?} sent {cmds:?} {what}");
    assert_eq!(snapshot(&after), before, "{action:?} changed state {what}");
}

#[test]
fn choosing_and_running_refuse_the_apply_busy_set() {
    type Build = (&'static str, fn() -> AppState);
    let builds: [Build; 3] = [
        ("choosing a plan", || choosing(control_baseline())),
        ("running a plan", || {
            act(
                choosing(control_baseline()),
                CompareAction::ExportPathChosen(super::export::out_path()),
            )
            .0
        }),
        ("running a clipboard report", running_clipboard),
    ];
    for (what, build) in builds {
        for index in 0..action_count() {
            // Fresh fixture per action: connection ids are random per fixture.
            let state = build();
            let action = actions_for(&state).remove(index);
            assert_refused(state, action, what);
        }
    }
}

fn action_count() -> usize {
    refused_while_busy().len() + 1
}

/// The refused set, plus opening the session for the swapped connections.
fn actions_for(state: &AppState) -> Vec<CompareAction> {
    let (source, target) = state.compare.connections.expect("a session");
    let mut all = refused_while_busy();
    all.push(open_action(target, source));
    all
}

#[test]
fn the_refused_actions_fire_from_the_non_busy_baseline() {
    for index in 0..action_count() {
        let state = control_baseline();
        let action = actions_for(&state).remove(index);
        if cannot_fire_in_the_fixture(&action) {
            continue;
        }
        let before = snapshot(&state);
        let (after, cmds) = act(state, action.clone());
        assert!(
            !cmds.is_empty() || snapshot(&after) != before,
            "{action:?} does nothing even when idle"
        );
    }
}

#[test]
fn opening_an_export_fires_again_from_a_closed_result() {
    let finished = running_plan_finished();
    let (idle, _) = act(finished, CompareAction::CloseExport);
    for kind in [ExportKind::Plan, ExportKind::Report] {
        let (open, _) = act(copy_of(&idle), CompareAction::OpenExport(kind));
        assert!(
            matches!(open.compare.export, ExportUi::Configuring(_)),
            "{kind:?}"
        );
    }
}

#[test]
fn the_same_actions_work_again_once_the_export_is_closed() {
    let state = running_plan_finished();
    let (state, _) = act(state, CompareAction::CloseExport);
    let (queued, _) = act(copy_of(&state), CompareAction::RemovePending(0));
    assert!(queued.compare.pending.is_empty(), "remove works again");
    let (state, cmds) = act(state, CompareAction::CloseSession);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Close { session: 1 })]
    ));
    assert!(state.compare.session.is_none());
    let (chosen_back, _) = act(choosing(plan_modal()), CompareAction::ExportPathCancelled);
    assert!(!chosen_back.compare.pending.is_empty());
    let (cleared, _) = act(chosen_back, CompareAction::ClearPending);
    assert!(
        cleared.compare.pending.is_empty(),
        "the action works again after the dialog is cancelled"
    );
}

#[test]
fn closing_the_session_drops_the_export_state() {
    for state in [plan_modal(), running_plan_finished()] {
        let (state, cmds) = act(state, CompareAction::CloseSession);
        assert_eq!(cmds.len(), 1);
        assert_eq!(state.compare.export, ExportUi::Idle);
    }
}

fn running_plan_finished() -> AppState {
    let state = running_plan();
    let run = run_of(&state);
    update(state, done_event(run, ExportOutcome::default())).0
}
