//! The query history inside the running app (#72): recording a find where it actually goes out,
//! the "Remember queries" switch, writing `query_history.json` from the dispatch loop, and
//! following a connection that is deleted or duplicated. The pure rules are in
//! `curlybrace_core::query_history`; the file rules are in `query_history_store`.
//!
//! The history holds the text of queries, which can contain personal values: nothing here logs a
//! query, a name or a namespace, only error kinds.
use super::persistence::{self, PersistGuard, Recovery};
use super::query_history_store::{self as store, HistorySaveError, Reload};
use super::state::{AppState, Banner};
use super::{ConfigPath, CurlybraceApp, connection_ops};
use crate::viewmodel::aggregate_options::OptionsForm;
use crate::viewmodel::query_history::HistoryUi;
use crate::viewmodel::{FindBarState, submit_find};
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::query::QueryText;
use curlybrace_core::query_history::{FindText, QueryHistory, SavedError};
use curlybrace_core::store::Query;
use std::path::PathBuf;
use uuid::Uuid;

/// One change to the store. Changes are kept until they are on disk so that, when another window
/// wrote the file first, they can be made again on top of what it wrote.
#[derive(Clone)]
pub(super) enum Op {
    Record {
        connection: Uuid,
        db: String,
        collection: String,
        text: Box<FindText>,
        now: i64,
    },
    Star {
        /// Chosen when the dialog is confirmed, so the favourite has the same id when the change
        /// is made again on top of another window's file.
        id: Uuid,
        connection: Uuid,
        db: String,
        collection: String,
        name: String,
        text: Box<FindText>,
        now: i64,
    },
    Rename {
        connection: Uuid,
        id: Uuid,
        name: String,
    },
    Unstar {
        connection: Uuid,
        id: Uuid,
    },
    ClearNamespace {
        connection: Uuid,
        db: String,
        collection: String,
    },
    Remember(bool),
    ForgetConnection(Uuid),
    CopyConnection {
        from: Uuid,
        to: Uuid,
    },
}

/// Only the kind of change: the rest is the person's text.
impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Record { .. } => "Op::Record",
            Self::Star { .. } => "Op::Star",
            Self::Rename { .. } => "Op::Rename",
            Self::Unstar { .. } => "Op::Unstar",
            Self::ClearNamespace { .. } => "Op::ClearNamespace",
            Self::Remember(_) => "Op::Remember",
            Self::ForgetConnection(_) => "Op::ForgetConnection",
            Self::CopyConnection { .. } => "Op::CopyConnection",
        })
    }
}

impl Op {
    fn apply(&self, store: &QueryHistory) -> Result<QueryHistory, SavedError> {
        match self {
            Self::Record {
                connection,
                db,
                collection,
                text,
                now,
            } => store.recorded(*connection, db, collection, (**text).clone(), *now),
            Self::Star {
                id,
                connection,
                db,
                collection,
                name,
                text,
                now,
            } => store.starred_with_id(
                *id,
                *connection,
                db,
                collection,
                name,
                (**text).clone(),
                *now,
            ),
            Self::Rename {
                connection,
                id,
                name,
            } => store.renamed(*connection, *id, name),
            Self::Unstar { connection, id } => Ok(store.unstarred(*connection, *id)),
            Self::ClearNamespace {
                connection,
                db,
                collection,
            } => Ok(store.cleared(*connection, db, collection)),
            Self::Remember(on) => Ok(store.with_remembering(*on)),
            Self::ForgetConnection(id) => Ok(store.without_connection(*id)),
            Self::CopyConnection { from, to } => Ok(store.with_connection_copied(*from, *to)),
        }
    }
}

/// The history, where its file is and what is still to be written.
#[derive(Debug, Default)]
pub struct QueryHistoryState {
    pub store: QueryHistory,
    /// `None` when there is no settings folder (or a test did not set one): the history then
    /// lives in memory only and nothing is ever written to the real config directory.
    pub path: Option<PathBuf>,
    /// Whether the file may be replaced (not when a bad one could not be backed up).
    pub guard: PersistGuard,
    /// What this window last read from or wrote to the file, to notice another window.
    pub seen: super::pipelines_store::DiskStamp,
    /// The History popup: its search, dialog and last error.
    pub ui: HistoryUi,
    /// Changes applied to `store` and not yet on disk, oldest first. At most
    /// [`MAX_PENDING_RECORDS`] of them are recordings (the history itself is all in `store`);
    /// the changes to favourites are never dropped.
    pending: Vec<Op>,
    /// Counts the changes made, so a step that made none can be told from one that did.
    revision: u64,
    /// The last write failed: nothing is tried again until the history or the file changed.
    stalled: Option<Stall>,
    /// The file could not be read at launch (damaged, or from a newer version): what the
    /// person's "Remember queries" setting was is not known, so nothing is recorded until they
    /// turn it on.
    paused: bool,
    /// A change made again on top of another window's file was refused there.
    replay_dropped: bool,
    /// Reads the file's signature; the real one when `None` (a test puts its own here).
    signature_reader: Option<fn(&std::path::Path) -> Sig>,
    /// The failure of this streak was logged.
    logged: bool,
    /// The backup copies of the file are to be deleted (they hold query text).
    purge_backups: bool,
    /// When the deletion of the backup copies is tried again after it failed.
    purge_retry_at: Option<std::time::Instant>,
    /// The failure to delete them was shown (once per streak; a successful write of the file does
    /// not end it, only the deletion does).
    purge_announced: bool,
    /// A failed write was announced; no second banner until a write works.
    failing: bool,
}

/// The most recordings kept waiting for the file after a failed write.
const MAX_PENDING_RECORDS: usize = 32;

fn file_signature_stamp(path: &std::path::Path) -> Option<super::pipelines_store::DiskStamp> {
    super::pipelines_store::DiskStamp::read(path).ok()
}

/// How long a failed write waits before it is tried again with nothing having changed (the
/// cause may be gone: a full disk freed, a folder made writable).
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// What the failed write saw: the number of changes made, the size and time of the file and
/// when it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stall {
    error: HistorySaveError,
    at: std::time::Instant,
    revision: u64,
    file: Sig,
}

/// The size and time of the file: what tells a look at it from the last one.
type Sig = Option<(u64, Option<std::time::SystemTime>)>;

fn file_signature(path: &std::path::Path) -> Sig {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

impl QueryHistoryState {
    fn signature(&self, path: &std::path::Path) -> Sig {
        match self.signature_reader {
            Some(read) => read(path),
            None => file_signature(path),
        }
    }

    /// Whether finds that are run are recorded: the setting is on and the file was readable.
    pub fn recording(&self) -> bool {
        self.store.remembering() && !self.paused
    }

    /// How many changes wait for the file, and how many of them are recordings.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn pending_records(&self) -> usize {
        self.pending
            .iter()
            .filter(|op| matches!(op, Op::Record { .. }))
            .count()
    }

    fn queued(&mut self, op: Op) {
        self.pending.push(op);
        self.revision += 1;
        while self.pending_records() > MAX_PENDING_RECORDS {
            match self
                .pending
                .iter()
                .position(|o| matches!(o, Op::Record { .. }))
            {
                Some(oldest) => {
                    self.pending.remove(oldest);
                }
                None => break,
            }
        }
    }
}

impl AppState {
    /// Loads the history from `path`. A file that cannot be used is backed up and the history
    /// starts empty, with the recovery banner (an earlier banner keeps its place). An unknown
    /// config folder gives an empty history that is kept in memory only.
    pub fn with_query_history_path(self, path: ConfigPath) -> Self {
        let Ok(path) = path else {
            return self;
        };
        let backup = |p: &std::path::Path, tag: &str| {
            persistence::back_up_limited(p, tag, Some(store::MAX_FILE_BYTES as u64 + 1))
        };
        let started = store::load(&path, &backup);
        let unreadable = matches!(
            started.recovery,
            Some(Recovery::QueryHistory { .. } | Recovery::QueryHistoryNewer)
        );
        let state = Self {
            query_history: Box::new(QueryHistoryState {
                paused: unreadable,
                store: started.settings,
                path: Some(path),
                guard: started.guard,
                seen: started.seen,
                ..QueryHistoryState::default()
            }),
            ..self
        };
        match started.recovery {
            Some(recovery) => state.with_recovery_notice(recovery),
            None => state,
        }
    }

    /// Applies `op` to the history in memory and queues it for the file (written by
    /// [`Self::query_history_flush`] at the end of the step). A refused change leaves the
    /// history as it was.
    pub(super) fn query_history_apply(&mut self, op: Op) -> Result<(), SavedError> {
        let next = op.apply(&self.query_history.store)?;
        if next != self.query_history.store {
            self.query_history.store = next;
            self.query_history.queued(op);
        }
        Ok(())
    }

    /// Writes the history when a step changed it (the one hook of the dispatch loop). A step that
    /// changed nothing costs a length check. The file is checked for another window's write
    /// first; when it changed, its history is taken and this window's changes are made again on
    /// top, once. After a failure nothing is tried again until the history changed or the file
    /// did (a step that changed neither would only fail the same way, at the cost of a
    /// serialisation and a read); the failure is logged once and shown once in a banner.
    pub fn query_history_flush(&mut self) {
        self.query_history_backups_purged(std::time::Instant::now());
        if self.query_history.pending.is_empty() {
            return;
        }
        let Some(path) = self.query_history.path.clone() else {
            self.query_history.pending.clear();
            return;
        };
        // What the file was before this attempt: a failure is remembered against that, so a file
        // replaced while the attempt ran is seen as changed by the next look.
        let before = self.query_history.signature(&path);
        if let Some(stall) = &self.query_history.stalled {
            if stall.revision == self.query_history.revision && stall.file == before {
                return;
            }
        }
        let was_paused = self.query_history.paused;
        let result = self.query_history_written(&path);
        // Whatever the result: a pause that ended (another window's file was followed) must not
        // leave its notice holding the banner slot a write failure needs.
        if was_paused && !self.query_history.paused {
            self.query_history_pause_ended();
        }
        match result {
            Ok(()) => {
                // A failure that was announced is over: its banner goes (a notice stays).
                let over = self.query_history.failing
                    && matches!(
                        &self.banner,
                        Some(Banner::QueryHistoryPersist(e))
                            if !matches!(e, HistorySaveError::ReplayDropped | HistorySaveError::BackupsNotDeleted)
                    );
                if over {
                    self.banner = None;
                }
                let qh = &mut *self.query_history;
                (qh.failing, qh.logged, qh.stalled) = (false, false, None);
                if std::mem::take(&mut qh.replay_dropped) {
                    self.query_history_noticed(&HistorySaveError::ReplayDropped);
                }
            }
            Err(e) => self.query_history_failed(&e, before),
        }
    }

    /// A change to a connection that changes nothing (a delete of one that never had a query, a
    /// duplicate of one with no favourites) needs no write when the file is as this window last
    /// saw it and there is nothing in this window's store: no file is created for a history that
    /// was never used, and an unreadable one is left to be reported again. Asked before the first
    /// attempt only: after following another window's file an empty result must be written.
    fn query_history_needless(&self, path: &std::path::Path) -> bool {
        let qh = &*self.query_history;
        let only_connections = qh
            .pending
            .iter()
            .all(|op| matches!(op, Op::ForgetConnection(_) | Op::CopyConnection { .. }));
        let stamp = file_signature_stamp(path);
        // No file at all: nothing to scrub. A file that is there is always rewritten (a delete
        // must take the connection's queries out of it, whatever this window's store holds
        // after the delete) except a damaged one this window could not read: there is nothing
        // it can scrub, and the notice should come back at the next launch.
        let nothing_to_scrub = stamp == Some(super::pipelines_store::DiskStamp::default())
            || (qh.paused && stamp == Some(qh.seen));
        only_connections && qh.store.is_empty() && nothing_to_scrub
    }

    fn query_history_written(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        if self.query_history_needless(path) {
            self.query_history.pending.clear();
            return Ok(());
        }
        let first = self.query_history_try(path);
        if first != Err(HistorySaveError::ChangedOnDisk) {
            return first;
        }
        self.query_history_followed(path)?;
        self.query_history_try(path)
    }

    fn query_history_try(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        let qh = &mut *self.query_history;
        // While paused the person has not said they want queries kept: what is written says off.
        // (Only the flag: a store that was repaired by another window's file is not this one.)
        if qh.paused {
            qh.store = qh.store.flag_set(false);
        }
        let written = store::saved(path, &qh.guard, qh.seen, &qh.store)?;
        qh.store = written.store;
        qh.seen = written.stamp;
        qh.pending.clear();
        Ok(())
    }

    /// Another window wrote the file: its history becomes the base and the unwritten changes of
    /// this window are made again on top. A file that cannot be used is kept as a copy first; a
    /// copy that cannot be made blocks saving, so the file is never overwritten unseen.
    fn query_history_followed(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        let qh = &mut *self.query_history;
        match store::reloaded(path) {
            Reload::Usable(base, seen) => {
                let gone = seen == super::pipelines_store::DiskStamp::default();
                // A file that is gone says nothing about the switch: this window's stays. One
                // that is there says what the switch is (and the person's choice is known).
                let base = match gone {
                    true => base.with_remembering(qh.store.remembering()),
                    false => base,
                };
                qh.paused &= gone;
                let mut dropped = false;
                qh.store = qh.pending.iter().fold(base, |store, op| {
                    op.apply(&store).unwrap_or_else(|e| {
                        log::warn!("query history change not repeated: {}", e.error_variant());
                        dropped = true;
                        store
                    })
                });
                qh.replay_dropped |= dropped;
                qh.seen = seen;
                Ok(())
            }
            Reload::Unusable { seen, tag } => match persistence::back_up(path, tag) {
                Ok(_) => {
                    qh.seen = seen;
                    Ok(())
                }
                Err(e) => {
                    log::warn!("could not back up the query history ({tag}): {}", e.kind());
                    qh.guard = PersistGuard::blocked();
                    Err(HistorySaveError::Blocked)
                }
            },
        }
    }

    /// A write failed: no retry until something changes, one log line and one banner per streak.
    fn query_history_failed(&mut self, error: &HistorySaveError, file: Sig) {
        let qh = &mut *self.query_history;
        qh.stalled = Some(Stall {
            error: error.clone(),
            at: std::time::Instant::now(),
            revision: qh.revision,
            file,
        });
        self.query_history_announced(error);
    }

    /// A notice that is not a failure streak: shown when the banner slot is free, and touching
    /// none of the flags a streak uses (the next real failure is announced as usual).
    fn query_history_noticed(&mut self, error: &HistorySaveError) {
        log::warn!("query history notice: {error:?}");
        if self.banner.is_none() {
            self.banner = Some(Banner::QueryHistoryPersist(error.clone()));
        }
    }

    /// The write is tried again once it has been failing for [`RETRY_AFTER`], with nothing
    /// changed; gives how long to wait before looking again while one is waiting. Called every
    /// frame; costs a comparison.
    pub(super) fn query_history_tick(
        &mut self,
        now: std::time::Instant,
    ) -> Option<std::time::Duration> {
        let purge = self.query_history_purge_tick(now);
        let stall = self.query_history_stall_tick(now);
        match (purge, stall) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn query_history_stall_tick(&mut self, now: std::time::Instant) -> Option<std::time::Duration> {
        let stall = self.query_history.stalled.as_ref()?;
        // A file from a newer version stays that way until it is replaced: only its changing
        // (checked by the flush) is a reason to look again.
        if stall.error == HistorySaveError::NewerVersion {
            return None;
        }
        let at = stall.at;
        let waited = now.saturating_duration_since(at);
        if waited < RETRY_AFTER {
            return Some(RETRY_AFTER - waited);
        }
        self.query_history.stalled = None;
        self.query_history_flush();
        self.query_history.stalled.as_ref().map(|_| RETRY_AFTER)
    }

    /// The app is closing: whatever waits is tried once more, whatever failed before.
    pub(super) fn query_history_flush_now(&mut self) {
        self.query_history.stalled = None;
        self.query_history_flush();
    }

    fn query_history_announced(&mut self, error: &HistorySaveError) {
        if !std::mem::replace(&mut self.query_history.logged, true) {
            log::warn!("could not save the query history: {error:?}");
        }
        let slot_free = self.banner.is_none()
            || matches!(
                self.banner,
                Some(Banner::Recovery(Recovery::QueryHistoryResumed))
            );
        if !self.query_history.failing && slot_free {
            self.query_history.failing = true;
            self.banner = Some(Banner::QueryHistoryPersist(error.clone()));
        }
    }

    /// Deletes the backup copies of the file once the switch was turned off: they can hold the
    /// text it erased. Kept wanted until it works, and tried again after [`RETRY_AFTER`]; the
    /// person is told once, apart from the write of the file (whose success does not end this).
    fn query_history_backups_purged(&mut self, now: std::time::Instant) {
        let qh = &mut *self.query_history;
        let (true, Some(path)) = (qh.purge_backups, qh.path.clone()) else {
            return;
        };
        if qh.purge_retry_at.is_some_and(|at| now < at) {
            return;
        }
        match store::remove_backups(&path) {
            Ok(()) => {
                (qh.purge_backups, qh.purge_retry_at, qh.purge_announced) = (false, None, false);
            }
            Err(e) => {
                qh.purge_retry_at = Some(now + RETRY_AFTER);
                if !std::mem::replace(&mut qh.purge_announced, true) {
                    log::warn!("could not delete the query history backups: {:?}", e);
                    let notice = HistorySaveError::BackupsNotDeleted;
                    self.query_history_noticed(&notice);
                }
            }
        }
    }

    /// The deletion of the backups is tried again when it is due; how long until it is, while
    /// one is wanted.
    fn query_history_purge_tick(&mut self, now: std::time::Instant) -> Option<std::time::Duration> {
        if !self.query_history.purge_backups {
            return None;
        }
        self.query_history_backups_purged(now);
        let at = self.query_history.purge_retry_at?;
        self.query_history
            .purge_backups
            .then(|| at.saturating_duration_since(now))
    }
}

impl CurlybraceApp {
    /// Loads the query history from `path` (see [`AppState::with_query_history_path`]).
    pub fn with_query_history_path(mut self, path: ConfigPath) -> Self {
        self.state = std::mem::take(&mut self.state).with_query_history_path(path);
        self
    }

    /// A delete that reached the disk takes the history and favourites of the connections it
    /// removed along.
    pub(super) fn forget_queries_of_removed(&mut self, saved: bool, before: &[Uuid]) {
        if !saved {
            return;
        }
        let removed: Vec<Uuid> = before
            .iter()
            .copied()
            .filter(|id| self.state.connection(*id).is_none())
            .collect();
        for id in removed {
            self.state
                .query_history_apply_queued(Op::ForgetConnection(id));
        }
        self.state.query_history_flush();
    }

    /// A duplicate starts with the favourites of the connection it was copied from, never its
    /// history.
    pub(super) fn copy_queries_to_new(&mut self, source: Uuid, before: &[Uuid]) {
        for id in connection_ops::saved_ids(&self.state)
            .into_iter()
            .filter(|id| !before.contains(id))
        {
            let op = Op::CopyConnection {
                from: source,
                to: id,
            };
            self.state.query_history_apply_queued(op);
        }
        self.state.query_history_flush();
    }
}

impl AppState {
    /// The pause ended (the person switched on, or another window's valid file was taken as the
    /// base): the notice that said nothing is recorded is no longer true.
    fn query_history_pause_ended(&mut self) {
        let unreadable = |r: &Recovery| matches!(r, Recovery::QueryHistory { .. });
        let told = matches!(&self.banner, Some(Banner::Recovery(r)) if unreadable(r))
            || self.pending_recovery.iter().any(unreadable);
        let stale = matches!(
            self.banner,
            Some(Banner::Recovery(
                Recovery::QueryHistory { .. } | Recovery::QueryHistoryNewer
            ))
        );
        if stale {
            self.banner = None;
        }
        // One that is still waiting its turn is as stale.
        self.pending_recovery.retain(|r| {
            !matches!(
                r,
                Recovery::QueryHistory { .. } | Recovery::QueryHistoryNewer
            )
        });
        // The person was told the file could not be read and the history started empty: say
        // once that recording is on again (a failure announced later takes the slot).
        if told {
            *self = std::mem::take(self).with_recovery_notice(Recovery::QueryHistoryResumed);
        }
    }

    /// Applies `op` to this window's store when it can, and queues it either way: a change to a
    /// connection (deleted, duplicated) must reach the file even when this window's store knows
    /// nothing of it (another window wrote it, or this one is paused), where the replay on the
    /// file's own history makes it.
    fn query_history_apply_queued(&mut self, op: Op) {
        match op.apply(&self.query_history.store) {
            Ok(next) => self.query_history.store = next,
            Err(e) => log::debug!("query history change refused: {}", e.error_variant()),
        }
        self.query_history.queued(op);
    }

    /// [`Self::query_history_apply`] for a change that cannot be refused.
    fn query_history_apply_infallible(&mut self, op: Op) {
        if let Err(e) = self.query_history_apply(op) {
            log::debug!("query history change refused: {}", e.error_variant());
        }
    }
}

/// The typed text of the find bar and the options it applies, for the history. A form with a
/// problem in it is not an option that applies: it is not kept.
pub(super) fn find_text_of(find: &FindBarState) -> FindText {
    let text = &find.text;
    FindText {
        filter: text.filter.clone(),
        sort: text.sort.clone(),
        projection: text.projection.clone(),
        skip: text.skip.clone(),
        limit: text.limit.clone(),
        options: find.options.parsed().ok().cloned().unwrap_or_default(),
    }
}

/// The find bar a stored query makes: its text and options, no error.
pub(super) fn bar_of(text: &FindText) -> FindBarState {
    let options: &PipelineOptions = &text.options;
    FindBarState {
        options: OptionsForm::from_options(options),
        text: QueryText {
            filter: text.filter.clone(),
            sort: text.sort.clone(),
            projection: text.projection.clone(),
            skip: text.skip.clone(),
            limit: text.limit.clone(),
        },
        ..FindBarState::default()
    }
}

/// Records the find that was just sent (`query`) under what the find bar says, for the active
/// connection and the selected collection. Called only where a find actually goes out. When the
/// bar no longer parses to `query` (it was edited while a guard check held the find back) the
/// text is not what was sent, so nothing is recorded.
pub(super) fn recorded(mut state: AppState, query: &Query) -> AppState {
    // A find that goes out is a reason to try a failed write again, recording or not.
    state.query_history.stalled = None;
    if !state.query_history.recording() {
        return state;
    }
    let (Some(connection), Some(ns)) = (state.active, state.selected_ns().cloned()) else {
        return state;
    };
    if submit_find(&state.find).ok().as_ref() != Some(query) {
        return state;
    }
    let op = Op::Record {
        connection,
        db: ns.db,
        collection: ns.collection,
        text: Box::new(find_text_of(&state.find)),
        now: curlybrace_core::clock::now_secs(),
    };
    if let Err(e) = state.query_history_apply(op) {
        log::debug!("query not recorded: {}", e.error_variant());
    }
    state
}

/// `ToolsAction::SetRememberQueries`: the switch set, and the person's choice whatever the file
/// said at launch (it ends a pause). Off erases the history at once, in memory and in the file,
/// and deletes the backup copies of the file; favourites stay. On records from now: when another
/// window repaired the file meanwhile, the change is made again on top of that file and its
/// history is kept. When saving is off (a newer file, a failed backup) the switch applies for
/// this run and the banner says the file is not updated.
pub(super) fn remember_set(mut state: AppState, on: bool) -> AppState {
    if std::mem::take(&mut state.query_history.paused) {
        state.query_history_pause_ended();
    }
    state.query_history_apply_infallible(Op::Remember(on));
    // A person's choice, whatever the file said: written even when the setting looks the same as
    // what is in memory (the file may not say it).
    if state.query_history.pending.is_empty() {
        state.query_history.queued(Op::Remember(on));
    }
    if !on {
        state.query_history.purge_backups = true;
    }
    state
}

#[path = "query_history_actions.rs"]
mod actions;
pub(in crate::app) use actions::{on_action, stale_dialog, target};

#[cfg(test)]
#[path = "query_history_app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_history_actions_tests.rs"]
mod action_tests;

#[cfg(test)]
#[path = "query_history_log_tests.rs"]
mod log_tests;

#[cfg(test)]
#[path = "query_history_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "query_history_second_tests.rs"]
mod second_tests;
