//! `Command::ShellRun` / `Command::ShellCancel`: running `mongosh` for the Shell tab.
//!
//! Everything that decides a refusal (session, connection, read-only, the one-run slot) happens
//! synchronously in [`dispatch`], so request order decides which of two runs holds the slot. Only
//! the keychain read is awaited, in a task. The runner's callbacks run on its supervisor thread and
//! send on the runtime's (unbounded, so synchronous) event channel.
//!
//! Locks: the session's shell slot is taken only to check, insert or clear, never around
//! `spawn_shell`, whose `on_done` may run before it returns and clears the slot itself.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use tokio::sync::mpsc::UnboundedSender;

use crate::connection::ConnectionConfig;
use crate::shell::{Outcome, RunLimits, build_invocation, percent_encode_component, spawn_shell};

use super::session::Session;
use super::{Command, Event, Repaint, SessionId, Shared, ShellRefusal, handlers};

/// What a run needs once its slot is held.
struct Run {
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
                tokio::spawn(start(shared, events, repaint, run));
            }
            Err(reason) => send(&events, &repaint, refused(id, run_id, reason)),
        },
        _ => unreachable!("only shell commands are routed here"),
    }
}

/// The synchronous checks, in order; on success the session's slot is held for `run_id`.
fn claim(
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

async fn start(shared: Arc<Shared>, events: UnboundedSender<Event>, repaint: Repaint, run: Run) {
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
    let invocation = build_invocation(
        &run.cfg,
        password.as_deref(),
        run.session.tunnel_addr(),
        &run.script,
    );
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
    let on_done = move |outcome| {
        // Free the slot before the event, so a run sent on seeing `ShellFinished` is not refused.
        if let Some(session) = weak.upgrade() {
            session.shell.release(run_id);
        }
        send(&events, &repaint, finished(id, run_id, outcome));
    };
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
