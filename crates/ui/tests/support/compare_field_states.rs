//! States for the field-level apply tests: the finished pair of `compare_apply_states` with a
//! detail of several changed fields open, ticked and queued through the real reducer.
use super::compare_apply_states::{
    ONLY_IN_TARGET, PAIR, act, finished, key, reviewing, rows_of, step,
};
use bson::{Bson, doc};
use curlybrace_core::compare::diff::{ChangeKind, FieldChange, PathSegment};
use curlybrace_core::compare::protocol::CompareEvent;
use curlybrace_core::compare::results::Category;
use curlybrace_core::runtime::Event;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::RowAction;

/// The path texts of the fixture's changes, in order.
pub const NAME: &str = "name";
pub const CITY: &str = "address.city";
pub const DOLLAR: &str = "$x";
pub const DOTTED: &str = "a.b";
pub const ANY: &str = "tags.[]";

pub fn field(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

pub fn path_of(text: &str) -> Vec<PathSegment> {
    match text {
        CITY => vec![field("address"), field("city")],
        ANY => vec![field("tags"), PathSegment::AnyElement],
        other => vec![field(other)],
    }
}

fn change(text: &str, kind: ChangeKind) -> FieldChange {
    FieldChange {
        path: text.into(),
        segments: path_of(text),
        kind,
        source: Some(Bson::String("x".into())),
        target: Some(Bson::String("y".into())),
    }
}

/// Two addressable changes, a `$`-named field, a field named with a dot and an element of an
/// array whose order is ignored (the last three cannot be copied on their own).
pub fn changes() -> Vec<FieldChange> {
    vec![
        change(NAME, ChangeKind::Changed),
        change(CITY, ChangeKind::Changed),
        change(DOLLAR, ChangeKind::Changed),
        change(DOTTED, ChangeKind::Changed),
        change(ANY, ChangeKind::Added),
    ]
}

/// Key 1 of the Different tab opened, with the detail answered.
pub fn with_field_detail() -> AppState {
    let state = act(finished(), CompareAction::Select(key(1)));
    let detail = CompareEvent::Detail {
        session: 1,
        pair: PAIR,
        key: key(1),
        source: Some(doc! {"_id": 1, "name": "x"}),
        target: Some(doc! {"_id": 1, "name": "y"}),
        changes: changes(),
    };
    step(state, Input::Event(Event::Compare(detail)))
}

pub fn tick(state: AppState, text: &str) -> AppState {
    act(state, CompareAction::ToggleField(path_of(text)))
}

/// The detail with `name` and `address.city` ticked.
pub fn ticked() -> AppState {
    tick(tick(with_field_detail(), NAME), CITY)
}

/// `name` and `address.city` queued for a copy to the target (a Fields item).
pub fn with_fields_pending() -> AppState {
    let state = act(
        ticked(),
        CompareAction::AddFieldsToPending(RowAction::CopyFieldsToTarget),
    );
    assert_eq!(state.compare.pending.items().len(), 1, "queued");
    state
}

/// The same, with a delete of an only-in-target row next to it.
pub fn with_fields_and_delete_pending() -> AppState {
    let state = act(
        with_fields_pending(),
        CompareAction::SelectCategory(Category::OnlyInTarget),
    );
    let state = rows_of(state, Category::OnlyInTarget, ONLY_IN_TARGET);
    let state = act(state, CompareAction::ToggleRow(key(1)));
    act(
        state,
        CompareAction::AddToPending(RowAction::DeleteFromTarget),
    )
}

/// The review dialog over a field-only pending list.
pub fn reviewing_fields() -> AppState {
    reviewing(with_fields_pending())
}
