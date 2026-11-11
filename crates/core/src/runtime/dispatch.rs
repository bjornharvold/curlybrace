//! Routing of one [`Command`] to its handler: the commands that need no session are answered
//! here, every other one runs in its own task against the session it names.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use super::session::Session;
use super::{
    Command, CoreError, Event, Repaint, SessionId, Shared, compare, copy, handlers, indexes,
    profiler_handlers, profiler_start, shell_handlers, snapshots,
};

/// Runs one command in its own task. The work runs in an inner task so a
/// panic surfaces as a `JoinError` here and still produces one `Failed` event.
pub(super) fn dispatch(
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    command: Command,
) {
    if let Command::Compare(c) = command {
        compare::dispatch(shared, events, repaint, c);
        return;
    }
    if let Command::Copy(c) = command {
        copy::dispatch(shared, events, repaint, c);
        return;
    }
    // Like Compare: independent of the connection epoch (no begin_epoch, no generation read).
    if let Command::TestConnection {
        request_id,
        cfg,
        secrets,
    } = command
    {
        handlers::spawn_test(shared, events, repaint, request_id, cfg, secrets);
        return;
    }
    // Synchronous, so a write sent after the lock is already refused. No session, no epoch, no
    // answer: the caller learns of it from the next refused write.
    if let Command::LockReadOnly { connection } = command {
        shared.read_only.lock(connection);
        return;
    }
    if let Command::SetSamplingExcluded { databases } = command {
        shared
            .sampler_config
            .send_modify(|cfg| cfg.excluded_dbs = databases);
        return;
    }
    if let Command::SnapshotList { request } = command {
        snapshots::spawn_listing(&shared, events, repaint, request, None);
        return;
    }
    if let Command::SnapshotDelete { run, request } = command {
        snapshots::spawn_listing(&shared, events, repaint, request, Some(run));
        return;
    }
    if let Command::PurgeHistory { connection } = command {
        indexes::spawn_purge(&shared, connection);
        return;
    }
    // Shell history needs only the history file, not a live session.
    if matches!(
        command,
        Command::ShellHistory { .. }
            | Command::ClearShellHistory { .. }
            | Command::CopyShellHistory { .. }
    ) {
        shell_handlers::dispatch_history(shared, events, repaint, command);
        return;
    }
    // Synchronous, so request order decides: ending the session bumps its epoch before any later
    // command is dispatched. The session is gone afterwards, so the answer needs no `Session`.
    if let Command::Disconnect { session: id } = command {
        let epoch = shared.end_session(id);
        if events
            .send(Event::Disconnected { session: id, epoch })
            .is_ok()
        {
            repaint();
        }
        return;
    }
    // Synchronous, so request order decides which of two runs holds the session's one slot.
    if matches!(
        command,
        Command::ShellRun { .. } | Command::ShellCancel { .. }
    ) {
        shell_handlers::dispatch(shared, events, repaint, command);
        return;
    }
    let context = command.context();
    let request = command.request();
    // Synchronous, so request order decides which connection wins.
    let (target, work) = match command {
        Command::Connect(cfg) => {
            let (session, generation) = shared.begin_session_epoch(cfg.id);
            // Synchronous like the epoch, so a LockReadOnly sent after this Connect reaches the
            // store it installs even while the connect is still in flight. A connect that fails
            // or is superseded just drops the flag.
            let read_only = shared.read_only.attach(cfg.id, cfg.read_only);
            let target = Target::new(cfg.id, session, generation, false);
            (
                target,
                handlers::Work::Connect {
                    cfg: Box::new(cfg),
                    read_only,
                },
            )
        }
        data => {
            let id = data.session().expect("a data command names its session");
            let Some(session) = shared.session(id) else {
                let error = CoreError::Unsupported("not connected".into());
                let unknown = match profiler_handlers::profiler_db(&data) {
                    Some(db) => profiler_handlers::profiler_failure(id, db.to_owned(), error),
                    None => handlers::failure_event(id, context, request, 0, error),
                };
                if events.send(unknown).is_ok() {
                    repaint();
                }
                return;
            };
            let generation = session.generation();
            let target = Target::new(id, session, generation, true);
            (target, handlers::Work::Data(Box::new(data)))
        }
    };
    tokio::spawn(async move {
        let Target {
            id,
            session,
            generation,
            is_data,
        } = target;
        let work = handlers::execute(shared.clone(), session.clone(), id, work, generation);
        let event = match tokio::spawn(work).await {
            Ok(event) => event,
            Err(_) => handlers::failure_event(
                id,
                context,
                request,
                generation,
                handlers::gate(
                    &session,
                    generation,
                    CoreError::Database("internal error".into()),
                ),
            ),
        };
        let connected = matches!(event, Event::Connected { .. });
        if handlers::send_answer(&session, id, generation, is_data, context, event, &events) {
            repaint();
        }
        // After `Connected` is queued, so the sampler's first status can never overtake it.
        if connected {
            indexes::start_sampler(
                &shared,
                &session,
                id,
                generation,
                events.clone(),
                repaint.clone(),
            );
            profiler_start::start_profiler(&shared, &session, id, generation, events, repaint);
        }
    });
}

/// The session a dispatched command runs against, with the epoch it was dispatched in.
struct Target {
    id: SessionId,
    session: Arc<Session>,
    generation: u64,
    is_data: bool,
}

impl Target {
    fn new(id: SessionId, session: Arc<Session>, generation: u64, is_data: bool) -> Self {
        Self {
            id,
            session,
            generation,
            is_data,
        }
    }
}
