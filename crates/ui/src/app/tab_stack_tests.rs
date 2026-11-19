//! The reducer moves `AppState` by value through every step, and its deepest event path is near
//! the debug-build stack limit (see `reducer::event_then_close_stale`). Routing an answer to a
//! parked tab adds a swap on that path; these tests run the deepest paths, with one tab and with
//! two (the parked one owning the answer), on a thread with a bounded stack: a stack overflow
//! aborts the whole test binary, so passing is the proof.
//!
//! The bound is Rust's default 2 MiB test-thread stack, the smallest the app's own threads see in
//! a debug build. Only the event runs on the bounded thread; the scenario that leads up to it is
//! built on the test's own. `STACK_KIB` shrinks the bound (a measurement aid, to find how much
//! room a path needs: `STACK_KIB=1024 cargo test ... tab_routing::stack_tests`).
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use crate::app::tabs::TabId;
use crate::app::{AggregateAction, CountsAction};
use crate::viewmodel::{CollectionTab, DocAction, SidebarAction};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// The default test-thread stack, unless `STACK_KIB` says otherwise.
fn stack_bytes() -> usize {
    std::env::var("STACK_KIB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2048)
        * 1024
}

/// A state handed to the bounded thread and back. The thread is joined before the test goes on, so
/// nothing is ever shared: the state only changes hands.
struct Handoff(AppState);
// SAFETY: moved into one thread, which the caller joins; no other reference exists meanwhile.
unsafe impl Send for Handoff {}

/// Applies `input` to `state` on a thread whose stack is `stack_bytes()` long.
fn on_bounded_stack(state: AppState, input: Input) -> AppState {
    on_stack_of(state, input, stack_bytes())
}

/// `kib` KiB, or `STACK_KIB` when set (to find the real minimum of a bounded test).
fn bound_kib(kib: usize) -> usize {
    std::env::var("STACK_KIB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(kib)
        * 1024
}

/// Applies `input` to `state` on a thread whose stack is `bytes` long.
fn on_stack_of(state: AppState, input: Input, bytes: usize) -> AppState {
    let moved = Handoff(state);
    let input = AssertSend(input);
    std::thread::Builder::new()
        .name("bounded-stack".into())
        .stack_size(bytes)
        .spawn(move || {
            let (moved, input) = (moved, input);
            Handoff(update(moved.0, input.0).0)
        })
        .expect("a thread")
        .join()
        .expect("the event ran to its end")
        .0
}

struct AssertSend(Input);
// SAFETY: as for `Handoff`.
unsafe impl Send for AssertSend {}

fn act(s: AppState, tab: Option<TabId>, input: Input) -> (AppState, Vec<Command>) {
    match tab {
        Some(tab) => s.with_tab(tab, |s| update(s, input)),
        None => update(s, input),
    }
}

/// Session `a` connected on `app.users`, an Aggregate draft with a per-stage count in flight.
/// With `parked` the count belongs to a second, parked tab; otherwise to the session's one tab.
/// With `editors`, an editor is open in the tabs.
fn counting(parked: bool, editors: bool) -> (AppState, Uuid, u64) {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let a = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(a));
    let epoch = s.epoch_counter;
    let (s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: a,
            epoch,
            topology: None,
        }),
    );
    let users = Namespace::new("app", "users");
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(users.clone())));
    let (s, tab) = match parked {
        true => {
            let (s, second) = s.tab_opened(a);
            let second = second.expect("a second tab");
            let (s, _) = act(s, Some(second), Input::Sidebar(SidebarAction::Open(users)));
            (s, Some(second))
        }
        false => (s, None),
    };
    let (s, _) = act(s, tab, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        tab,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (s, commands) = act(
        s,
        tab,
        Input::Aggregate(AggregateAction::Counts(CountsAction::Start)),
    );
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::AggregateCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a count in flight");
    let s = match editors {
        true => {
            let (s, _) = act(s, tab, Input::Doc(DocAction::Insert));
            update(s, Input::Doc(DocAction::Insert)).0
        }
        false => s,
    };
    (s, a, request)
}

fn count_lost(session: Uuid, request: u64) -> Input {
    Input::Event(Event::RequestFailed {
        session,
        context: "aggregate_count",
        request,
        error: CoreError::Unreachable,
    })
}

fn find_lost(session: Uuid, request: u64, numbered: bool) -> Input {
    Input::Event(match numbered {
        true => Event::RequestFailed {
            session,
            context: "find",
            request,
            error: CoreError::Unreachable,
        },
        false => Event::Failed {
            session,
            context: "find",
            error: CoreError::Unreachable,
        },
    })
}

#[test]
fn a_lost_server_during_a_count_with_editors_open_fits_the_stack_with_one_tab() {
    let (s, a, request) = counting(false, true);
    let s = on_bounded_stack(s, count_lost(a, request));
    // One transient error with an editor open keeps the session; the point is the depth.
    assert_eq!(s.status, ConnStatus::Connected);
}

#[test]
fn a_lost_server_during_a_parked_tabs_count_with_editors_open_fits_the_stack() {
    let (s, a, request) = counting(true, true);
    let s = on_bounded_stack(s, count_lost(a, request));
    assert_eq!(s.status, ConnStatus::Connected);
}

#[test]
fn a_lost_server_during_a_count_fails_the_root_within_the_stack_with_one_tab() {
    let (s, a, request) = counting(false, false);
    let s = on_bounded_stack(s, count_lost(a, request));
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_lost_server_during_a_parked_tabs_count_fails_the_root_within_the_stack() {
    let (s, a, request) = counting(true, false);
    let s = on_bounded_stack(s, count_lost(a, request));
    assert_eq!(
        s.status,
        ConnStatus::Failed,
        "the lost server is seen for a parked tab too"
    );
}

#[test]
fn a_bare_lost_find_fails_the_root_within_the_stack_with_one_tab() {
    let (s, a, _) = counting(false, false);
    let s = on_bounded_stack(s, find_lost(a, 0, false));
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_numbered_lost_find_fails_the_root_within_the_stack_with_one_tab() {
    let (s, a, _) = counting(false, false);
    let request = s.issued_request;
    let s = on_bounded_stack(s, find_lost(a, request, true));
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_lost_find_of_a_parked_tab_fails_the_root_within_the_stack() {
    let (s, a, _) = counting(true, false);
    let second = *s.parked_tabs.keys().next().expect("a parked tab");
    let request = s.parked_tabs[&second].issued_request;
    let s = on_bounded_stack(s, find_lost(a, request, true));
    assert_eq!(s.status, ConnStatus::Failed);
}

// Measured margins (debug build, bisected with `STACK_KIB`, 16 KiB steps): the event-only minimum
// of a find lost on a parked tab is 1576 KiB, of a bare find lost on a one-tab session 1428 KiB, of
// a count lost on a parked tab 1768 KiB (the unchanged code needed 1560, 1428 and 1768). The
// bounds below sit about 200 KiB above the first two, so a change that costs a frame of any size
// on these paths fails here before it overflows a test thread.

#[test]
fn a_lost_find_of_a_parked_tab_stays_within_its_measured_bound() {
    let (s, a, _) = counting(true, false);
    let second = *s.parked_tabs.keys().next().expect("a parked tab");
    let request = s.parked_tabs[&second].issued_request;
    let s = on_stack_of(s, find_lost(a, request, true), bound_kib(1792));
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_bare_lost_find_with_one_tab_stays_within_its_measured_bound() {
    let (s, a, _) = counting(false, false);
    let s = on_stack_of(s, find_lost(a, 0, false), bound_kib(1536));
    assert_eq!(s.status, ConnStatus::Failed);
}

/// A state whose parked tab has an insert in flight (its editor is open), and the session.
fn writing_in_a_parked_tab() -> (AppState, Uuid) {
    use crate::viewmodel::EditorAction;
    let (s, a, _) = counting(true, false);
    let second = *s.parked_tabs.keys().next().expect("a parked tab");
    let (s, _) = act(s, Some(second), Input::Doc(DocAction::Insert));
    let text = s.parked_tabs[&second]
        .editor
        .as_ref()
        .expect("an editor")
        .text
        .clone();
    let (s, commands) = act(
        s,
        Some(second),
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(commands.as_slice(), [Command::Insert { .. }]));
    (s, a)
}

fn insert_failed(session: Uuid, error: CoreError) -> Input {
    Input::Event(Event::Failed {
        session,
        context: "insert",
        error,
    })
}

// The write of a parked tab fails through `with_tab` (the answer follows `Pending::tab`): a lost
// server with the editor open (the session stays) needs 1548 KiB event-only, a refusal 1484 KiB;
// the bound is 1792 KiB, like the parked find (1576 KiB).

#[test]
fn a_lost_write_of_a_parked_tab_stays_within_its_measured_bound() {
    let (s, a) = writing_in_a_parked_tab();
    let s = on_stack_of(s, insert_failed(a, CoreError::Unreachable), bound_kib(1792));
    assert_eq!(
        s.status,
        ConnStatus::Connected,
        "the editor keeps the session"
    );
}

#[test]
fn a_refused_write_of_a_parked_tab_stays_within_its_measured_bound() {
    let (s, a) = writing_in_a_parked_tab();
    let s = on_stack_of(
        s,
        insert_failed(a, CoreError::Database("no".into())),
        bound_kib(1792),
    );
    assert_eq!(s.status, ConnStatus::Connected);
}

#[test]
fn the_state_the_reducer_moves_by_value_stays_the_size_it_was() {
    // Every reducer frame copies it: a new field that is not boxed shows here before it shows as
    // a stack overflow in a debug build. It was 10.9 KiB before the tab model, 11.1 KiB with it.
    let size = std::mem::size_of::<AppState>();
    assert!(size < 12 * 1024, "AppState is {size} bytes");
}
