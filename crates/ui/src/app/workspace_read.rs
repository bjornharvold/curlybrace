//! Reading `workspace.json` and reading a tab's saved parts out of the state (#159).
use super::persistence::{BackupFn, PersistGuard, Recovery};
use super::settings_file::json_object;
use super::state::AppState;
use super::tabs::TabId;
use super::workspace_file::{
    MAX_FILE_BYTES, MAX_SAVED_TABS, SavedSplit, SavedTab, VERSION, WorkspaceFile, WorkspaceStartup,
    valid_namespace,
};
use crate::viewmodel::aggregate_view::AggregateView;
use crate::viewmodel::{AggregateState, CollectionTab, FindBarState, ViewMode};
use curlybrace_core::pipeline_options::PipelineOptions;
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
    pub aggregate_view: &'a AggregateView,
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
            aggregate_view: &v.aggregate_view,
        });
    }
    if state.active == Some(session) {
        return Some(TabRead {
            ns: state.sidebar.selected.as_ref(),
            sub: state.tab,
            view_mode: state.view_mode,
            find: &state.find,
            aggregate: &state.aggregate,
            aggregate_view: &state.aggregate_view,
        });
    }
    let v = state.parked.get(&session)?;
    Some(TabRead {
        ns: v.sidebar.selected.as_ref(),
        sub: v.tab,
        view_mode: v.view_mode,
        find: &v.find,
        aggregate: &v.aggregate,
        aggregate_view: &v.aggregate_view,
    })
}

enum Unusable {
    Corrupt,
    Newer,
}

#[cfg(test)]
pub(super) fn load(path: &Path, backup: BackupFn<'_>, known: &BTreeSet<Uuid>) -> WorkspaceStartup {
    load_with(path, backup, known, false)
}

/// Whether the file at `path` says tabs are reopened, and whether it may be replaced: a file
/// that is missing says yes; one that cannot be read, is not ours or is newer is left alone.
/// Never backs anything up and never keeps a tab: for when the connection list could not be read.
pub(super) fn peek_restore(path: &Path) -> (bool, bool) {
    if std::fs::metadata(path).is_ok_and(|m| !m.is_file()) {
        return (true, false);
    }
    let bytes = match read_bounded(path) {
        Ok(None) => return (true, true),
        Ok(Some(bytes)) if bytes.len() <= MAX_FILE_BYTES => bytes,
        _ => return (true, false),
    };
    let Ok(value) = json_object(&bytes) else {
        return (true, false);
    };
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(VERSION) => {
            let restore = value.get("restore").and_then(serde_json::Value::as_bool);
            (restore.unwrap_or(true), true)
        }
        _ => (true, false),
    }
}

/// [`load`]; with `keep_unknown` an entry whose connection is not in `known` is kept, waiting and
/// unverified, instead of skipped (the list of connections lost entries, so its ids prove nothing).
pub(super) fn load_with(
    path: &Path,
    backup: BackupFn<'_>,
    known: &BTreeSet<Uuid>,
    keep_unknown: bool,
) -> WorkspaceStartup {
    // Something that is not a file (a pipe would block the launch, a folder cannot be read) is
    // never opened, and never backed up: saving stays off so it is not replaced either.
    if std::fs::metadata(path).is_ok_and(|m| !m.is_file()) {
        log::warn!("the saved tabs path is not a regular file");
        return unusable(Recovery::Workspace { backed_up: false }, false);
    }
    let tag = match read_bounded(path) {
        Ok(None) => return usable(WorkspaceFile::default(), 0, false),
        Ok(Some(bytes)) => match parse(&bytes) {
            Ok((file, repaired)) => {
                return filtered(path, backup, (known, keep_unknown), file, repaired);
            }
            Err(Unusable::Newer) => return newer(),
            Err(Unusable::Corrupt) => "corrupt",
        },
        Err(_) => "unreadable",
    };
    let backed_up = backed_up(path, backup, tag);
    WorkspaceStartup {
        guard: guard_after(backed_up),
        needs_rewrite: true,
        ..unusable(Recovery::Workspace { backed_up }, backed_up)
    }
}

fn unusable(recovery: Recovery, may_save: bool) -> WorkspaceStartup {
    WorkspaceStartup {
        file: WorkspaceFile::default(),
        guard: guard_after(may_save),
        recovery: Some(recovery),
        also: None,
        needs_rewrite: false,
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

/// The file and whether bad options had to be dropped from some tab to read it: one tab's
/// invalid option costs that tab its options, not every saved tab.
fn parse(bytes: &[u8]) -> Result<(WorkspaceFile, bool), Unusable> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Unusable::Corrupt);
    }
    let mut value = json_object(bytes).map_err(|_| Unusable::Corrupt)?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v > u64::from(VERSION) => return Err(Unusable::Newer),
        Some(v) if v == u64::from(VERSION) => {}
        _ => return Err(Unusable::Corrupt),
    }
    if let Ok(file) = serde_json::from_slice(bytes) {
        return Ok((file, false));
    }
    if !without_bad_options(&mut value) {
        return Err(Unusable::Corrupt);
    }
    serde_json::from_value(value)
        .map(|file| (file, true))
        .map_err(|_| Unusable::Corrupt)
}

/// Removes the options of every tab whose options do not validate; whether any was removed.
fn without_bad_options(value: &mut serde_json::Value) -> bool {
    let Some(tabs) = value.get_mut("tabs").and_then(|t| t.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for tab in tabs {
        for part in ["find", "draft"] {
            let Some(part) = tab.get_mut(part).and_then(|p| p.as_object_mut()) else {
                continue;
            };
            let bad = part.get("options").is_some_and(|options| {
                serde_json::from_value::<PipelineOptions>(options.clone()).is_err()
            });
            if bad {
                part.remove("options");
                changed = true;
            }
        }
    }
    if changed {
        log::warn!("options of a saved tab were not valid and were dropped");
    }
    changed
}

fn newer() -> WorkspaceStartup {
    unusable(Recovery::WorkspaceNewer, false)
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
        also: None,
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

/// Keeps the entries that can be used: a known connection and a valid namespace, each within its
/// text bound, at most `MAX_SAVED_TABS` of them. The active tab and the split follow their
/// entries. An entry that is invalid is skipped and counted; one past the bound is counted apart.
fn filtered(
    path: &Path,
    backup: BackupFn<'_>,
    (known, keep_unknown): (&BTreeSet<Uuid>, bool),
    file: WorkspaceFile,
    repaired: bool,
) -> WorkspaceStartup {
    if !file.restore {
        return usable(WorkspaceFile::without_tabs(false), 0, true);
    }
    let mut kept: Vec<SavedTab> = Vec::new();
    let mut position: Vec<Option<usize>> = Vec::new();
    let (mut skipped, mut overflow) = (0, 0);
    for tab in file.tabs {
        let usable = (keep_unknown || known.contains(&tab.connection)) && valid_namespace(&tab);
        if !usable {
            position.push(None);
            skipped += 1;
        } else if kept.len() >= MAX_SAVED_TABS {
            position.push(None);
            overflow += 1;
        } else {
            position.push(Some(kept.len()));
            kept.push(tab.bounded());
        }
    }
    let moved = |at: usize| position.get(at).copied().flatten();
    let active = file.active.and_then(moved);
    let split = file.split.and_then(|s| {
        // Two panes are two tabs: a split of a tab with itself is dropped.
        let secondary = moved(s.secondary).filter(|at| Some(*at) != active)?;
        active?;
        Some(SavedSplit { secondary, ..s })
    });
    let file = WorkspaceFile {
        tabs: kept,
        active,
        split,
        ..WorkspaceFile::default()
    };
    let lost = skipped + overflow;
    let backed_up = lost == 0 || backed_up(path, backup, "skipped");
    let mut notices = [
        (skipped > 0).then_some(Recovery::WorkspaceSkipped {
            count: skipped,
            backed_up,
        }),
        (overflow > 0).then_some(Recovery::WorkspaceOverflow {
            count: overflow,
            backed_up,
        }),
    ]
    .into_iter()
    .flatten();
    WorkspaceStartup {
        file,
        guard: guard_after(backed_up),
        recovery: notices.next(),
        also: notices.next(),
        needs_rewrite: lost > 0 || repaired,
    }
}
