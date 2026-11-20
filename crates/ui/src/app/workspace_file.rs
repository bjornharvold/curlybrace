//! `workspace.json`: the tabs to reopen at the next launch (#159).
//!
//! It holds connection ids, namespaces, the sub-tab, the view mode and the text of the find bar
//! and of the Aggregate draft, and nothing else: never a document, a result page, a URI or a
//! secret. Loading follows the rules of the other settings files (a bad file is backed up, a
//! failed backup blocks saving, a newer version is left alone); saving goes through
//! `persistence::atomic_write`, which writes a 0600 file.
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::{ConfigPath, workspace_read};
use crate::viewmodel::aggregate::Mode;
use crate::viewmodel::aggregate_options::OptionsForm;
use crate::viewmodel::aggregate_view::PendingEdit;
use crate::viewmodel::split_layout::Orientation;
use crate::viewmodel::{AggregateState, CollectionTab, FindBarState, ViewMode};
use curlybrace_core::copy::names::validate_namespace;
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::query::QueryText;
use curlybrace_core::saved_pipeline::SavedStage;
use curlybrace_core::store::Namespace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

/// The only schema this build reads and writes.
pub const VERSION: u32 = 1;
/// The most query text one tab keeps; a tab over it is saved without its text.
pub const MAX_TEXT_PER_TAB: usize = 256 * 1024;
/// The most stages one Aggregate draft keeps; a draft over it is saved without its text.
pub const MAX_STAGES_PER_DRAFT: usize = 200;
/// The most tabs the file holds: the open ones (at most `MAX_TABS`) and the saved ones still
/// waiting for a connection or for room.
pub const MAX_SAVED_TABS: usize = 200;
/// The largest file read; a bigger one is refused as corrupt.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const FILE_NAME: &str = "workspace.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedSubTab {
    Documents,
    Indexes,
    Aggregate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedViewMode {
    Tree,
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedOrientation {
    SideBySide,
    Stacked,
}

/// The find bar's typed text, as typed, and the options it applies.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedFind {
    pub filter: String,
    pub sort: String,
    pub projection: String,
    pub skip: String,
    pub limit: String,
    /// The Query options form as it applies (collation, hint, time limit, read preference,
    /// comment, disk use). Absent in files written before it was saved.
    #[serde(default, skip_serializing_if = "PipelineOptions::is_empty")]
    pub options: PipelineOptions,
}

/// The Aggregate draft: its stages as typed, the mode it was in and the options it applies.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedDraftText {
    pub stages: Vec<SavedStage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub text_mode: bool,
    #[serde(default, skip_serializing_if = "PipelineOptions::is_empty")]
    pub options: PipelineOptions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub connection: Uuid,
    pub db: String,
    pub collection: String,
    pub sub_tab: SavedSubTab,
    pub view_mode: SavedViewMode,
    /// `None` when the find bar was empty, or when its text was over the bound (`text_dropped`).
    #[serde(default)]
    pub find: Option<SavedFind>,
    #[serde(default)]
    pub draft: Option<SavedDraftText>,
    #[serde(default)]
    pub text_dropped: bool,
    /// Its saved options were not valid and were dropped when the file was read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub options_dropped: bool,
}

/// The secondary pane of a split, by its position in `tabs`; the active tab is the other pane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSplit {
    pub secondary: usize,
    pub ratio: f32,
    pub orientation: SavedOrientation,
    pub secondary_first: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    pub restore: bool,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active: Option<usize>,
    #[serde(default)]
    pub split: Option<SavedSplit>,
}

impl Default for WorkspaceFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            restore: true,
            tabs: Vec::new(),
            active: None,
            split: None,
        }
    }
}

/// What startup found.
#[derive(Debug, PartialEq)]
pub struct WorkspaceStartup {
    pub file: WorkspaceFile,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
    /// A second notice, when the file had more than one thing wrong with it.
    pub also: Vec<Recovery>,
    /// The file on disk is not what `file` says (entries were skipped or it was replaced after a
    /// backup): the first write cleans it up.
    pub needs_rewrite: bool,
}

/// `<config dir>/curlybrace/workspace.json`, beside the connection list.
pub fn default_workspace_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

impl From<ViewMode> for SavedViewMode {
    fn from(mode: ViewMode) -> Self {
        match mode {
            ViewMode::Tree => Self::Tree,
            ViewMode::Table => Self::Table,
            ViewMode::Json => Self::Json,
        }
    }
}

impl From<SavedViewMode> for ViewMode {
    fn from(mode: SavedViewMode) -> Self {
        match mode {
            SavedViewMode::Tree => Self::Tree,
            SavedViewMode::Table => Self::Table,
            SavedViewMode::Json => Self::Json,
        }
    }
}

impl From<CollectionTab> for SavedSubTab {
    fn from(tab: CollectionTab) -> Self {
        match tab {
            CollectionTab::Documents => Self::Documents,
            CollectionTab::Indexes => Self::Indexes,
            CollectionTab::Aggregate => Self::Aggregate,
        }
    }
}

impl From<SavedSubTab> for CollectionTab {
    fn from(tab: SavedSubTab) -> Self {
        match tab {
            SavedSubTab::Documents => Self::Documents,
            SavedSubTab::Indexes => Self::Indexes,
            SavedSubTab::Aggregate => Self::Aggregate,
        }
    }
}

impl From<Orientation> for SavedOrientation {
    fn from(o: Orientation) -> Self {
        match o {
            Orientation::SideBySide => Self::SideBySide,
            Orientation::Stacked => Self::Stacked,
        }
    }
}

impl From<SavedOrientation> for Orientation {
    fn from(o: SavedOrientation) -> Self {
        match o {
            SavedOrientation::SideBySide => Self::SideBySide,
            SavedOrientation::Stacked => Self::Stacked,
        }
    }
}

impl SavedFind {
    pub(super) fn of(find: &FindBarState) -> Option<Self> {
        let text = &find.text;
        let saved = Self {
            filter: text.filter.clone(),
            sort: text.sort.clone(),
            projection: text.projection.clone(),
            skip: text.skip.clone(),
            limit: text.limit.clone(),
            // A form with a problem in it is not an option that applies: it is not saved.
            options: find.options.parsed().ok().cloned().unwrap_or_default(),
        };
        (saved != Self::default()).then_some(saved)
    }

    /// The find bar this makes: the text and the options form.
    pub fn into_bar(self) -> FindBarState {
        FindBarState {
            options: OptionsForm::from_options(&self.options),
            text: QueryText {
                filter: self.filter,
                sort: self.sort,
                projection: self.projection,
                skip: self.skip,
                limit: self.limit,
            },
            ..FindBarState::default()
        }
    }

    fn bytes(&self) -> usize {
        let text: usize = [
            &self.filter,
            &self.sort,
            &self.projection,
            &self.skip,
            &self.limit,
        ]
        .iter()
        .map(|t| t.len())
        .sum();
        text + options_bytes(&self.options)
    }
}

/// What the options take in the file.
fn options_bytes(options: &PipelineOptions) -> usize {
    if options.is_empty() {
        return 0;
    }
    serde_json::to_string(options).map_or(0, |text| text.len())
}

impl SavedDraftText {
    /// The draft as saved: its stages (the text buffer when it is the one edited), with a stage
    /// typed in and not applied yet included, so the text still inside the typing pause is kept.
    pub(super) fn of(draft: &AggregateState, pending: Option<&PendingEdit>) -> Option<Self> {
        let mut stages = draft.as_saved_stages();
        if let Some(edit) = pending.filter(|_| draft.mode == Mode::Stages) {
            if let Some(stage) = stages.get_mut(edit.index) {
                stage.text = edit.text.clone();
            }
        }
        let saved = Self {
            stages,
            text_mode: draft.mode == Mode::Text,
            options: draft.options.clone(),
        };
        (!saved.stages.is_empty() || !saved.options.is_empty()).then_some(saved)
    }

    fn bytes(&self) -> usize {
        self.stages.iter().map(|s| s.text.len()).sum::<usize>() + options_bytes(&self.options)
    }
}

impl SavedTab {
    pub fn namespace(&self) -> Namespace {
        Namespace::new(self.db.clone(), self.collection.clone())
    }

    /// The text this tab holds, in bytes.
    pub(super) fn text_bytes(&self) -> usize {
        self.find.as_ref().map_or(0, SavedFind::bytes)
            + self.draft.as_ref().map_or(0, SavedDraftText::bytes)
    }

    /// Whether the tab holds any text, a draft or options to drop.
    pub(super) fn has_text(&self) -> bool {
        self.find.is_some() || self.draft.is_some()
    }

    /// What the tab's options take in the file, as compact JSON.
    pub(super) fn options_len(&self) -> usize {
        self.find.as_ref().map_or(0, |f| options_bytes(&f.options))
            + self.draft.as_ref().map_or(0, |d| options_bytes(&d.options))
    }

    /// This tab with its text dropped and flagged.
    pub(super) fn without_text(self) -> Self {
        Self {
            find: None,
            draft: None,
            text_dropped: true,
            ..self
        }
    }

    /// This tab with its text dropped (and flagged) when it is over the bound.
    pub fn bounded(self) -> Self {
        let stages = self.draft.as_ref().map_or(0, |d| d.stages.len());
        match self.text_bytes() > MAX_TEXT_PER_TAB || stages > MAX_STAGES_PER_DRAFT {
            true => Self {
                find: None,
                draft: None,
                text_dropped: true,
                ..self
            },
            false => self,
        }
    }

    /// The part of this tab that changing its text does not change.
    pub(super) fn same_shape(&self, other: &Self) -> bool {
        (self.connection, &self.db, &self.collection, self.sub_tab)
            == (
                other.connection,
                &other.db,
                &other.collection,
                other.sub_tab,
            )
            && self.view_mode == other.view_mode
            && self.text_dropped == other.text_dropped
            && self.options_dropped == other.options_dropped
    }
}

impl WorkspaceFile {
    /// Whether `other` differs from this file by typed text alone.
    pub(super) fn same_shape(&self, other: &Self) -> bool {
        self.restore == other.restore
            && self.active == other.active
            && self.split == other.split
            && self.tabs.len() == other.tabs.len()
            && self
                .tabs
                .iter()
                .zip(&other.tabs)
                .all(|(a, b)| a.same_shape(b))
    }

    /// The file with no tabs: what is written when the person turns the setting off.
    pub fn without_tabs(restore: bool) -> Self {
        Self {
            restore,
            ..Self::default()
        }
    }
}

/// Loads the file at `path`. A missing file restores nothing. A file that cannot be read, is not
/// a version-1 workspace or is over [`MAX_FILE_BYTES`] is backed up first and nothing is
/// restored; when the backup fails saving is blocked. A newer version is left alone with saving
/// blocked. Entries naming a connection not in `known`, or a namespace that fails
/// `validate_namespace`, are skipped and counted (with a backup of the original).
#[cfg(test)]
pub fn load(path: &Path, backup: BackupFn<'_>, known: &BTreeSet<Uuid>) -> WorkspaceStartup {
    workspace_read::load(path, backup, known)
}

/// [`load`] keeping the entries of unknown connections (waiting, unverified) when the list of
/// connections lost entries and `keep_unknown` says so.
pub fn load_with(
    path: &Path,
    backup: BackupFn<'_>,
    known: &BTreeSet<Uuid>,
    keep_unknown: bool,
) -> WorkspaceStartup {
    workspace_read::load_with(path, backup, known, keep_unknown)
}

/// Writes `file` (pretty JSON, 0600, atomic). Refused when `guard` blocks saving.
pub fn save(path: &Path, guard: &PersistGuard, file: &WorkspaceFile) -> Result<(), PersistError> {
    guard.check()?;
    let bytes = serde_json::to_vec_pretty(file)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PersistError::Write(std::io::ErrorKind::InvalidData));
    }
    atomic_write(path, &bytes)
}

pub use super::workspace_snapshot::snapshot;

/// Deletes the backup copies of the file at `path` (`<name>.corrupt-*`, `.unreadable-*`,
/// `.skipped-*`, `.repaired-*`) and the temp files a write left behind (`.<name>.tmp-*`): they
/// hold the query text a person asked to erase. It matches only this app's workspace names, never
/// another file of the folder. A second running instance's write in progress has its temp file
/// deleted too (the write then fails once and is tried again). Every copy is tried; the first
/// failure is reported.
pub fn remove_backups(path: &Path) -> Result<(), PersistError> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(PersistError::InvalidPath);
    };
    let name = name.to_string_lossy();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(PersistError::Write(e.kind())),
    };
    let mut failed = None;
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let is_backup = ["corrupt", "unreadable", "skipped", "repaired"]
            .iter()
            .any(|tag| file.starts_with(&format!("{name}.{tag}-")))
            // A temp file a crashed write left behind holds the same text.
            || file.starts_with(&format!(".{name}.tmp-"));
        if !is_backup {
            continue;
        }
        if let Err(e) = std::fs::remove_file(entry.path()) {
            failed.get_or_insert(e.kind());
        }
    }
    failed.map_or(Ok(()), |kind| Err(PersistError::Write(kind)))
}

/// Whether `tab` names a namespace MongoDB accepts.
pub(super) fn valid_namespace(tab: &SavedTab) -> bool {
    validate_namespace(&tab.namespace()).is_ok()
}

#[cfg(test)]
#[path = "workspace_file_tests.rs"]
mod tests;
