//! What the app would write to `workspace.json` now, and a cheap fingerprint of it (#159).
use super::state::AppState;
use super::tabs::TabId;
use super::workspace_file::{
    MAX_SAVED_TABS, SavedDraftText, SavedFind, SavedSplit, SavedTab, VERSION, WorkspaceFile,
};
use super::workspace_read;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

/// What the state would write now: the tabs the strip shows with a collection, then the saved
/// tabs that wait (their connection has not connected, or there was no room). Pure; reads the
/// find bars and drafts but never a page, an editor, a URI or a secret.
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
    let at = |tab: Option<TabId>| live.iter().position(|(t, _)| Some(*t) == tab);
    let active_live = at(state.active_tab());
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
    WorkspaceFile {
        version: VERSION,
        restore: true,
        tabs,
        active: active.filter(|i| *i < MAX_SAVED_TABS),
        split,
    }
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
