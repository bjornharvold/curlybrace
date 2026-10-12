//! Compare sessions in the runtime. They are independent of the main browsing connection: their
//! own connections, their own lifetime, never touching `Shared::store`, the connection epoch or
//! the "exactly one event per command" bookkeeping of the data commands.
//!
//! Answers: `Open` → `Opened` | `OpenFailed`; `Databases`/`Collections`/`Check`/`Rows`/`Detail`
//! → their answer | `Failed { context }`; `Start` → one `PairDone` per pair, then one `RunDone`
//! (or `Failed` when refused); `Cancel`, `Discard`, `Close` have no answer of their own.

mod runner;
mod session;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::compare::protocol::{CompareCommand, CompareEvent, PairSpec, Preflight, SessionId};
use crate::compare::results::Limits;
use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, IndexStatus};

use super::tunneling::{Currentness, open_tunnel_if_configured};
use super::{Event, Repaint, Shared, TunnelHandle};
pub(super) use session::Registry;
use session::{Connections, Session};

pub(super) type Events = UnboundedSender<Event>;

/// Sends a compare event and repaints. Returns false once the receiver is gone.
pub(super) fn emit(events: &Events, repaint: &Repaint, event: CompareEvent) -> bool {
    let sent = events.send(Event::Compare(event)).is_ok();
    if sent {
        repaint();
    }
    sent
}

/// Entry point from `runtime::dispatch`. Session bookkeeping happens synchronously here so that
/// command ORDER decides outcomes (an `Open` followed by a `Close` always closes); the slow work
/// runs in a spawned task.
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
            let record = shared.compare.create(session, Limits::default());
            tokio::spawn(open_session(
                shared,
                events,
                repaint,
                session,
                record,
                [source, target],
            ));
        }
        CompareCommand::Close { session } => shared.compare.remove(session),
        CompareCommand::Cancel { session, run } => runner::cancel(&shared, session, run),
        CompareCommand::Discard { session, pair } => runner::discard(&shared, session, pair),
        other => {
            tokio::spawn(async move {
                let (session, context) = (command_session(&other), command_context(&other));
                if let Err(error) = handle(&shared, &events, &repaint, other).await {
                    let failed = CompareEvent::Failed {
                        session,
                        context,
                        error,
                    };
                    emit(&events, &repaint, failed);
                }
            });
        }
    }
}

fn command_session(c: &CompareCommand) -> SessionId {
    match c {
        CompareCommand::Open { session, .. }
        | CompareCommand::Databases { session, .. }
        | CompareCommand::Collections { session, .. }
        | CompareCommand::Check { session, .. }
        | CompareCommand::Start { session, .. }
        | CompareCommand::Cancel { session, .. }
        | CompareCommand::Rows { session, .. }
        | CompareCommand::Detail { session, .. }
        | CompareCommand::Discard { session, .. }
        | CompareCommand::Close { session } => *session,
    }
}

fn command_context(c: &CompareCommand) -> &'static str {
    match c {
        CompareCommand::Databases { .. } => "compare_databases",
        CompareCommand::Collections { .. } => "compare_collections",
        CompareCommand::Check { .. } => "compare_check",
        CompareCommand::Start { .. } => "compare_start",
        CompareCommand::Rows { .. } => "compare_rows",
        CompareCommand::Detail { .. } => "compare_detail",
        _ => "compare",
    }
}

async fn handle(
    shared: &Arc<Shared>,
    events: &Events,
    repaint: &Repaint,
    command: CompareCommand,
) -> Result<()> {
    let session_id = command_session(&command);
    let session = shared
        .compare
        .get(session_id)
        .filter(|s| s.is_open())
        .ok_or_else(|| CoreError::Unsupported("the comparison is closed".into()))?;
    let limit = shared.config.command_timeout;
    match command {
        CompareCommand::Databases { side, .. } => {
            let store = connections(&session)?.store(side);
            let names = bounded(limit, async { store.list_databases().await }).await?;
            let event = CompareEvent::Databases {
                session: session_id,
                side,
                names,
            };
            emit(events, repaint, event);
        }
        CompareCommand::Collections { side, db, .. } => {
            let store = connections(&session)?.store(side);
            let entries = bounded(limit, async { store.list_collection_kinds(&db).await }).await?;
            let event = CompareEvent::Collections {
                session: session_id,
                side,
                db,
                entries,
            };
            emit(events, repaint, event);
        }
        CompareCommand::Check { pair, .. } => {
            let preflight = bounded(limit, check(&session, &pair)).await?;
            let event = CompareEvent::Checked {
                session: session_id,
                pair: pair.id,
                preflight,
            };
            emit(events, repaint, event);
        }
        CompareCommand::Start { run, pairs, .. } => {
            let (shared, events, repaint) = (shared.clone(), events.clone(), repaint.clone());
            runner::start(shared, events, repaint, session, session_id, run, pairs)?;
        }
        CompareCommand::Rows {
            pair,
            category,
            offset,
            limit: rows,
            ..
        } => {
            runner::rows(
                &session, events, repaint, session_id, pair, category, offset, rows,
            )?;
        }
        CompareCommand::Detail { pair, key, .. } => {
            let event = bounded(limit, runner::detail(&session, session_id, pair, key)).await?;
            emit(events, repaint, event);
        }
        CompareCommand::Open { .. }
        | CompareCommand::Cancel { .. }
        | CompareCommand::Discard { .. }
        | CompareCommand::Close { .. } => {}
    }
    Ok(())
}

async fn bounded<T>(
    limit: std::time::Duration,
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
    if !session.is_open() {
        // Closed (or replaced) while connecting: whatever was opened is dropped here.
        return;
    }
    match result {
        Ok(connections) => {
            if session.connections.set(connections).is_ok() {
                emit(&events, &repaint, CompareEvent::Opened { session: id });
            }
        }
        Err(error) => {
            shared.compare.remove_exact(id, &session);
            emit(
                &events,
                &repaint,
                CompareEvent::OpenFailed { session: id, error },
            );
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
