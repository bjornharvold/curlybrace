//! `workspace.json`: the tabs to reopen at the next launch (#159).
//!
//! It holds connection ids, namespaces, the sub-tab, the view mode and the text of the find bar
//! and of the Aggregate draft, and nothing else: never a document, a result page, a URI or a
//! secret. Loading follows the rules of the other settings files (a bad file is backed up, a
//! failed backup blocks saving, a newer version is left alone); saving goes through
//! `persistence::atomic_write`, which writes a 0600 file.
#![allow(dead_code)] // wired in by the next commits
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::state::AppState;
use super::tab_actions::MAX_TABS;
use super::tabs::TabId;
use super::{ConfigPath, workspace_read};
use crate::viewmodel::split_layout::Orientation;
use crate::viewmodel::{AggregateState, CollectionTab, FindBarState, ViewMode};
use curlybrace_core::copy::names::validate_namespace;
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

/// The find bar's typed text, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedFind {
    pub filter: String,
    pub sort: String,
    pub projection: String,
    pub skip: String,
    pub limit: String,
}

/// The Aggregate draft's stages, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDraftText {
    pub stages: Vec<SavedStage>,
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
    fn of(text: &QueryText) -> Option<Self> {
        let find = Self {
            filter: text.filter.clone(),
            sort: text.sort.clone(),
            projection: text.projection.clone(),
            skip: text.skip.clone(),
            limit: text.limit.clone(),
        };
        (find != Self::default()).then_some(find)
    }

    /// The find bar this text makes: the text and nothing else.
    pub fn into_bar(self) -> FindBarState {
        FindBarState {
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
        [
            &self.filter,
            &self.sort,
            &self.projection,
            &self.skip,
            &self.limit,
        ]
        .iter()
        .map(|t| t.len())
        .sum()
    }
}

impl SavedDraftText {
    fn of(draft: &AggregateState) -> Option<Self> {
        let stages = draft.as_saved_stages();
        (!stages.is_empty()).then_some(Self { stages })
    }

    fn bytes(&self) -> usize {
        self.stages.iter().map(|s| s.text.len()).sum()
    }
}

impl SavedTab {
    pub fn namespace(&self) -> Namespace {
        Namespace::new(self.db.clone(), self.collection.clone())
    }

    /// The text this tab holds, in bytes.
    fn text_bytes(&self) -> usize {
        self.find.as_ref().map_or(0, SavedFind::bytes)
            + self.draft.as_ref().map_or(0, SavedDraftText::bytes)
    }

    /// This tab with its text dropped (and flagged) when it is over the bound.
    pub fn bounded(self) -> Self {
        match self.text_bytes() > MAX_TEXT_PER_TAB {
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
pub fn load(path: &Path, backup: BackupFn<'_>, known: &BTreeSet<Uuid>) -> WorkspaceStartup {
    workspace_read::load(path, backup, known)
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

/// What the state would write now: the tabs the strip shows with a collection, then the saved
/// tabs of connections that have not connected yet. Pure; reads the find bars and drafts but
/// never a page, an editor, a URI or a secret.
pub fn snapshot(state: &AppState) -> WorkspaceFile {
    if !state.restore.restore {
        return WorkspaceFile::without_tabs(false);
    }
    let live: Vec<(TabId, SavedTab)> = state
        .visible_tabs()
        .into_iter()
        .filter_map(|tab| Some((tab, saved_tab(state, tab)?)))
        .collect();
    let live_count = live.len();
    let active_tab = state.active_tab();
    let at = |tab: Option<TabId>| live.iter().position(|(t, _)| Some(*t) == tab);
    let split = state.tabs.split.as_ref().and_then(|s| {
        let secondary = at(Some(s.secondary))?;
        at(Some(s.primary))?;
        Some(SavedSplit {
            secondary,
            ratio: s.ratio,
            orientation: s.orientation.into(),
            secondary_first: s.secondary_first,
        })
    });
    let mut tabs: Vec<SavedTab> = live.iter().map(|(_, tab)| tab.clone()).collect();
    tabs.extend(state.restore.dormant.values().flatten().cloned());
    tabs.truncate(MAX_TABS);
    let active = at(active_tab)
        .or_else(|| dormant_active(state, live_count))
        .filter(|i| *i < tabs.len());
    WorkspaceFile {
        version: VERSION,
        restore: true,
        tabs,
        active,
        split,
    }
}

/// The saved active tab that has not been restored yet (its connection has not connected), as a
/// position in the list `snapshot` builds.
fn dormant_active(state: &AppState, live_count: usize) -> Option<usize> {
    let (connection, index) = state.restore.active_hint?;
    state.restore.dormant.get(&connection)?.get(index)?;
    let before: usize = state
        .restore
        .dormant
        .range(..connection)
        .map(|(_, tabs)| tabs.len())
        .sum();
    Some(live_count + before + index)
}

/// `tab` as it is saved: its namespace, choices and text. `None` for a tab without a collection
/// and for one of a session that is not a saved connection.
fn saved_tab(state: &AppState, tab: TabId) -> Option<SavedTab> {
    let session = state.tabs.owner.get(&tab).copied()?;
    state.connection(session)?;
    let read = workspace_read::tab_read(state, session, tab)?;
    let ns = read.ns?;
    let (find, draft) = (
        SavedFind::of(&read.find.text),
        SavedDraftText::of(read.aggregate),
    );
    // A restored tab whose text was dropped stays flagged until something is typed in it.
    let still_dropped = state.tabs.text_dropped.contains(&tab) && find.is_none() && draft.is_none();
    Some(
        SavedTab {
            connection: session,
            db: ns.db.clone(),
            collection: ns.collection.clone(),
            sub_tab: read.sub.into(),
            view_mode: read.view_mode.into(),
            find,
            draft,
            text_dropped: still_dropped,
        }
        .bounded(),
    )
}

/// Whether `tab` names a namespace MongoDB accepts.
pub(super) fn valid_namespace(tab: &SavedTab) -> bool {
    validate_namespace(&tab.namespace()).is_ok()
}

#[cfg(test)]
#[path = "workspace_file_tests.rs"]
mod tests;
