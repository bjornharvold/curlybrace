//! The state of the paste dialog: what the person is choosing before a copy, and what the copy
//! reports while it runs and after it ended. Immutable by convention, like the rest of the state.
//! Holds keys, counts and typed errors only; document values never get here (a failed document is
//! shown by its `_id`).
use crate::viewmodel::compare::key_label;
use crate::viewmodel::typed_text_matches;
use bson::Bson;
use curlybrace_core::compare::apply::MAX_REPORTED_FAILURES;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::RunId;
use curlybrace_core::copy::engine::{CopyProgress, CopyReport};
use curlybrace_core::copy::names::{
    NameProblem, is_excluded_database, validate_database_name, validate_namespace,
};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::preview::PreviewCounts;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, WriteFailure};

/// Why a target name cannot be used. Every variant has its own message key in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum NameIssue {
    /// MongoDB would refuse the name (the Task 2 validators).
    Invalid(NameProblem),
    /// Server-owned databases are never written.
    ExcludedDatabase,
    /// A collection cannot be copied onto itself.
    SameAsSource,
    /// Another name of this paste is the same.
    Duplicate,
    /// The name is the source of another collection of this paste, in the same database of the
    /// same connection: writing it would change what that collection reads.
    CollidesWithSource,
}

impl NameIssue {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            NameIssue::Invalid(NameProblem::Empty) => "copy.name_problem.empty",
            NameIssue::Invalid(NameProblem::TooLong) => "copy.name_problem.too_long",
            NameIssue::Invalid(NameProblem::ForbiddenChar) => "copy.name_problem.forbidden_char",
            NameIssue::Invalid(NameProblem::SystemPrefix) => "copy.name_problem.system_prefix",
            NameIssue::ExcludedDatabase => "copy.name_problem.excluded_database",
            NameIssue::SameAsSource => "copy.name_problem.same_as_source",
            NameIssue::Duplicate => "copy.name_problem.duplicate",
            NameIssue::CollidesWithSource => "copy.name_problem.collides_with_source",
        }
    }
}

/// What the counts area shows for one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountRow {
    /// Not reached yet.
    Waiting,
    /// Being counted; the number of source documents read so far.
    Counting(u64),
    Counted(PreviewCounts),
    /// The compare result cap was exceeded, or the preview failed.
    Unavailable,
}

/// How a copy ended.
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Completed,
    Cancelled,
    /// `Superseded` means a connection of the copy was closed or reconnected.
    Failed(CoreError),
}

/// One collection of a running or finished copy.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyRow {
    pub target: Namespace,
    pub progress: CopyProgress,
    /// The collection's estimated document count, when the whole collection is copied.
    pub estimated_total: Option<u64>,
    /// The collection reported its exact counts.
    pub finished: bool,
    pub error: Option<CoreError>,
}

impl CopyRow {
    fn of(item: &CopyItem) -> Self {
        Self {
            target: item.target.clone(),
            progress: CopyProgress::default(),
            estimated_total: None,
            finished: false,
            error: None,
        }
    }

    /// The collection was reached: it reported progress or its exact counts.
    pub fn started(&self) -> bool {
        self.finished || self.progress != CopyProgress::default()
    }
}

/// One document the target refused, by the `_id` the core reported.
#[derive(Clone, Debug, PartialEq)]
pub struct RunFailure {
    pub item: usize,
    pub key: Bson,
    pub reason: WriteFailure,
}

/// A failed document as the report lists it: the collection, the key text and a message key.
#[derive(Clone, Debug, PartialEq)]
pub struct FailureRow {
    pub collection: String,
    pub key: String,
    pub reason_key: &'static str,
}

/// The message key of a refused write. No server code or text is ever shown.
pub fn failure_key(reason: &WriteFailure) -> &'static str {
    match reason {
        WriteFailure::Missing => "copy.failure.missing",
        WriteFailure::DuplicateKey => "copy.failure.duplicate_key",
        WriteFailure::Rejected(_) => "copy.failure.rejected",
    }
}

/// A copy that was sent to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyRun {
    pub run: RunId,
    pub rows: Vec<CopyRow>,
    pub cancel_requested: bool,
    /// `None` while it runs.
    pub outcome: Option<RunOutcome>,
    /// The first `MAX_REPORTED_FAILURES` refused documents of the whole copy.
    pub failures: Vec<RunFailure>,
    /// Some refused documents are not listed (only counted).
    pub more_failures: bool,
    /// Confirm was pressed while the counts were still being computed.
    pub counts_incomplete: bool,
}

impl CopyRun {
    pub fn new(run: RunId, counts_incomplete: bool) -> Self {
        Self {
            run,
            rows: Vec::new(),
            cancel_requested: false,
            outcome: None,
            failures: Vec::new(),
            more_failures: false,
            counts_incomplete,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.outcome.is_some()
    }

    /// Cancel may still be sent.
    pub fn can_cancel(&self) -> bool {
        !self.is_finished() && !self.cancel_requested
    }

    pub fn with_plan(self, items: &[CopyItem]) -> Self {
        Self {
            rows: items.iter().map(CopyRow::of).collect(),
            ..self
        }
    }

    pub fn with_progress(self, index: usize, progress: CopyProgress, total: Option<u64>) -> Self {
        self.with_row(index, |row| CopyRow {
            progress,
            estimated_total: total,
            ..row
        })
    }

    /// The exact counts of one collection; its refused documents join the bounded list.
    pub fn with_item_done(self, index: usize, report: CopyReport) -> Self {
        if index >= self.rows.len() {
            return self;
        }
        let room = MAX_REPORTED_FAILURES.saturating_sub(self.failures.len());
        let listed = report.failures.len().min(room);
        let hidden = report.failures.len() > listed || report.progress.failed > listed as u64;
        let mut failures = self.failures.clone();
        failures.extend(report.failures.iter().take(listed).map(|f| RunFailure {
            item: index,
            key: f.key.clone(),
            reason: f.reason,
        }));
        let error = match &report.outcome {
            curlybrace_core::copy::CopyOutcome::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let more_failures = self.more_failures || hidden;
        let next = self.with_row(index, |row| CopyRow {
            progress: report.progress,
            finished: true,
            error,
            ..row
        });
        Self {
            failures,
            more_failures,
            ..next
        }
    }

    pub fn ended(self, outcome: RunOutcome) -> Self {
        Self {
            outcome: Some(outcome),
            ..self
        }
    }

    pub fn cancelling(self) -> Self {
        Self {
            cancel_requested: true,
            ..self
        }
    }

    fn with_row(self, index: usize, change: impl FnOnce(CopyRow) -> CopyRow) -> Self {
        let Some(row) = self.rows.get(index).cloned() else {
            return self;
        };
        let mut rows = self.rows.clone();
        rows[index] = change(row);
        Self { rows, ..self }
    }

    /// A connection of the copy was closed or reconnected while it ran.
    pub fn session_lost(&self) -> bool {
        self.outcome == Some(RunOutcome::Failed(CoreError::Superseded))
    }

    /// Per collection: its name and the documents written to it, `None` when it never started.
    /// The counts are the exact ones of `ItemDone`, or the last acknowledged progress.
    pub fn written_lines(&self) -> Vec<(String, Option<u64>)> {
        self.rows
            .iter()
            .map(|row| {
                let written = row.started().then_some(row.progress.written);
                (row.target.collection.clone(), written)
            })
            .collect()
    }

    pub fn nothing_written(&self) -> bool {
        self.rows.iter().all(|row| row.progress.written == 0)
    }

    pub fn failure_rows(&self) -> Vec<FailureRow> {
        self.failures
            .iter()
            .map(|f| FailureRow {
                collection: self
                    .rows
                    .get(f.item)
                    .map(|row| row.target.collection.clone())
                    .unwrap_or_default(),
                key: key_label(&KeyTuple(vec![f.key.clone()])),
                reason_key: failure_key(&f.reason),
            })
            .collect()
    }
}

/// Where the dialog is.
#[derive(Clone, Debug, PartialEq)]
pub enum CopyPhase {
    /// The person is choosing. Nothing has been written.
    Editing,
    /// Confirm was pressed while the preview still held the runtime's one run slot: it is being
    /// cancelled and the copy starts the moment it has ended.
    Starting,
    /// The copy was sent; the same value is the report once it has an outcome.
    Running(CopyRun),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyDialog {
    /// The paste as the sidebar built it; edits are applied by [`CopyDialog::final_request`].
    pub request: CopyRequest,
    pub policy: ConflictPolicy,
    /// `(source name, target name)` per collection of a collection paste, or the one
    /// `(source database, target database)` of a database paste. The target half is editable.
    pub names: Vec<(String, String)>,
    pub with_indexes_and_options: bool,
    /// Per planned collection; `None` when unavailable (see [`CopyDialog::count_row`]).
    pub counts: Vec<Option<PreviewCounts>>,
    /// The preview is still running.
    pub counting: bool,
    pub typed: String,
    /// The collections the paste goes through; a database paste learns them from `Planned`.
    pub items: Vec<CopyItem>,
    /// Collections whose count has settled.
    pub counted: usize,
    /// Source documents read so far of the collection being counted.
    pub scanned: u64,
    /// The preview run in flight.
    pub preview_run: Option<RunId>,
    /// Names changed while a preview was in flight: a fresh one starts when that one has ended.
    pub restart_preview: bool,
    /// Why the counts could not be had, when the preview failed.
    pub preview_error: Option<CoreError>,
    pub phase: CopyPhase,
}

impl CopyDialog {
    /// The dialog for `request`: default policy, options off, nothing counted yet.
    pub fn opened(request: CopyRequest) -> Self {
        let (items, names) = match &request.scope {
            CopyScope::Collections(items) => {
                let names = items
                    .iter()
                    .map(|i| (i.source.collection.clone(), i.target.collection.clone()))
                    .collect();
                (items.clone(), names)
            }
            CopyScope::Database {
                source_db,
                target_db,
            } => (Vec::new(), vec![(source_db.clone(), target_db.clone())]),
        };
        Self {
            policy: request.policy,
            with_indexes_and_options: request.with_indexes_and_options,
            counts: vec![None; items.len()],
            request,
            names,
            counting: false,
            typed: String::new(),
            items,
            counted: 0,
            scanned: 0,
            preview_run: None,
            restart_preview: false,
            preview_error: None,
            phase: CopyPhase::Editing,
        }
    }

    /// Overwrite replaces documents, and a production target is never written casually.
    pub fn needs_typed_confirmation(&self, target_is_production: bool) -> bool {
        self.policy == ConflictPolicy::Overwrite || target_is_production
    }

    /// The typed text equals the word (trimmed, exactly), or nothing has to be typed.
    pub fn confirmation_passes(&self, target_is_production: bool) -> bool {
        if !self.needs_typed_confirmation(target_is_production) {
            return true;
        }
        typed_text_matches(&self.required_text(), &self.typed)
    }

    /// What the person types to confirm, as Compare asks for a collection or database name: the
    /// target collection for one collection, else the target database.
    pub fn required_text(&self) -> String {
        match &self.request.scope {
            CopyScope::Database { .. } => self.names.first().map(|(_, t)| t.clone()),
            CopyScope::Collections(items) => match (items.as_slice(), self.names.first()) {
                ([_], Some((_, name))) => Some(name.clone()),
                ([first, ..], _) => Some(first.target.db.clone()),
                _ => None,
            },
        }
        .unwrap_or_default()
    }

    /// Confirm may fire: the person is choosing, every name is usable and the typed word passes.
    /// Counting does not matter, and neither do unavailable counts.
    pub fn can_confirm(&self, target_is_production: bool) -> bool {
        matches!(self.phase, CopyPhase::Editing)
            && self.names_valid()
            && self.confirmation_passes(target_is_production)
    }

    pub fn names_valid(&self) -> bool {
        (0..self.names.len()).all(|i| self.name_issue(i).is_none())
    }

    /// The first rule of MongoDB's naming (Task 2 validators) the target name at `index` breaks.
    pub fn name_problem(&self, index: usize) -> Option<NameProblem> {
        let (_, name) = self.names.get(index)?;
        match &self.request.scope {
            CopyScope::Database { .. } => validate_database_name(name),
            CopyScope::Collections(items) => {
                let db = items.get(index)?.target.db.clone();
                validate_namespace(&Namespace::new(db, name.clone()))
            }
        }
        .err()
    }

    /// Why the target name at `index` blocks Confirm, if it does.
    pub fn name_issue(&self, index: usize) -> Option<NameIssue> {
        if let Some(problem) = self.name_problem(index) {
            return Some(NameIssue::Invalid(problem));
        }
        let (_, name) = self.names.get(index)?;
        if matches!(self.request.scope, CopyScope::Database { .. }) && is_excluded_database(name) {
            return Some(NameIssue::ExcludedDatabase);
        }
        if self.is_source_itself(index) {
            return Some(NameIssue::SameAsSource);
        }
        let twin = self
            .names
            .iter()
            .enumerate()
            .any(|(i, (_, other))| i != index && other == name);
        match twin {
            true => Some(NameIssue::Duplicate),
            false => self
                .collides_with_source(index)
                .then_some(NameIssue::CollidesWithSource),
        }
    }

    fn collides_with_source(&self, index: usize) -> bool {
        let (CopyScope::Collections(items), Some((_, name))) =
            (&self.request.scope, self.names.get(index))
        else {
            return false;
        };
        let Some(item) = items.get(index) else {
            return false;
        };
        self.request.source == self.request.target
            && items.iter().enumerate().any(|(j, other)| {
                j != index && other.source.db == item.target.db && other.source.collection == *name
            })
    }

    fn is_source_itself(&self, index: usize) -> bool {
        if self.request.source != self.request.target {
            return false;
        }
        let Some((source, target)) = self.names.get(index) else {
            return false;
        };
        match &self.request.scope {
            CopyScope::Database { .. } => source == target,
            CopyScope::Collections(items) => items.get(index).is_some_and(|item| {
                item.source.db == item.target.db && item.source.collection == *target
            }),
        }
    }

    /// The request as chosen: edited names, policy and options.
    pub fn final_request(&self) -> CopyRequest {
        let scope = match &self.request.scope {
            CopyScope::Database { source_db, .. } => CopyScope::Database {
                source_db: source_db.clone(),
                target_db: self
                    .names
                    .first()
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default(),
            },
            CopyScope::Collections(items) => CopyScope::Collections(
                items
                    .iter()
                    .zip(&self.names)
                    .map(|(item, (_, name))| CopyItem {
                        target: Namespace::new(item.target.db.clone(), name.clone()),
                        ..item.clone()
                    })
                    .collect(),
            ),
        };
        CopyRequest {
            scope,
            policy: self.policy,
            with_indexes_and_options: self.with_indexes_and_options,
            ..self.request.clone()
        }
    }

    pub fn with_policy(self, policy: ConflictPolicy) -> Self {
        Self { policy, ..self }
    }

    pub fn with_typed(self, typed: String) -> Self {
        Self { typed, ..self }
    }

    pub fn with_options_toggled(self) -> Self {
        Self {
            with_indexes_and_options: !self.with_indexes_and_options,
            ..self
        }
    }

    /// The target name at `index` replaced; the counts it made are stale and are dropped.
    pub fn with_name(self, index: usize, name: String) -> Self {
        let names = self
            .names
            .iter()
            .enumerate()
            .map(|(i, (source, target))| match i == index {
                true => (source.clone(), name.clone()),
                false => (source.clone(), target.clone()),
            })
            .collect();
        Self { names, ..self }.counts_cleared()
    }

    /// No counts: nothing counted, nothing counting, no preview error.
    pub fn counts_cleared(self) -> Self {
        Self {
            counts: vec![None; self.items.len()],
            counted: 0,
            scanned: 0,
            counting: false,
            preview_error: None,
            ..self
        }
    }

    /// A preview `run` was sent.
    pub fn counting_run(self, run: RunId) -> Self {
        Self {
            preview_run: Some(run),
            counting: true,
            ..self.counts_cleared()
        }
    }

    pub fn with_plan(self, items: Vec<CopyItem>) -> Self {
        Self {
            counts: vec![None; items.len()],
            items,
            counted: 0,
            scanned: 0,
            ..self
        }
    }

    pub fn with_scanned(self, index: usize, scanned: u64) -> Self {
        match index == self.counted {
            true => Self { scanned, ..self },
            false => self,
        }
    }

    pub fn with_count(self, index: usize, counts: Option<PreviewCounts>) -> Self {
        if index >= self.counts.len() {
            return self;
        }
        let mut all = self.counts.clone();
        all[index] = counts;
        Self {
            counts: all,
            counted: self.counted.max(index + 1),
            scanned: 0,
            ..self
        }
    }

    /// The preview ended (or failed with `error`).
    pub fn preview_ended(self, error: Option<CoreError>) -> Self {
        Self {
            preview_run: None,
            counting: false,
            scanned: 0,
            preview_error: error,
            ..self
        }
    }

    pub fn with_phase(self, phase: CopyPhase) -> Self {
        Self { phase, ..self }
    }

    pub fn with_restart(self, restart_preview: bool) -> Self {
        Self {
            restart_preview,
            ..self
        }
    }

    /// What the counts area shows for the collection at `index`.
    pub fn count_row(&self, index: usize) -> CountRow {
        if index < self.counted {
            return match self.counts.get(index).copied().flatten() {
                Some(counts) => CountRow::Counted(counts),
                None => CountRow::Unavailable,
            };
        }
        match (self.counting, index == self.counted) {
            (true, true) => CountRow::Counting(self.scanned),
            (true, false) => CountRow::Waiting,
            (false, _) => CountRow::Unavailable,
        }
    }

    /// The sent copy changed by `change`; a dialog that has sent none stays as it is.
    pub fn with_run(self, change: impl FnOnce(CopyRun) -> CopyRun) -> Self {
        match self.phase.clone() {
            CopyPhase::Running(run) => self.with_phase(CopyPhase::Running(change(run))),
            _ => self,
        }
    }

    /// The running or finished copy, if the dialog has sent one.
    pub fn run(&self) -> Option<&CopyRun> {
        match &self.phase {
            CopyPhase::Running(run) => Some(run),
            _ => None,
        }
    }

    /// The dialog after the session `id` closed: what has not written anything is gone; a copy
    /// that was sent stays (the runtime ends it with `Superseded` and the report says so).
    pub fn after_session_closed(self, id: uuid::Uuid) -> Option<Self> {
        let involved = self.request.source == id || self.request.target == id;
        match (&self.phase, involved) {
            (CopyPhase::Running(_), _) | (_, false) => Some(self),
            _ => None,
        }
    }
}
