//! The Shell tab's index-miss guard: before a Run sends its script, the literal queries the
//! script holds are planned by the server, and a plan that would scan a collection (or sort in
//! memory) is shown as the `IndexMiss` notice instead of running. Like the query bar's guard it
//! is advice that fails open: whenever the check cannot answer, the run goes on as without it.
//!
//! The check comes after the syntax check and the connection and tool checks, and before the
//! production confirmation. Its request number comes from `AppState::guard_request`, shared
//! with the query bar; `guard_actions` offers each answer to the query bar first, then here.
use super::explain_actions::error_variant;
use super::guard_actions::worth_checking;
use super::reducer::Out;
use super::shell_actions::{notice, send};
use super::shell_preflight::usable;
use super::state::AppState;
use crate::viewmodel::shell::{ShellNotice, ShellState};
use crate::viewmodel::shell_queries::{ShellCall, read_calls};
use curlybrace_core::error::CoreError;
use curlybrace_core::guard::{GUARD_MAX_TARGETS, GUARD_MIN_DOCS, GuardReport, GuardTarget};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use std::ops::Range;
use uuid::Uuid;

/// The database `mongosh` starts in when the connection string names none.
const MONGOSH_DEFAULT_DB: &str = "test";

/// What the guard made of a Run.
pub(super) enum Guarded {
    /// The check was sent and the shell waits for it: the state and the command to send.
    Checking(Out),
    /// Nothing to check (guard off, or no query worth checking): run as without a guard.
    Clear(AppState),
}

/// A Run of `range` of `shell`'s script (`None`: all of it) that passed every earlier check.
/// With the guard on and at least one readable query worth checking, sends the check (the first
/// `GUARD_MAX_TARGETS` queries) and holds the run back; otherwise leaves it to run. Calls the
/// reader could not read never hold a run back on their own.
pub(super) fn check_first(
    state: AppState,
    id: Uuid,
    shell: &ShellState,
    range: Option<Range<usize>>,
) -> Guarded {
    if !state.guard_enabled() {
        return Guarded::Clear(state);
    }
    let start = start_db(state.connection(id).map(|c| c.uri.as_str()));
    let reading = read_calls(shell.text_of(&range), &start);
    let worth: Vec<ShellCall> = reading
        .calls
        .into_iter()
        .filter(|call| worth_checking(&call.query))
        .collect();
    if worth.is_empty() {
        return Guarded::Clear(state);
    }
    let beyond = worth.len().saturating_sub(GUARD_MAX_TARGETS);
    let targets: Vec<GuardTarget> = worth
        .into_iter()
        .take(GUARD_MAX_TARGETS)
        .map(|call| GuardTarget {
            ns: Namespace::new(call.db, call.collection),
            method: call.method.to_string(),
            query: call.query,
        })
        .collect();
    let request = state.guard_request + 1;
    // A production connection warns whatever the collection's size, as in the query bar.
    let min_docs = (!state.session_production(id)).then_some(GUARD_MIN_DOCS);
    let command = Command::GuardCheck {
        session: id,
        request,
        targets,
        min_docs,
    };
    let waiting = shell.checking(request, range, reading.unchecked.saturating_add(beyond));
    let next = AppState {
        guard_request: request,
        ..state.with_shell(id, waiting)
    };
    Guarded::Checking((next, vec![command]))
}

/// `Event::GuardChecked` (or a guard check that could not start, as `Err`) the query bar did
/// not await. It belongs to the active session's shell when that shell waits for `request` (the
/// reducer makes the event's session the active one); any other answer is stale and dropped.
/// `background`: that session is not the one the person is looking at.
pub(super) fn answered(
    state: AppState,
    request: u64,
    outcome: Result<GuardReport, CoreError>,
    background: bool,
) -> Out {
    let Some(id) = state.active else {
        return (state, Vec::new());
    };
    let Some(shell) = state.shell.get(id).filter(|s| s.awaits_check(request)) else {
        return (state, Vec::new());
    };
    let shell = shell.clone();
    match outcome {
        // A guard turned off since the check was sent means "run as without a guard".
        Ok(report) if !report.findings.is_empty() && state.guard_enabled() => {
            (state.with_shell(id, shell.index_missed(report)), Vec::new())
        }
        Ok(_) => go_on(state, id, &shell, background),
        Err(error) => {
            log::warn!(
                "shell index guard check unavailable: {}",
                error_variant(&error)
            );
            go_on(state, id, &shell, background)
        }
    }
}

/// The check let the run go on: the run is sent as a Run would send it, through every guard
/// that can have changed since (connection, read-only, `mongosh`) and the production warning.
fn go_on(state: AppState, id: Uuid, shell: &ShellState, background: bool) -> Out {
    let (idle, pending) = shell.check_settled();
    let state = state.with_shell(id, idle.clone());
    // A production run is confirmed only from the session the person is looking at (the
    // confirmation shows and checks the active one): a background session's run is not started.
    if background && state.session_production(id) {
        return notice(state, id, ShellNotice::GuardNotStartedBackground);
    }
    // Only a production run needs the warning, which holds one write at a time: when another
    // write took it while this check ran, it keeps it and the shell says the run did not start.
    if state.write_warning.is_some() && state.session_production(id) {
        return notice(state, id, ShellNotice::GuardNotStarted);
    }
    match usable(&state, id) {
        Ok(exe) => send(state, id, &idle, exe, pending.ran),
        Err(refusal) => notice(state, id, refusal),
    }
}

/// The guard was turned off: every Shell tab's index notice closes (the person can Run again).
pub(super) fn notices_closed(state: AppState) -> AppState {
    let tabs = Box::new((*state.shell).clone().each(ShellState::index_notice_closed));
    AppState {
        shell: tabs,
        ..state
    }
}

/// The database a script starts in, as `mongosh` opens it: the connection string's path (the
/// part after the hosts, before any options), percent-decoded, else `test`.
fn start_db(uri: Option<&str>) -> String {
    let path = uri.map(uri_path).unwrap_or_default();
    let name = decoded(path).unwrap_or_else(|| path.to_owned());
    match name.is_empty() {
        true => MONGOSH_DEFAULT_DB.to_owned(),
        false => name,
    }
}

/// The path of a `mongodb://` or `mongodb+srv://` URI, split the way the shell's own
/// connection string is built (`shell::invocation`).
fn uri_path(uri: &str) -> &str {
    let rest = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let head = rest.split_once('?').map_or(rest, |(head, _)| head);
    head.split_once('/').map_or("", |(_, path)| path)
}

/// `text` with its `%XX` escapes decoded, or `None` when an escape is broken or the bytes are
/// not UTF-8.
fn decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = text.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod start_db_tests {
    use super::start_db;

    #[test]
    fn the_start_database_is_the_uri_path_or_test() {
        let cases = [
            (None, "test"),
            (Some("mongodb://localhost:27017"), "test"),
            (Some("mongodb://localhost:27017/"), "test"),
            (Some("mongodb://localhost:27017/?tls=true"), "test"),
            (Some("mongodb://u@a:1,b:2/shop?replicaSet=r"), "shop"),
            (Some("mongodb+srv://c.example.net/app"), "app"),
            (Some("mongodb://h/my%2Ddb"), "my-db"),
            (Some("mongodb://h/bad%zz"), "bad%zz"),
            (Some("mongodb://h/cut%2"), "cut%2"),
        ];
        for (uri, db) in cases {
            assert_eq!(start_db(uri), db, "{uri:?}");
        }
    }
}

#[cfg(test)]
#[path = "shell_guard_tests.rs"]
mod shell_guard_tests;

#[cfg(test)]
#[path = "shell_guard_drop_tests.rs"]
mod shell_guard_drop_tests;

#[cfg(test)]
#[path = "shell_guard_order_tests.rs"]
mod shell_guard_order_tests;

#[cfg(test)]
#[path = "shell_guard_switch_tests.rs"]
mod shell_guard_switch_tests;

#[cfg(test)]
#[path = "shell_guard_background_tests.rs"]
mod shell_guard_background_tests;
