//! The apply workflow of the Compare reducer: review, confirmation, cancel and the apply events.
//!
//! `confirm_apply` is the only place that builds `CompareCommand::Apply`, and it is reached only
//! from the `ConfirmApply` action. It sends nothing unless a review is open, its typed
//! confirmation passes against the word stored in the review, and the state still allows the
//! review and would produce the very same writes now. Anything else drops the review.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::{AppState, Banner};
use crate::viewmodel::compare::{ApplyUi, CompareState, Review};
use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::protocol::{CompareCommand, CompareEvent, PairId, Side};
use curlybrace_core::error::CoreError;
use uuid::Uuid;

fn with_apply(state: AppState, apply: ApplyUi) -> AppState {
    let compare = CompareState {
        apply,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// Opens the review dialog for the pending list. The connection names are read now, from the
/// app's connections; a connection that is gone refuses the review with a banner (the dialog
/// must name where the writes land), and so does a pending list that writes to a read-only
/// connection. `word` is the localized confirmation word.
pub(super) fn review(state: AppState, word: &str) -> Out {
    let (Some(session), Some((source, target))) = (ready(&state), state.compare.connections) else {
        return none(state);
    };
    if !state.compare.can_review() {
        return none(state);
    }
    if let Some(side) = state.compare.read_only_destination(&state.conn.connections) {
        return none(read_only_refusal(state, side));
    }
    let names = (
        state.connection(source).map(|c| c.name.clone()),
        state.connection(target).map(|c| c.name.clone()),
    );
    let (Some(source_name), Some(target_name)) = names else {
        return none(failure_banner(state, "compare_apply", CoreError::NotFound));
    };
    let name_of = |id: Uuid| match id == source {
        true => source_name.clone(),
        false => target_name.clone(),
    };
    match state.compare.review(IdPolicy::default(), &name_of) {
        Some(found) => none(with_apply(
            state,
            ApplyUi::reviewing(found.bound(session, word)),
        )),
        None => none(state),
    }
}

pub(super) fn set_confirm_text(state: AppState, text: &str) -> Out {
    let apply = state.compare.apply.clone().confirm_typed(text);
    none(with_apply(state, apply))
}

pub(super) fn set_id_policy(state: AppState, policy: IdPolicy) -> Out {
    let apply = match state.compare.apply.clone() {
        ApplyUi::Reviewing(review) => ApplyUi::Reviewing(review.with_id_policy(policy)),
        other => other,
    };
    none(with_apply(state, apply))
}

pub(super) fn cancel_review(state: AppState) -> Out {
    match state.compare.apply.review() {
        Some(_) => none(with_apply(state, ApplyUi::Idle)),
        None => none(state),
    }
}

/// The banner that says which read-only side refused the apply.
fn read_only_refusal(state: AppState, side: Side) -> AppState {
    let banner = match side {
        Side::Target => Banner::CompareApplyReadOnly,
        Side::Source => Banner::CompareApplyReadOnlySource,
    };
    AppState {
        banner: Some(banner),
        ..state
    }
}

/// Reviewing is over without a write: the dialog closes.
fn dropped_review(state: AppState) -> Out {
    none(with_apply(state, ApplyUi::Idle))
}

/// The review as it would be built right now, with the apply idle (a review in progress blocks
/// itself). Display names are not compared, only what is written where.
fn rebuilt(c: &CompareState, reviewed: &Review) -> Option<Review> {
    let idle = CompareState {
        apply: ApplyUi::Idle,
        ..c.clone()
    };
    idle.review(reviewed.id_policy, &|_| String::new())
        .filter(|now| now.same_writes(reviewed))
}

/// The only constructor of `CompareCommand::Apply`.
///
/// A review whose session is gone or whose writes would differ now is dropped (back to idle,
/// nothing sent); one whose typed confirmation does not pass yet stays open, nothing sent.
pub(super) fn confirm_apply(state: AppState) -> Out {
    let c = &state.compare;
    let Some(reviewed) = c.apply.review() else {
        return none(state);
    };
    if ready(&state) != Some(reviewed.session) || rebuilt(c, reviewed).is_none() {
        return dropped_review(state);
    }
    // A read-only connection never receives a write, whichever side it is: any queued change
    // that lands on one refuses the whole apply before anything is sent.
    if let Some(side) = c.read_only_destination(&state.conn.connections) {
        let (state, out) = dropped_review(state);
        return (read_only_refusal(state, side), out);
    }
    if !reviewed.confirmation_passes() {
        return none(state);
    }
    // The reviewed list, which `rebuilt` has just proven equal to the current one.
    let (ops, bulk) = reviewed.pending.to_command_parts();
    let run = c.next_run;
    let command = CompareCommand::Apply {
        session: reviewed.session,
        run,
        ops,
        bulk,
        id_policy: reviewed.id_policy,
    };
    let compare = CompareState {
        apply: c.apply.clone().started(run),
        next_run: run + 1,
        ..c.clone()
    };
    (with_compare(state, compare), vec![cmd(command)])
}

/// Sends Cancel once for the running apply.
pub(super) fn cancel_apply(state: AppState) -> Out {
    let c = &state.compare;
    let (Some(session), Some(run)) = (ready(&state), c.apply.running_run()) else {
        return none(state);
    };
    if !c.apply.can_cancel() {
        return none(state);
    }
    let apply = c.apply.clone().cancelling();
    (
        with_apply(state, apply),
        vec![cmd(CompareCommand::Cancel { session, run })],
    )
}

/// Closes the report; a review is closed with `cancel_review` and a running apply stays.
pub(super) fn dismiss_result(state: AppState) -> Out {
    match state.compare.apply.is_finished() {
        true => none(with_apply(state, ApplyUi::Idle)),
        false => none(state),
    }
}

/// Events of the running apply. `on_event` has checked the session; the apply state ignores
/// every other run, and what it ignores changes nothing else either.
pub(super) fn on_event(state: AppState, ev: CompareEvent) -> Out {
    let before = state.compare.apply.clone();
    let apply = match ev {
        CompareEvent::ApplyProgress {
            run,
            pair,
            done,
            total,
            ..
        } => before.clone().progressed(run, pair, done, total),
        CompareEvent::ApplyPairDone {
            run, pair, report, ..
        } => before.clone().pair_done(run, pair, report),
        CompareEvent::ApplyDone { run, .. } => before.clone().apply_done(run),
        _ => return none(state),
    };
    if apply == before {
        return none(state);
    }
    let touched = touched_pairs(&apply);
    let next = with_apply(state, apply);
    none(mark_applied(next, &touched))
}

/// The pairs that reported a write (or a failed one) in the apply state: their results are stale.
fn touched_pairs(apply: &ApplyUi) -> Vec<PairId> {
    match apply {
        ApplyUi::Running { rows, .. } => rows
            .iter()
            .filter(|r| {
                r.report
                    .as_ref()
                    .is_some_and(|rep| rep.applied > 0 || rep.failed > 0)
            })
            .map(|r| r.pair)
            .collect(),
        finished => finished.touched_pairs(),
    }
}

/// The rows of `pairs` are applied: the overview marks them and their ticks and queued writes go.
fn mark_applied(state: AppState, pairs: &[PairId]) -> AppState {
    let c = &state.compare;
    let compare = CompareState {
        overview: pairs
            .iter()
            .fold(c.overview.clone(), |o, pair| o.marked_applied(*pair)),
        selection: pairs
            .iter()
            .fold(c.selection.clone(), |s, pair| s.without_pair(*pair)),
        pending: pairs
            .iter()
            .fold(c.pending.clone(), |p, pair| p.without_pair(*pair)),
        ..c.clone()
    };
    with_compare(state, compare)
}

/// `Apply` was refused as a whole (it carries no run): the banner shows it and the workflow is
/// idle again with the pending list kept. A failure while nothing runs only raises the banner.
pub(super) fn failed(state: AppState, error: CoreError) -> Out {
    let next = match state.compare.apply.is_running() {
        true => with_apply(state, ApplyUi::Idle),
        false => state,
    };
    none(failure_banner(next, "compare_apply", error))
}
