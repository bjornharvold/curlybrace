//! What the app would write to `workspace.json` now, and a cheap fingerprint of it (#159).
use super::state::AppState;
use super::tabs::TabId;
use super::workspace_file::{
    MAX_FILE_BYTES, MAX_SAVED_TABS, SavedDraftText, SavedFind, SavedSplit, SavedTab, VERSION,
    WorkspaceFile,
};
use super::workspace_read;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

/// What the state would write now: the tabs the strip shows with a collection, then the saved
/// tabs that wait (their connection has not connected, or there was no room). Pure; reads the
/// find bars and drafts but never a page, an editor, a URI or a secret.
pub fn snapshot(state: &AppState) -> WorkspaceFile {
    snapshot_parts(state).file
}

/// A snapshot and what the writer needs to know about how it was made.
pub(super) struct Parts {
    pub file: WorkspaceFile,
    /// The tabs of the strip that were saved, in the order they lead the file.
    pub live: Vec<TabId>,
    /// The tab in front is one of them: no remembered front tab is needed.
    pub front_saved: bool,
}

pub(super) fn snapshot_parts(state: &AppState) -> Parts {
    if !state.restore.restore {
        return Parts {
            file: WorkspaceFile::without_tabs(false),
            live: Vec::new(),
            front_saved: false,
        };
    }
    let live: Vec<(TabId, SavedTab)> = state
        .visible_tabs()
        .into_iter()
        .filter_map(|tab| Some((tab, saved_tab(state, tab)?)))
        .collect();
    let live_count = live.len();
    let at = |tab: Option<TabId>| live.iter().position(|(t, _)| Some(*t) == tab);
    let front_saved = at(state.active_tab()).is_some();
    let active_live = at(state.active_tab()).or_else(|| at(state.restore.front_live));
    let live_split = state.tabs.split.as_ref().and_then(|s| {
        let secondary = at(Some(s.secondary))?;
        let primary = at(Some(s.primary))?;
        (primary != secondary).then_some(SavedSplit {
            secondary,
            ratio: s.ratio,
            orientation: s.orientation.into(),
            secondary_first: s.secondary_first,
        })
    });
    let mut tabs: Vec<SavedTab> = live.iter().map(|(_, tab)| tab.clone()).collect();
    tabs.extend(state.restore.dormant.values().flatten().cloned());
    tabs.truncate(MAX_SAVED_TABS);
    // The tab in front, or else the saved one that has not been reopened yet.
    let (active, split) = match active_live {
        Some(_) => (active_live, live_split),
        None => (
            waiting_active(state, live_count),
            waiting_split(state, live_count),
        ),
    };
    let mut file = WorkspaceFile {
        version: VERSION,
        restore: true,
        tabs,
        active: active.filter(|i| *i < MAX_SAVED_TABS),
        split,
    };
    size_fitted(&mut file);
    Parts {
        file,
        live: live.into_iter().map(|(tab, _)| tab).collect(),
        front_saved,
    }
}

/// The text of the largest tabs dropped (and flagged) until the file, written the way `save`
/// writes it, fits the bound: 200 tabs of 256 KiB, or of 200 stages each, would be refused as too
/// big and nothing would ever be written. Almost always a cheap upper bound shows it fits and
/// nothing is serialised; only near the bound is the file measured exactly.
fn size_fitted(file: &mut WorkspaceFile) {
    // The estimate only ever excludes what it is sure of: past half the bound the file is
    // measured exactly, so slop in the estimate can never reach `save`.
    if upper_bound(&file.tabs) <= MAX_FILE_BYTES / 2 {
        return;
    }
    fit_exact(file);
}

/// [`size_fitted`] without the shortcut: the file is measured as `save` writes it, whatever the
/// estimate says. For a write that was refused as too big anyway.
pub(super) fn refit(file: &mut WorkspaceFile) {
    fit_exact(file);
}

fn fit_exact(file: &mut WorkspaceFile) {
    loop {
        let size = pretty_len(&*file);
        if size <= MAX_FILE_BYTES {
            return;
        }
        // The tabs that take most room go first, by what each takes as JSON, not by its text.
        let mut order: Vec<(usize, usize)> = file
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.has_text())
            .map(|(i, tab)| (pretty_len(tab), i))
            .collect();
        order.sort_by_key(|(size, i)| (std::cmp::Reverse(*size), *i));
        if order.is_empty() {
            // Nothing left to take text from: the last tabs are cut (there is room for none
            // of this in a file this size), deterministically, rather than never writing.
            file.tabs.pop();
            file.active = file.active.filter(|at| *at < file.tabs.len());
            file.split = file.split.filter(|s| s.secondary < file.tabs.len());
            if file.tabs.is_empty() {
                return;
            }
            continue;
        }
        let mut need = size - MAX_FILE_BYTES;
        for (_, i) in order {
            if need == 0 {
                break;
            }
            let before = pretty_len(&file.tabs[i]);
            file.tabs[i] = file.tabs[i].clone().without_text();
            need = need.saturating_sub(before.saturating_sub(pretty_len(&file.tabs[i])));
        }
    }
}

/// The bytes `value` takes as pretty JSON, the way the file is written.
fn pretty_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    #[cfg(test)]
    MEASURED.with(|count| count.set(count.get() + 1));
    serde_json::to_vec_pretty(value).map_or(0, |bytes| bytes.len())
}

#[cfg(test)]
thread_local! {
    /// How many times a value was serialised to be measured, in this thread.
    pub(super) static MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A bound the file cannot exceed, worked out by counting (escaped text, each stage's lines at
/// the deepest indentation, a generous allowance per tab) without serialising anything.
fn upper_bound(tabs: &[SavedTab]) -> usize {
    let escaped = |text: &str| -> usize {
        text.bytes()
            .map(|b| match b {
                b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
                0..=0x1f => 6,
                _ => 1,
            })
            .sum()
    };
    let per_tab = |tab: &SavedTab| -> usize {
        let find = tab.find.as_ref().map_or(0, |f| {
            [&f.filter, &f.sort, &f.projection, &f.skip, &f.limit]
                .iter()
                .map(|t| escaped(t))
                .sum::<usize>()
        });
        let draft = tab.draft.as_ref().map_or(0, |d| {
            d.stages
                .iter()
                .map(|s| 90 + escaped(&s.text))
                .sum::<usize>()
        });
        // Options of the find bar and of the draft alike, one line per key once indented: a
        // key pattern takes four times its compact length, and more with empty keys.
        let options = 5 * tab.options_len();
        1024 + escaped(&tab.db) + escaped(&tab.collection) + find + draft + options
    };
    tabs.iter().map(per_tab).sum::<usize>() + 1024
}

/// Where the saved tab `index` of `connection`'s waiting group sits in the list `snapshot`
/// builds.
fn waiting_position(
    state: &AppState,
    live_count: usize,
    connection: Uuid,
    index: usize,
) -> Option<usize> {
    state.restore.dormant.get(&connection)?.get(index)?;
    let before: usize = state
        .restore
        .dormant
        .range(..connection)
        .map(|(_, tabs)| tabs.len())
        .sum();
    Some(live_count + before + index)
}

fn waiting_active(state: &AppState, live_count: usize) -> Option<usize> {
    let (connection, index) = state.restore.active_hint?;
    waiting_position(state, live_count, connection, index)
}

/// The saved split whose panes are both waiting.
fn waiting_split(state: &AppState, live_count: usize) -> Option<SavedSplit> {
    let (connection, split) = state.restore.split_hint?;
    let secondary = waiting_position(state, live_count, connection, split.secondary)?;
    waiting_active(state, live_count)?;
    Some(SavedSplit { secondary, ..split })
}

/// Whether `tab` is one that is saved: it shows a collection of a saved connection.
pub(super) fn is_saved_tab(state: &AppState, tab: TabId) -> bool {
    let known = state
        .tabs
        .owner
        .get(&tab)
        .is_some_and(|session| state.connection(*session).is_some());
    known && state.tab_ns(tab).is_some()
}

/// `tab` as it is saved: its namespace, choices and text. `None` for a tab without a collection
/// and for one of a session that is not a saved connection.
pub(super) fn saved_tab(state: &AppState, tab: TabId) -> Option<SavedTab> {
    let session = state.tabs.owner.get(&tab).copied()?;
    state.connection(session)?;
    let read = workspace_read::tab_read(state, session, tab)?;
    let ns = read.ns?;
    let (find, draft) = (
        SavedFind::of(read.find),
        SavedDraftText::of(read.aggregate, read.aggregate_view.pending.as_ref()),
    );
    // A restored tab whose text was dropped stays flagged until something is typed in it.
    // A restored tab whose text was dropped stays flagged until it runs. A running tab the fit
    // dropped is flagged by the writer, so it clears itself when the text is gone or fits.
    let still_dropped = state.tabs.restored.contains(&tab)
        && state.tabs.text_dropped.contains(&tab)
        && find.is_none()
        && draft.is_none();
    let options_dropped = state.tabs.options_dropped.contains(&tab)
        && find.as_ref().is_none_or(|f| f.options.is_empty())
        && draft.as_ref().is_none_or(|d| d.options.is_empty());
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
            options_dropped,
        }
        .bounded(),
    )
}

/// A hash of everything `snapshot` reads, built without cloning a string or parsing a draft, so
/// a frame in which nothing changed costs this and no snapshot.
pub(super) fn fingerprint(state: &AppState) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.restore.restore.hash(&mut h);
    if !state.restore.restore {
        return h.finish();
    }
    state.active_tab().hash(&mut h);
    state.tabs.order.hash(&mut h);
    if let Some(s) = &state.tabs.split {
        (s.primary, s.secondary, s.ratio.to_bits(), s.secondary_first).hash(&mut h);
        std::mem::discriminant(&s.orientation).hash(&mut h);
    }
    for tab in state.visible_tabs() {
        tab.hash(&mut h);
        state.tabs.text_dropped.contains(&tab).hash(&mut h);
        state.tabs.options_dropped.contains(&tab).hash(&mut h);
        let Some(session) = state.tabs.owner.get(&tab).copied() else {
            continue;
        };
        session.hash(&mut h);
        if let Some(read) = workspace_read::tab_read(state, session, tab) {
            read.ns.map(|ns| (&ns.db, &ns.collection)).hash(&mut h);
            std::mem::discriminant(&read.sub).hash(&mut h);
            std::mem::discriminant(&read.view_mode).hash(&mut h);
            hash_find(read.find, &mut h);
            hash_draft(&read, &mut h);
        }
    }
    for (id, tabs) in &state.restore.dormant {
        id.hash(&mut h);
        for tab in tabs {
            (
                &tab.db,
                &tab.collection,
                std::mem::discriminant(&tab.sub_tab),
            )
                .hash(&mut h);
        }
    }
    state.restore.active_hint.hash(&mut h);
    state.restore.front_live.hash(&mut h);
    h.finish()
}

fn hash_find(find: &crate::viewmodel::FindBarState, h: &mut impl Hasher) {
    let t = &find.text;
    (&t.filter, &t.sort, &t.projection, &t.skip, &t.limit).hash(h);
    if let Ok(options) = find.options.parsed() {
        hash_options(options, h);
    }
}

fn hash_draft(read: &workspace_read::TabRead<'_>, h: &mut impl Hasher) {
    let draft = read.aggregate;
    for stage in &draft.stages {
        (&stage.text, stage.enabled).hash(h);
    }
    std::mem::discriminant(&draft.mode).hash(h);
    // In text mode the buffer is what is edited: hashed, not parsed.
    if draft.mode == crate::viewmodel::aggregate::Mode::Text {
        draft.text.hash(h);
    }
    hash_options(&draft.options, h);
    if let Some(edit) = &read.aggregate_view.pending {
        (edit.index, &edit.text).hash(h);
    }
}

fn hash_options(options: &curlybrace_core::pipeline_options::PipelineOptions, h: &mut impl Hasher) {
    if !options.is_empty() {
        serde_json::to_string(options).unwrap_or_default().hash(h);
    }
}
