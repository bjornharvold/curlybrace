//! Field-level selection and pending changes of the Compare reducer (slice 2c, ruling 8). Ticking
//! a field and queueing the ticked fields only change view-model state: nothing here sends a
//! runtime command, so a field write still reaches `CompareCommand::Apply` only through the review
//! and `ConfirmApply`.
//!
//! All of it acts on the LOADED detail of the open tab's pair, under the gate of the whole-document
//! queue actions (`open_tab`: no apply reviewing or running, the pair completed and not applied).
//! The ticks belong to the detail they were made in: whatever closes or replaces the detail
//! drops them (`fields_dropped`), and nothing here writes a tick from an event.
use super::compare::{none, with_compare};
use super::compare_select::open_tab;
use super::reducer::Out;
use super::state::{AppState, Banner};
use crate::viewmodel::compare::{
    ChangeLine, CompareState, DetailView, FieldSelection, FieldsQueued, RowAction,
};
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;

const DIRECTIONS: [Direction; 2] = [Direction::SourceToTarget, Direction::TargetToSource];

/// The detail whose fields may be ticked and queued right now.
struct Editable<'a> {
    pair: PairId,
    detail: &'a DetailView,
}

/// The loaded detail of a row of the open `Different` tab, if the tab accepts changes at all.
/// Only rows that exist on both sides have fields to copy.
fn editable(c: &CompareState) -> Option<Editable<'_>> {
    let tab = open_tab(c).filter(|t| t.cat == Category::Different)?;
    let detail = c
        .detail
        .as_ref()
        .filter(|d| !d.loading && tab.has_row(&d.key))?;
    Some(Editable {
        pair: tab.pair,
        detail,
    })
}

fn copyable(line: &ChangeLine, direction: Direction) -> bool {
    line.copy_block(direction).is_none()
}

/// The detail's change at exactly `path`.
fn line_at<'a>(detail: &'a DetailView, path: &[PathSegment]) -> Option<&'a ChangeLine> {
    detail.changes.iter().find(|l| l.segments == path)
}

fn with_field_selection(state: AppState, field_selection: FieldSelection) -> Out {
    let compare = CompareState {
        field_selection,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// Ticks or unticks one changed field of the open detail. A path the detail does not list, or
/// one that cannot be copied in either direction, is ignored (the direction is checked per path
/// when queueing).
pub(super) fn toggle_field(state: AppState, path: Vec<PathSegment>) -> Out {
    let c = &state.compare;
    let Some(editable) = editable(c) else {
        return none(state);
    };
    let tickable = line_at(editable.detail, &path)
        .is_some_and(|line| DIRECTIONS.iter().any(|d| copyable(line, *d)));
    if !tickable {
        return none(state);
    }
    let selection = c
        .field_selection
        .toggled(editable.pair, &editable.detail.key, &path);
    with_field_selection(state, selection)
}

/// Unticks every field of the open detail.
pub(super) fn clear_fields(state: AppState) -> Out {
    let c = &state.compare;
    let Some(editable) = editable(c) else {
        return none(state);
    };
    let selection = c
        .field_selection
        .cleared(editable.pair, &editable.detail.key);
    with_field_selection(state, selection)
}

/// The ticked paths that are listed by the detail and can be copied in `direction`.
fn copyable_ticks(
    c: &CompareState,
    editable: &Editable,
    direction: Direction,
) -> Vec<Vec<PathSegment>> {
    c.field_selection
        .paths(editable.pair, &editable.detail.key)
        .into_iter()
        .filter(|path| line_at(editable.detail, path).is_some_and(|l| copyable(l, direction)))
        .collect()
}

fn refused(state: AppState, banner: Banner) -> Out {
    none(AppState {
        banner: Some(banner),
        ..state
    })
}

/// Queues the ticked fields of the open detail for `action` (`CopyFieldsToTarget` or
/// `CopyFieldsToSource`; any other action is ignored) and clears that document's ticks. Only
/// the ticked paths that can be copied in the action's direction are queued; with none, nothing
/// happens. An overlap of paths, or a whole-category item that already covers the tab, leaves
/// the list and the ticks as they were and raises a banner.
pub(super) fn add_fields(state: AppState, action: RowAction) -> Out {
    let c = &state.compare;
    if !RowAction::offered_for_fields().contains(&action) {
        return none(state);
    }
    let Some(editable) = editable(c) else {
        return none(state);
    };
    let paths = copyable_ticks(c, &editable, action.direction());
    if paths.is_empty() {
        return none(state);
    }
    let key = &editable.detail.key;
    let (pending, outcome) = c.pending.with_fields(editable.pair, key, action, paths);
    match outcome {
        FieldsQueued::Queued | FieldsQueued::Merged => {
            let compare = CompareState {
                pending,
                field_selection: c.field_selection.cleared(editable.pair, key),
                ..c.clone()
            };
            none(with_compare(state, compare))
        }
        FieldsQueued::Conflict => refused(state, Banner::FieldsConflict),
        FieldsQueued::Swallowed => refused(state, Banner::FieldsCovered),
        FieldsQueued::Empty => none(state),
    }
}

/// The state without the ticks of the open pair: call it, on the state as it was, wherever the
/// detail is closed or replaced (the ticks are only meaningful for the detail they were made in).
pub(super) fn fields_dropped(c: CompareState) -> CompareState {
    let field_selection = match c.open_pair {
        Some(pair) => c.field_selection.without_pair(pair),
        None => c.field_selection.clone(),
    };
    CompareState {
        field_selection,
        ..c
    }
}
