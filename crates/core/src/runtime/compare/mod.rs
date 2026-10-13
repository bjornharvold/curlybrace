//! Compare sessions in the runtime. They are independent of the main browsing connection: their
//! own connections, their own lifetime, never touching `Shared::store`, the connection epoch or
//! the "exactly one event per command" bookkeeping of the data commands.
//!
//! Answers: `Open` → `Opened` | `OpenFailed`; `Databases`/`Collections`/`Check`/`Rows`/`Detail`
//! → their answer | `Failed { context }`; `Start` → one `PairDone` per pair, then one `RunDone`
//! (or `Failed` when refused); `Apply` → `ApplyProgress`*, one `ApplyPairDone` per pair, one
//! `ApplyDone` (or `Failed { context: "compare_apply" }` when refused); `DuplicateKeys` → `DuplicateKeys` | `Failed { context:
//! "compare_duplicates" }`; `Cancel`, `Discard`, `Close` have no answer of their own.
//!
//! Once a session is closed it emits nothing more: every event tied to a session goes through
//! [`Session::emit_if_open`], which checks "open" and sends under one lock. A command naming an
//! unknown or already closed session is answered with `Failed`.

#[cfg(test)]
mod abort_tests;
mod apply;
#[cfg(test)]
mod apply_bulk_tests;
#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod closed_tests;
#[cfg(test)]
mod duplicates_tests;
#[cfg(test)]
mod run_tests;
mod runner;
mod session;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use bson::Document;
use tokio::sync::mpsc::UnboundedSender;

use crate::compare::protocol::{
    CompareCommand, CompareEvent, DUPLICATE_KEYS_LIMIT, DuplicateReport, PairSpec, Preflight,
    RunId, SessionId, Side,
};
use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};

use crate::store::{DocumentStore, IndexStatus, Namespace};

use super::tunneling::{Currentness, open_tunnel_if_configured};
use super::{Event, Repaint, Shared, TunnelHandle};
pub(super) use session::Registry;
use session::{Connections, Session};

pub(super) type Events = UnboundedSender<Event>;

/// Entry point from `runtime::dispatch`. Session bookkeeping (create, close, lookup) happens
/// synchronously here so that command ORDER decides outcomes (an `Open` followed by a `Close`
/// always closes); the slow work runs in a spawned task.
pub(super) fn dispatch(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    command: CompareCommand,
) {
    match command {
        CompareCommand::Open {
            session,
            source,
            target,
        } => {
            let record = shared.compare.create(session, shared.config.compare_limits);
            spawn_open(shared, events, repaint, session, record, [source, target]);
        }
        CompareCommand::Close { session } => shared.compare.remove(session),
        CompareCommand::Cancel { session, run } => runner::cancel(&shared, session, run),
        CompareCommand::Discard { session, pair } => runner::discard(&shared, session, pair),
        CompareCommand::Start {
            session,
            run,
            pairs,
        } => start_run(&shared, events, repaint, session, run, pairs),
        CompareCommand::Apply {
            session,
            run,
            ops,
            bulk,
            id_policy,
        } => apply::dispatch(
            &shared,
            events,
            repaint,
            session,
            run,
            (ops, bulk),
            id_policy,
        ),
        other => spawn_command(shared, events, repaint, other),
    }
}

/// Runs the open in an inner task so a panic still ends in exactly one `OpenFailed`
/// (unless the session was closed meanwhile, which silences everything).
fn spawn_open(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    record: Arc<Session>,
    configs: [ConnectionConfig; 2],
) {
    tokio::spawn(async move {
        let work = tokio::spawn(open_session(
            shared.clone(),
            events.clone(),
            repaint.clone(),
            id,
            record.clone(),
            configs,
        ));
        if work.await.is_err() {
            let error = CoreError::Database("internal error".into());
            record.emit_if_open(
                &events,
                &repaint,
                CompareEvent::OpenFailed { session: id, error },
            );
            shared.compare.remove_exact(id, &record);
        }
    });
}

/// Answers a command naming an unknown or closed session, synchronously.
fn answer_closed(events: &Events, repaint: &Repaint, id: SessionId, context: &'static str) {
    let error = CoreError::Unsupported("the comparison is closed".into());
    let failed = CompareEvent::Failed {
        session: id,
        context,
        error,
    };
    if events.send(Event::Compare(failed)).is_ok() {
        repaint();
    }
}

/// Registers the run synchronously, so command ORDER decides outcomes (a `Cancel` or a second
/// `Start` right behind this one sees the run); only the run itself happens in a task.
fn start_run(
    shared: &Shared,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    run: RunId,
    pairs: Vec<PairSpec>,
) {
    let Some(session) = shared.compare.get(id).filter(|s| s.is_open()) else {
        return answer_closed(&events, &repaint, id, "compare_start");
    };
    if let Err(error) = runner::start(
        events.clone(),
        repaint.clone(),
        session.clone(),
        id,
        run,
        pairs,
    ) {
        let failed = CompareEvent::Failed {
            session: id,
            context: "compare_start",
            error,
        };
        session.emit_if_open(&events, &repaint, failed);
    }
}

fn spawn_command(shared: Arc<Shared>, events: Events, repaint: Repaint, command: CompareCommand) {
    let (id, context) = (command_session(&command), command_context(&command));
    let Some(session) = shared.compare.get(id).filter(|s| s.is_open()) else {
        return answer_closed(&events, &repaint, id, context);
    };
    tokio::spawn(async move {
        let work = tokio::spawn(handle(
            shared,
            events.clone(),
            repaint.clone(),
            session.clone(),
            command,
        ));
        let outcome = work
            .await
            .unwrap_or_else(|_| Err(CoreError::Database("internal error".into())));
        let event = match outcome {
            Ok(Some(answer)) => answer,
            Ok(None) => return,
            Err(error) => CompareEvent::Failed {
                session: id,
                context,
                error,
            },
        };
        session.emit_if_open(&events, &repaint, event);
    });
}

fn command_session(c: &CompareCommand) -> SessionId {
    match c {
        CompareCommand::Open { session, .. }
        | CompareCommand::Databases { session, .. }
        | CompareCommand::Collections { session, .. }
        | CompareCommand::Check { session, .. }
        | CompareCommand::Start { session, .. }
        | CompareCommand::Apply { session, .. }
        | CompareCommand::Cancel { session, .. }
        | CompareCommand::Rows { session, .. }
        | CompareCommand::Detail { session, .. }
        | CompareCommand::Discard { session, .. }
        | CompareCommand::DuplicateKeys { session, .. }
        | CompareCommand::Close { session } => *session,
    }
}

fn command_context(c: &CompareCommand) -> &'static str {
    match c {
        CompareCommand::Databases { .. } => "compare_databases",
        CompareCommand::Collections { .. } => "compare_collections",
        CompareCommand::Check { .. } => "compare_check",
        CompareCommand::Start { .. } => "compare_start",
        CompareCommand::Apply { .. } => "compare_apply",
        CompareCommand::Rows { .. } => "compare_rows",
        CompareCommand::Detail { .. } => "compare_detail",
        CompareCommand::DuplicateKeys { .. } => "compare_duplicates",
        _ => "compare",
    }
}

/// Runs one command against its session. `Ok(Some(event))` is the answer to emit; commands that
/// stream their own events (`Rows`) return `Ok(None)`.
async fn handle(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    session: Arc<Session>,
    command: CompareCommand,
) -> Result<Option<CompareEvent>> {
    let id = command_session(&command);
    let limit = shared.config.command_timeout;
    let answer = match command {
        CompareCommand::Databases { side, .. } => list_databases(&session, id, side, limit).await?,
        CompareCommand::Collections { side, db, .. } => {
            list_collections(&session, id, side, db, limit).await?
        }
        CompareCommand::Check { pair, .. } => check_pair(&session, id, &pair, limit).await?,
        CompareCommand::DuplicateKeys { pair, .. } => {
            find_duplicates(&session, id, &pair, shared.config.duplicate_timeout).await?
        }
        CompareCommand::Detail { pair, key, .. } => {
            bounded(limit, runner::detail(&session, id, pair, key)).await?
        }
        CompareCommand::Rows {
            pair,
            category,
            offset,
            limit: rows,
            ..
        } => {
            runner::rows(
                &session, &events, &repaint, id, pair, category, offset, rows,
            )?;
            return Ok(None);
        }
        CompareCommand::Open { .. }
        | CompareCommand::Start { .. }
        | CompareCommand::Apply { .. }
        | CompareCommand::Cancel { .. }
        | CompareCommand::Discard { .. }
        | CompareCommand::Close { .. } => return Ok(None),
    };
    Ok(Some(answer))
}

async fn list_databases(
    session: &Session,
    id: SessionId,
    side: Side,
    limit: Duration,
) -> Result<CompareEvent> {
    let store = connections(session)?.store(side);
    let names = bounded(limit, async { store.list_databases().await }).await?;
    Ok(CompareEvent::Databases {
        session: id,
        side,
        names,
    })
}

async fn list_collections(
    session: &Session,
    id: SessionId,
    side: Side,
    db: String,
    limit: Duration,
) -> Result<CompareEvent> {
    let store = connections(session)?.store(side);
    let entries = bounded(limit, async { store.list_collection_kinds(&db).await }).await?;
    Ok(CompareEvent::Collections {
        session: id,
        side,
        db,
        entries,
    })
}

async fn check_pair(
    session: &Session,
    id: SessionId,
    pair: &PairSpec,
    limit: Duration,
) -> Result<CompareEvent> {
    let preflight = bounded(limit, check(session, pair)).await?;
    Ok(CompareEvent::Checked {
        session: id,
        pair: pair.id,
        preflight,
    })
}

/// Asks both sides for their duplicate key groups, bounded as a whole by `timeout`.
async fn find_duplicates(
    session: &Session,
    id: SessionId,
    pair: &PairSpec,
    timeout: Duration,
) -> Result<CompareEvent> {
    runner::validate_spec(pair)?;
    let conns = connections(session)?;
    let ask = |store: Arc<dyn DocumentStore>, ns: &Namespace, filter: &Document| {
        let (ns, filter, key) = (ns.clone(), filter.clone(), pair.key.clone());
        async move {
            let shown = DUPLICATE_KEYS_LIMIT as usize;
            let mut groups = store
                .duplicate_keys(&ns, &filter, &key, DUPLICATE_KEYS_LIMIT + 1)
                .await?;
            let more = groups.len() > shown;
            groups.truncate(shown);
            Ok::<_, CoreError>(DuplicateReport { groups, more })
        }
    };
    let both = async {
        tokio::try_join!(
            ask(conns.source.clone(), &pair.source, &pair.source_filter),
            ask(conns.target.clone(), &pair.target, &pair.target_filter),
        )
    };
    let (source, target) = bounded(timeout, both).await?;
    Ok(CompareEvent::DuplicateKeys {
        session: id,
        pair: pair.id,
        source,
        target,
    })
}

async fn bounded<T>(
    limit: Duration,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| CoreError::Database("timeout".into()))?
}

fn connections(session: &Session) -> Result<&Connections> {
    session
        .connections
        .get()
        .ok_or_else(|| CoreError::Unsupported("the comparison is still opening".into()))
}

async fn check(session: &Session, pair: &PairSpec) -> Result<Preflight> {
    runner::validate_spec(pair)?;
    let conns = connections(session)?;
    let (source_count, target_count) = tokio::try_join!(
        conns.source.estimated_count(&pair.source),
        conns.target.estimated_count(&pair.target)
    )?;
    let (source_index, target_index) = tokio::try_join!(
        conns.source.key_index_status(&pair.source, &pair.key),
        conns.target.key_index_status(&pair.target, &pair.key)
    )?;
    let cap = session.budget.limits().max_unindexed_documents;
    let too_large =
        |index: IndexStatus, count: u64| index == IndexStatus::NotIndexed && count > cap;
    Ok(Preflight {
        source_count,
        target_count,
        source_index,
        target_index,
        unindexed_too_large: too_large(source_index, source_count)
            || too_large(target_index, target_count),
    })
}

async fn open_session(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    session: Arc<Session>,
    configs: [ConnectionConfig; 2],
) {
    let result = tokio::time::timeout(shared.config.connect_timeout, async {
        let [source, target] = configs;
        let (store_s, tunnel_s) = open_side(&shared, &session, &source).await?;
        let (store_t, tunnel_t) = open_side(&shared, &session, &target).await?;
        let tunnels: Vec<Box<dyn TunnelHandle>> =
            [tunnel_s, tunnel_t].into_iter().flatten().collect();
        Ok::<_, CoreError>(Connections {
            source: store_s,
            target: store_t,
            _tunnels: tunnels,
        })
    })
    .await
    .unwrap_or_else(|_| Err(CoreError::Database("timeout".into())));
    match result {
        // A closed session drops what was opened (connections, tunnels) and stays silent.
        Ok(connections) => {
            session.finish_open(connections, id, &events, &repaint);
        }
        Err(error) => {
            let failed = CompareEvent::OpenFailed { session: id, error };
            session.emit_if_open(&events, &repaint, failed);
            shared.compare.remove_exact(id, &session);
        }
    }
}

async fn open_side(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    cfg: &ConnectionConfig,
) -> Result<(Arc<dyn DocumentStore>, Option<Box<dyn TunnelHandle>>)> {
    cfg.validate()?;
    let password = super::handlers::read_password(shared, cfg).await?;
    let job: Arc<dyn Currentness> = session.clone();
    let tunnel = open_tunnel_if_configured(shared, job, cfg).await?;
    let addr = tunnel.as_ref().map(|t| t.local_addr());
    let store = shared
        .connector
        .connect(cfg, password.as_deref(), addr)
        .await?;
    Ok((store, tunnel))
}
