//! Reading `workspace.json` and reading a tab's saved parts out of the state (#159).
#![allow(dead_code)] // wired in by the next commits
use super::persistence::{BackupFn, PersistGuard, Recovery};
use super::settings_file::json_object;
use super::state::AppState;
use super::tab_actions::MAX_TABS;
use super::tabs::TabId;
use super::workspace_file::{
    MAX_FILE_BYTES, SavedSplit, SavedTab, VERSION, WorkspaceFile, WorkspaceStartup, valid_namespace,
};
use crate::viewmodel::{AggregateState, CollectionTab, FindBarState, ViewMode};
use curlybrace_core::store::Namespace;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;

/// What a tab holds that the file keeps, borrowed from wherever the tab's fields live.
pub(super) struct TabRead<'a> {
    pub ns: Option<&'a Namespace>,
    pub sub: CollectionTab,
    pub view_mode: ViewMode,
    pub find: &'a FindBarState,
    pub aggregate: &'a AggregateState,
}

/// `tab` of `session`, read in place: the current tab of the active session from the foreground
/// fields, the current tab of another session from its parked view, any other tab from
/// `parked_tabs`. Nothing is swapped.
pub(super) fn tab_read(state: &AppState, session: Uuid, tab: TabId) -> Option<TabRead<'_>> {
    if state.tabs.current.get(&session) != Some(&tab) {
        let v = state.parked_tabs.get(&tab)?;
        return Some(TabRead {
            ns: v.ns.as_ref(),
            sub: v.tab,
            view_mode: v.view_mode,
            find: &v.find,
            aggregate: &v.aggregate,
        });
    }
    if state.active == Some(session) {
        return Some(TabRead {
            ns: state.sidebar.selected.as_ref(),
            sub: state.tab,
            view_mode: state.view_mode,
            find: &state.find,
            aggregate: &state.aggregate,
        });
    }
    let v = state.parked.get(&session)?;
    Some(TabRead {
        ns: v.sidebar.selected.as_ref(),
        sub: v.tab,
        view_mode: v.view_mode,
        find: &v.find,
        aggregate: &v.aggregate,
    })
}

enum Unusable {
    Corrupt,
    Newer,
}

pub(super) fn load(path: &Path, backup: BackupFn<'_>, known: &BTreeSet<Uuid>) -> WorkspaceStartup {
    let tag = match read_bounded(path) {
        Ok(None) => return usable(WorkspaceFile::default(), 0, false),
        Ok(Some(bytes)) => match parse(&bytes) {
            Ok(file) => return filtered(path, backup, known, file),
            Err(Unusable::Newer) => return newer(),
            Err(Unusable::Corrupt) => "corrupt",
        },
        Err(_) => "unreadable",
    };
    let backed_up = backed_up(path, backup, tag);
    WorkspaceStartup {
        file: WorkspaceFile::default(),
        guard: guard_after(backed_up),
        recovery: Some(Recovery::Workspace { backed_up }),
        needs_rewrite: true,
    }
}

/// The file's bytes: `None` when there is no file; over the bound is as good as corrupt.
fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn parse(bytes: &[u8]) -> Result<WorkspaceFile, Unusable> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Unusable::Corrupt);
    }
    let value = json_object(bytes).map_err(|_| Unusable::Corrupt)?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v > u64::from(VERSION) => return Err(Unusable::Newer),
        Some(v) if v == u64::from(VERSION) => {}
        _ => return Err(Unusable::Corrupt),
    }
    serde_json::from_value(value).map_err(|_| Unusable::Corrupt)
}

fn newer() -> WorkspaceStartup {
    WorkspaceStartup {
        file: WorkspaceFile::default(),
        guard: PersistGuard::blocked(),
        recovery: Some(Recovery::WorkspaceNewer),
        needs_rewrite: false,
    }
}

fn usable(file: WorkspaceFile, skipped: usize, backed_up: bool) -> WorkspaceStartup {
    let recovery = (skipped > 0).then_some(Recovery::WorkspaceSkipped {
        count: skipped,
        backed_up,
    });
    WorkspaceStartup {
        file,
        guard: guard_after(backed_up || skipped == 0),
        recovery,
        needs_rewrite: skipped > 0,
    }
}

fn guard_after(backed_up: bool) -> PersistGuard {
    match backed_up {
        true => PersistGuard::default(),
        false => PersistGuard::blocked(),
    }
}

fn backed_up(path: &Path, backup: BackupFn<'_>, tag: &str) -> bool {
    match backup(path, tag) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("could not back up the saved tabs ({tag}): {e}");
            false
        }
    }
}

/// Keeps the entries that can be used: a known connection and a valid namespace, at most
/// `MAX_TABS`, each within its text bound. The active tab and the split follow their entries.
fn filtered(
    path: &Path,
    backup: BackupFn<'_>,
    known: &BTreeSet<Uuid>,
    file: WorkspaceFile,
) -> WorkspaceStartup {
    if !file.restore {
        return usable(WorkspaceFile::without_tabs(false), 0, true);
    }
    let mut kept: Vec<SavedTab> = Vec::new();
    let mut position: Vec<Option<usize>> = Vec::new();
    let mut skipped = 0;
    for tab in file.tabs.into_iter().take(MAX_TABS) {
        if known.contains(&tab.connection) && valid_namespace(&tab) {
            position.push(Some(kept.len()));
            kept.push(tab.bounded());
        } else {
            position.push(None);
            skipped += 1;
        }
    }
    let moved = |at: usize| position.get(at).copied().flatten();
    let active = file.active.and_then(moved);
    let split = file.split.and_then(|s| {
        active?;
        Some(SavedSplit {
            secondary: moved(s.secondary)?,
            ..s
        })
    });
    let file = WorkspaceFile {
        tabs: kept,
        active,
        split,
        ..WorkspaceFile::default()
    };
    let backed_up = skipped == 0 || backed_up(path, backup, "skipped");
    usable(file, skipped, backed_up)
}
