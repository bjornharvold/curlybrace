//! `Command::ShellRun` / `Command::ShellCancel`: running `mongosh` for the Shell tab.
//!
//! Everything that decides a refusal (session, connection, read-only, the one-run slot) happens
//! synchronously in [`dispatch`], so request order decides which of two runs holds the slot. Only
//! the keychain read is awaited, in a task. The runner's callbacks run on its supervisor thread and
//! send on the runtime's (unbounded, so synchronous) event channel.
//!
//! Locks: the session's shell slot is taken only to check, insert or clear, never around
//! `spawn_shell`, whose `on_done` may run before it returns and clears the slot itself.

use std::future::Future;
use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::clock;
use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::history::{HistoryDb, ShellRunStatus};
use crate::shell::{Outcome, RunLimits, build_invocation, percent_encode_component, spawn_shell};

use super::indexes::blocking;
use super::session::Session;
use super::{Command, Event, Repaint, SessionId, Shared, ShellRefusal, handlers};

/// What a run needs once its slot is held.
pub(super) struct Run {
    session: Arc<Session>,
    id: SessionId,
    run_id: u64,
    generation: u64,
    cfg: ConnectionConfig,
    exe: PathBuf,
    script: String,
}

pub(super) fn dispatch(
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    command: Command,
) {
    match command {
        Command::ShellCancel { session, run_id } => {
            if let Some(session) = shared.session(session) {
                session.shell.cancel(run_id);
            }
        }
        Command::ShellRun {
            session: id,
            run_id,
            exe,
            script,
        } => match claim(&shared, id, run_id, exe, script) {
            Ok(run) => {
                let (session, spawned) = (run.session.clone(), Arc::new(AtomicBool::new(false)));
                let work = start(
                    shared,
                    events.clone(),
                    repaint.clone(),
                    run,
                    spawned.clone(),
                );
                tokio::spawn(supervised(
                    session, id, run_id, events, repaint, spawned, work,
                ));
            }
            Err(reason) => send(&events, &repaint, refused(id, run_id, reason)),
        },
        _ => unreachable!("only shell commands are routed here"),
    }
}

/// The synchronous checks, in order; on success the session's slot is held for `run_id`.
pub(super) fn claim(
    shared: &Shared,
    id: SessionId,
    run_id: u64,
    exe: PathBuf,
    script: String,
) -> Result<Run, ShellRefusal> {
    let session = shared.session(id).ok_or(ShellRefusal::NotConnected)?;
    let generation = session.generation();
    let (Some(store), Some(cfg)) = (session.current_store(), session.connection_config()) else {
        return Err(ShellRefusal::NotConnected);
    };
    // The store wrapper is the authority, so a lock sent after the connect refuses too.
    if store.is_read_only() {
        return Err(ShellRefusal::ReadOnly);
    }
    if !session.shell.reserve(run_id) {
        return Err(ShellRefusal::AlreadyRunning);
    }
    Ok(Run {
        session,
        id,
        run_id,
        generation,
        cfg,
        exe,
        script,
    })
}

/// Runs `work` in an inner task, like `dispatch` does for every command, so a panic cannot leave
/// the slot reserved: if it happens before the process was spawned, the slot is freed and the run
/// ends with `StartFailed` (the only failing outcome a run has). After the spawn the runner's own
/// `on_done` reports, so nothing is sent twice.
pub(super) async fn supervised(
    session: Arc<Session>,
    id: SessionId,
    run_id: u64,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    spawned: Arc<AtomicBool>,
    work: impl Future<Output = ()> + Send + 'static,
) {
    if tokio::spawn(work).await.is_ok() || spawned.load(Ordering::SeqCst) {
        return;
    }
    log::error!("shell run {run_id}: the start panicked");
    session.shell.release(run_id);
    send(
        &events,
        &repaint,
        finished(id, run_id, Outcome::StartFailed),
    );
}

/// The tunnel address to connect through. A connection configured with an SSH tunnel whose tunnel
/// is gone (its epoch ended meanwhile) is not connected: building an invocation without the
/// address would reach the real host directly.
fn tunnel_for(
    cfg: &ConnectionConfig,
    addr: Option<SocketAddr>,
) -> Result<Option<SocketAddr>, ShellRefusal> {
    match (cfg.ssh.is_some(), addr) {
        (true, None) => Err(ShellRefusal::NotConnected),
        (_, addr) => Ok(addr),
    }
}

pub(super) async fn start(
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    run: Run,
    spawned: Arc<AtomicBool>,
) {
    let (id, run_id) = (run.id, run.run_id);
    let password = match handlers::read_password(&shared, &run.cfg).await {
        Ok(password) => password,
        Err(error) => {
            // The error names no secret; the person sees a run that could not start.
            log::warn!("shell run {run_id}: the password lookup failed: {error}");
            run.session.shell.release(run_id);
            let outcome = Outcome::StartFailed;
            send(&events, &repaint, finished(id, run_id, outcome));
            return;
        }
    };
    if !run.session.is_current(run.generation) {
        run.session.shell.release(run_id);
        send(
            &events,
            &repaint,
            refused(id, run_id, ShellRefusal::NotConnected),
        );
        return;
    }
    // A cancel that came while the password was read must not start anything.
    if run.session.shell.cancel_requested(run_id) {
        run.session.shell.release(run_id);
        send(&events, &repaint, finished(id, run_id, Outcome::Cancelled));
        return;
    }
    let tunnel = match tunnel_for(&run.cfg, run.session.tunnel_addr()) {
        Ok(tunnel) => tunnel,
        Err(reason) => {
            run.session.shell.release(run_id);
            send(&events, &repaint, refused(id, run_id, reason));
            return;
        }
    };
    let invocation = build_invocation(&run.cfg, password.as_deref(), tunnel, &run.script);
    let invocation = match invocation {
        Ok(invocation) => invocation,
        Err(error) => {
            run.session.shell.release(run_id);
            send(
                &events,
                &repaint,
                refused(id, run_id, ShellRefusal::Invalid(error)),
            );
            return;
        }
    };
    let secrets = secrets_of(password);
    let path = crate::tools::default_search_path(std::env::var_os("PATH").as_deref());
    let weak: Weak<Session> = Arc::downgrade(&run.session);
    let (out_events, out_repaint) = (events.clone(), repaint.clone());
    let on_output = move |stream, text| {
        let event = Event::ShellOutput {
            session: id,
            run_id,
            stream,
            text,
        };
        send(&out_events, &out_repaint, event);
    };
    let (history, connection) = (shared.history.clone(), run.cfg.id);
    // Only kept when there is a history to record in.
    let script = history.as_ref().map(|_| run.script.clone());
    let on_done = move |outcome| {
        // Before the slot is freed and the event sent: whoever sees `ShellFinished` finds the run.
        if let Some(script) = &script {
            record_run(history.as_deref(), connection, script, &outcome);
        }
        // Free the slot before the event, so a run sent on seeing `ShellFinished` is not refused.
        if let Some(session) = weak.upgrade() {
            session.shell.release(run_id);
        }
        send(&events, &repaint, finished(id, run_id, outcome));
    };
    // Set before the spawn: from here on the runner's `on_done` reports, so `supervised` must
    // not report a panic as well. A panic inside `spawn_shell` itself (before it could call
    // `on_done`) therefore leaves the slot held until the session reconnects, which frees it.
    spawned.store(true, Ordering::SeqCst);
    let handle = spawn_shell(
        &run.exe,
        invocation,
        secrets,
        Some(&path),
        RunLimits::default(),
        on_output,
        on_done,
    );
    run.session.shell.attach(run_id, handle);
}

/// The password and its percent-encoded form (as it appears inside a connection string).
fn secrets_of(password: Option<String>) -> Vec<String> {
    match password.filter(|p| !p.is_empty()) {
        Some(password) => vec![password.clone(), percent_encode_component(&password)],
        None => Vec::new(),
    }
}

fn refused(session: SessionId, run_id: u64, reason: ShellRefusal) -> Event {
    Event::ShellRefused {
        session,
        run_id,
        reason,
    }
}

fn finished(session: SessionId, run_id: u64, outcome: Outcome) -> Event {
    Event::ShellFinished {
        session,
        run_id,
        outcome,
    }
}

fn send(events: &UnboundedSender<Event>, repaint: &Repaint, event: Event) {
    if events.send(event).is_ok() {
        repaint();
    }
}

/// Remembers a finished run in the history, if there is one and the script actually executed.
/// Runs on the runner's thread, before the slot is released and `ShellFinished` is sent, so a
/// reader that has seen the event finds the entry. A failure (or a panic) is logged and changes
/// nothing: the log names the connection only, because neither the script nor an error that may
/// quote it belongs in a log.
fn record_run(history: Option<&HistoryDb>, connection: Uuid, script: &str, outcome: &Outcome) {
    let (Some(history), Some(status)) = (history, ShellRunStatus::from_outcome(outcome)) else {
        return;
    };
    let now = clock::now_secs();
    let recorded = catch_unwind(AssertUnwindSafe(|| {
        history.record_shell_run(connection, script, status, now)
    }));
    match recorded {
        Ok(Ok(())) => {}
        Ok(Err(_)) => log::warn!("recording a shell run of {connection} in the history failed"),
        Err(_) => log::warn!("recording a shell run of {connection} in the history panicked"),
    }
}

/// `ShellHistory`, `ClearShellHistory` and `CopyShellHistory`: reads and writes of the history
/// file on the blocking pool. The history is keyed by the connection id, which is the session id.
pub(super) fn dispatch_history(
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    command: Command,
) {
    let history = shared.history.clone();
    match command {
        Command::ShellHistory { session } => {
            answer_history(history, session, false, "shell_history", events, repaint);
        }
        Command::ClearShellHistory { session } => {
            answer_history(
                history,
                session,
                true,
                "clear_shell_history",
                events,
                repaint,
            );
        }
        Command::CopyShellHistory { from, to } => {
            let Some(history) = history else {
                return;
            };
            tokio::spawn(async move {
                let copied = blocking(move || history.copy_shell_history(from, to)).await;
                // Ids only, like `record_run`: the error text is not logged.
                if copied.is_err() {
                    log::warn!("copying the shell history of {from} to {to} failed");
                }
            });
        }
        _ => unreachable!("only shell history commands are routed here"),
    }
}

/// Answers with the connection's entries, or with none after `clear`. Without history, and on a
/// failed read or clear, the answer is a `Failed` event with the command's context (never an
/// empty list, which would read as "no scripts yet"); the error carries no path or system text.
fn answer_history(
    history: Option<Arc<HistoryDb>>,
    session: SessionId,
    clear: bool,
    context: &'static str,
    events: UnboundedSender<Event>,
    repaint: Repaint,
) {
    tokio::spawn(async move {
        let entries = match history {
            None => Err(CoreError::NotFound),
            Some(history) if clear => {
                blocking(move || history.clear_shell_history(session).map(|()| Vec::new())).await
            }
            Some(history) => blocking(move || history.shell_history(session)).await,
        };
        let event = match entries {
            Ok(entries) => Event::ShellHistory { session, entries },
            Err(error) => handlers::failure_event(session, context, None, 0, error),
        };
        send(&events, &repaint, event);
    });
}
