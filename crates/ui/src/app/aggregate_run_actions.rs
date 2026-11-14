//! The Aggregate tab's confirmed write run: Run opens a typed-confirmation dialog, confirming it
//! sends the one `AggregateRun`, and the answer becomes the run's status. A run is a write: it is
//! refused on a read-only connection, and on a production connection the dialog IS the
//! confirmation, so it never also raises the production warning (`Guard::Confirmed`, used here
//! and only here for this command).
use super::actions::{gated, mutation};
use super::aggregate_actions::{AggregateAction, with_draft};
use super::reducer::Out;
use super::state::{AppState, Banner, ConnStatus};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::aggregate_run::{RunAvailability, RunDialog, RunEnv, RunStatus, RunUi};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::run_pipeline;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;

impl AppState {
    /// What the Run button of the active session's draft needs to know about its surroundings.
    pub fn run_env(&self) -> RunEnv {
        RunEnv {
            connected: self.status == ConnStatus::Connected,
            has_collection: self.selected_ns().is_some(),
            read_only: self.active_is_read_only(),
        }
    }

    /// Whether Run is shown and usable for the active session's draft.
    pub fn run_availability(&self) -> RunAvailability {
        self.aggregate.can_run(self.run_env())
    }
}

/// Whether `action` is one of the run actions.
pub(super) fn is_run_action(action: &AggregateAction) -> bool {
    matches!(
        action,
        AggregateAction::RunClicked
            | AggregateAction::RunTyped(_)
            | AggregateAction::RunConfirmed
            | AggregateAction::RunDismissed
    )
}

pub(super) fn on_action(state: AppState, action: AggregateAction) -> Out {
    match action {
        AggregateAction::RunClicked => gated(state, clicked),
        AggregateAction::RunTyped(text) => (typed(state, text), Vec::new()),
        AggregateAction::RunConfirmed => confirmed(state),
        AggregateAction::RunDismissed => (dialog_closed(state), Vec::new()),
        _ => (state, Vec::new()),
    }
}

/// `state` with the active session's run dialog closed.
pub(super) fn dialog_closed(state: AppState) -> AppState {
    let run = state.aggregate.run.clone().dialog_closed();
    with_run(state, run)
}

fn with_run(state: AppState, run: RunUi) -> AppState {
    let draft = (*state.aggregate).clone().with_run(run);
    with_draft(state, draft)
}

/// Run was clicked: opens the dialog for the pipeline's target and asks for the target's count.
/// Nothing opens on a read-only connection (the banner says why), without a usable pipeline,
/// while a run or another write is in flight, or while a write warning is open.
fn clicked(state: AppState) -> Out {
    if state.active_is_read_only() {
        let banner = Some(Banner::ReadOnly);
        return (AppState { banner, ..state }, Vec::new());
    }
    let (Some(source), true) = (
        state.selected_ns().cloned(),
        state.run_availability() == RunAvailability::Ready,
    ) else {
        return (state, Vec::new());
    };
    let busy = state.pending.is_some() || state.write_warning.is_some();
    if busy || state.aggregate.run.dialog.is_some() {
        return (state, Vec::new());
    }
    let Ok(plan) = run_pipeline(&state.aggregate.stages) else {
        return (state, Vec::new());
    };
    let dialog = RunDialog {
        target: plan.target,
        stage: plan.stage,
        source,
        typed: String::new(),
        target_count: None,
    };
    let count = Command::EstimatedCount {
        session: state.session_id(),
        ns: dialog.target_ns(),
    };
    let run = state.aggregate.run.clone().opened(dialog);
    (with_run(state, run), vec![count])
}

fn typed(state: AppState, text: String) -> AppState {
    let run = state.aggregate.run.clone().typed(text);
    with_run(state, run)
}

/// Confirm was pressed. Sends the run only when everything still holds: the typed name matches
/// the target, the open collection is the one the dialog was opened for, the pipeline still has
/// the target the dialog shows, and the connection is writable. A connection locked meanwhile
/// is refused with the banner and closes the dialog.
fn confirmed(state: AppState) -> Out {
    let Some(dialog) = state.aggregate.run.dialog.clone() else {
        return (state, Vec::new());
    };
    if state.active_is_read_only() {
        let banner = Some(Banner::ReadOnly);
        return (dialog_closed(AppState { banner, ..state }), Vec::new());
    }
    let same_source = state.selected_ns() == Some(&dialog.source);
    let plan = run_pipeline(&state.aggregate.stages)
        .ok()
        .filter(|p| p.target == dialog.target && p.stage == dialog.stage);
    let Some(plan) = plan.filter(|_| same_source && dialog.is_confirmed()) else {
        return (state, Vec::new());
    };
    let run = state.aggregate.run.run.wrapping_add(1);
    let (next, commands) = mutation(
        state,
        None,
        WriteKind::AggregateRun,
        Guard::Confirmed,
        |session, ns| Command::AggregateRun {
            session,
            ns,
            pipeline: plan.pipeline,
            run,
        },
    );
    match commands.is_empty() {
        true => (next, commands),
        false => {
            let ui = next.aggregate.run.clone().started(run);
            (with_run(next, ui), commands)
        }
    }
}

/// One count answers two askers, the index builder and the run dialog; each takes only the count
/// of the collection it asked for.
pub(super) fn estimate_arrived(state: AppState, ns: Namespace, count: u64) -> AppState {
    let state = match state.aggregate.run.dialog {
        Some(_) => {
            let run = state.aggregate.run.clone().count_arrived(&ns, count);
            with_run(state, run)
        }
        None => state,
    };
    super::index_write_actions::estimate_arrived(state, ns, count)
}

/// Whether `ev` answers a run and is not a lost server (that one takes the connection-lost path,
/// which marks the run's outcome unknown): an `AggregateRan`, or a failed `aggregate_run`.
pub(super) fn is_run_event(ev: &Event) -> bool {
    match ev {
        Event::AggregateRan { .. } => true,
        Event::Failed { context, error, .. } => {
            *context == "aggregate_run"
                && !matches!(
                    error,
                    CoreError::Unreachable | CoreError::TlsHandshake | CoreError::Superseded
                )
        }
        _ => false,
    }
}

/// A run event of its session, active or not (the event's own session is the active one while it
/// applies). A closed or disconnected session drops the late answer.
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    state.with_session(session, |s| match (s.status == ConnStatus::Connected, ev) {
        (
            true,
            Event::AggregateRan {
                run,
                target,
                outcome,
                ..
            },
        ) => ran(s, run, target, outcome),
        (true, Event::Failed { error, .. }) => command_failed(s, error),
        _ => (s, Vec::new()),
    })
}

/// The answer of run `run`. A stale one (another number, or no run in flight) changes nothing.
/// The write is over either way, so the guard on further writes is lifted. A target that is not
/// in the sidebar yet makes the sidebar list again.
fn ran(
    state: AppState,
    run: u64,
    target: Namespace,
    outcome: Result<Option<u64>, CoreError>,
) -> Out {
    let ui = &state.aggregate.run;
    if run != ui.run || !ui.is_running() {
        return (state, Vec::new());
    }
    let (status, wrote) = match outcome {
        Ok(count) => (
            RunStatus::Done {
                target: target.clone(),
                count,
            },
            true,
        ),
        Err(CoreError::WriteOutcomeUnknown) => (RunStatus::Unknown, false),
        Err(error) => (RunStatus::Failed(error), false),
    };
    let next = settled(state, run, status);
    match wrote && !listed(&next, &target) {
        true => super::admin_actions::changed(next, target.db),
        false => (next, Vec::new()),
    }
}

/// A run that failed as a command: unknown when the write may have gone through, a refusal on a
/// connection locked meanwhile (banner), a failure shown with the run otherwise.
fn command_failed(state: AppState, error: CoreError) -> Out {
    if !state.aggregate.run.is_running() {
        return (state, Vec::new());
    }
    let run = state.aggregate.run.run;
    let next = match error {
        CoreError::ReadOnly => AppState {
            banner: Some(Banner::ReadOnly),
            ..settled(state, run, RunStatus::Idle)
        },
        CoreError::WriteOutcomeUnknown => settled(state, run, RunStatus::Unknown),
        error => settled(state, run, RunStatus::Failed(error)),
    };
    (next, Vec::new())
}

/// The server was lost (editor open, so the session stays) while a run was in flight: the write
/// may have gone through, so its outcome is unknown.
pub(super) fn lost(state: AppState, error: &CoreError) -> AppState {
    let run = state.aggregate.run.clone().command_failed(error.clone());
    AppState {
        pending: None,
        ..with_run(state, run)
    }
}

/// `status` for run `run`, with the write marked finished.
fn settled(state: AppState, run: u64, status: RunStatus) -> AppState {
    let ui = state.aggregate.run.clone().answered(run, status);
    AppState {
        pending: None,
        ..with_run(state, ui)
    }
}

/// Whether the sidebar already lists `target` (a database whose collections are not loaded is
/// listed on its own expansion, so it is not asked again).
fn listed(state: &AppState, target: &Namespace) -> bool {
    state
        .sidebar
        .databases
        .iter()
        .find(|d| d.name == target.db)
        .is_some_and(|d| {
            d.collections
                .as_ref()
                .is_none_or(|c| c.contains(&target.collection))
        })
}

#[cfg(test)]
#[path = "aggregate_run_actions_tests.rs"]
mod tests;
