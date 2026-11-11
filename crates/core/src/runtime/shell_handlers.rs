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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::sync::mpsc::UnboundedSender;

use crate::connection::ConnectionConfig;
use crate::shell::{
    Outcome, RunLimits, ShellResult, build_invocation, parse_scrubbed, secret_forms, spawn_shell,
};

use super::session::Session;
use super::shell_history_queue::Job;
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
    // Only kept when there is a history to record in.
    let record = shared
        .history
        .as_ref()
        .map(|_| (shared.shell_history.clone(), run.script.clone()));
    let connection = run.cfg.id;
    // The parsed payload, written by `on_result` and read by `on_done`; both run on the
    // supervisor thread, `on_result` first.
    let lifted: Arc<Mutex<Option<ShellResult>>> = Arc::new(Mutex::new(None));
    let on_result = {
        let lifted = lifted.clone();
        move |payload: String, masked: bool| {
            let parsed = parse_scrubbed(&payload, masked);
            log::debug!(
                "shell run {run_id}: a result of {} bytes, masked: {masked}, parsed: {}",
                payload.len(),
                parsed.is_some()
            );
            if parsed == Some(ShellResult::Withheld) {
                log::warn!("shell run {run_id}: masking the password broke the result; withheld");
            }
            *lifted.lock().unwrap_or_else(PoisonError::into_inner) = parsed;
        }
    };
    let on_done = move |outcome| {
        // Free the slot before the event, so a run sent on seeing `ShellFinished` is not refused.
        if let Some(session) = weak.upgrade() {
            session.shell.release(run_id);
        }
        let parsed = lifted.lock().unwrap_or_else(PoisonError::into_inner).take();
        // Right before `ShellFinished`, and before the history record is queued.
        if let Some(result) = result_to_send(outcome, parsed) {
            let event = Event::ShellResult {
                session: id,
                run_id,
                result,
            };
            send(&events, &repaint, event);
        }
        send(&events, &repaint, finished(id, run_id, outcome));
        // After the event and without waiting: the history file never holds up the run's end.
        // The queue answers with `ShellHistoryChanged` once the record is written.
        if let Some((queue, script)) = record {
            queue.push(Job::Record {
                connection,
                script,
                outcome,
            });
        }
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
        on_result,
        on_done,
    );
    run.session.shell.attach(run_id, handle);
}

/// What a run's parsed payload becomes: it is delivered only for a run that exited 0, and dropped
/// for every other outcome (cancelled, capped, script errors, a non-zero exit, ...).
pub(super) fn result_to_send(outcome: Outcome, parsed: Option<ShellResult>) -> Option<ShellResult> {
    match outcome {
        Outcome::Exited(0) => parsed,
        Outcome::Exited(_)
        | Outcome::ScriptErrors
        | Outcome::Incomplete
        | Outcome::ConnectFailed
        | Outcome::Signalled
        | Outcome::Cancelled
        | Outcome::CapReached
        | Outcome::StartFailed => None,
    }
}

/// The password in every form it can be echoed in (raw, percent-encoded, JSON- and
/// inspect-escaped).
fn secrets_of(password: Option<String>) -> Vec<String> {
    password.as_deref().map(secret_forms).unwrap_or_default()
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

/// `ShellHistory`, `ClearShellHistory` and `CopyShellHistory`: queued behind every earlier
/// history job (a finished run's record included), so they run in send order. The history is
/// keyed by the connection id, which is the session id.
pub(super) fn dispatch_history(shared: &Shared, command: Command) {
    let job = match command {
        Command::ShellHistory { session } => Job::read(session),
        Command::ClearShellHistory { session } => Job::clear(session),
        Command::CopyShellHistory { from, to } => Job::Copy { from, to },
        _ => unreachable!("only shell history commands are routed here"),
    };
    shared.shell_history.push(job);
}
