//! Starting the profile poller of a session that just connected, beside the index sampler.

use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use super::indexes::{Forwarded, forward_event, spawn_forwarder};
use super::profiler;
use super::profiler_status::ProfilerStatus;
use super::sampler::SamplerPause;
use super::session::Session;
use super::{Event, Repaint, SessionId, Shared};

/// Starts the poller of a session that just connected, if history is available and the connect
/// is still current, and forwards its statuses as `Event::Profiler`. The poller reads through the
/// session's own (read-only wrapped) store; it only reads, and never switches a profiler on.
pub(super) fn start_profiler(
    shared: &Shared,
    session: &Arc<Session>,
    id: SessionId,
    generation: u64,
    events: UnboundedSender<Event>,
    repaint: Repaint,
) {
    if shared.history.is_none() && shared.config.history_path.is_some() {
        // History was wanted but could not be opened (logged at spawn): say so once, spawn nothing.
        let event = Event::Profiler {
            session: id,
            status: ProfilerStatus::Paused(SamplerPause::HistoryUnavailable),
        };
        let weak = Arc::downgrade(session);
        if let Forwarded::Sent = forward_event(&weak, generation, &events, event) {
            repaint();
        }
        return;
    }
    let (Some(history), Some(store)) = (shared.history.clone(), session.current_store()) else {
        return;
    };
    history.allow_recording(id);
    let (status_tx, statuses) = unbounded_channel();
    let handle = profiler::spawn(
        store,
        history,
        id,
        shared.sampler_config.subscribe(),
        status_tx,
    );
    let abort = handle.abort_handle();
    if !session.attach_profiler(generation, handle) {
        return;
    }
    spawn_forwarder(
        statuses,
        abort,
        Arc::downgrade(session),
        generation,
        events,
        repaint,
        move |status| Event::Profiler {
            session: id,
            status,
        },
    );
}
