//! Where the Shell tab's built Table and Tree rows are kept: egui temp memory, one entry per
//! session and mode, plus the set of sessions that have entries.
//!
//! egui never collects these entries by itself (it only collects serialisable ones), so they are
//! dropped explicitly, and no entry outlives its value's relevance:
//! - every frame, [`evict_stale`] drops a session's entries whose run or value is no longer the
//!   one on screen, in both modes, or all of them when there is no value;
//! - `shell_result_cache::retain_sessions` (called by the app every frame) forgets every session
//!   that is no longer open, and `shell_result_cache::forget` forgets one.
//!
//! An entry fits a state when it has its run id and the same result `Arc` (and, for the tree, the
//! same expansion, `TreeExpansion::is_same`). It holds clones of those `Arc`s, so an address
//! cannot be reused by another value while it is cached.
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result::ShellResult;
use crate::viewmodel::shell_result_tree::TreeExpansion;
use eframe::egui;
use std::collections::BTreeSet;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
struct Entry<T> {
    run_id: u64,
    result: Arc<ShellResult>,
    tree: Option<TreeExpansion>,
    data: Arc<T>,
}

impl<T> Entry<T> {
    /// The same run and value (whatever the expansion).
    fn is_current(&self, run_id: u64, result: &Arc<ShellResult>) -> bool {
        self.run_id == run_id && Arc::ptr_eq(&self.result, result)
    }

    fn fits(&self, run_id: u64, result: &Arc<ShellResult>, tree: Option<&TreeExpansion>) -> bool {
        let same_tree = match (&self.tree, tree) {
            (Some(cached), Some(wanted)) => cached.is_same(wanted),
            (None, None) => true,
            _ => false,
        };
        self.is_current(run_id, result) && same_tree
    }
}

/// What a store entry holds: the Table's cells or the Tree's lines.
pub(crate) trait Stored: Clone + Send + Sync + 'static {
    fn id(session: Uuid) -> egui::Id;
}

pub(crate) fn table_id(session: Uuid) -> egui::Id {
    egui::Id::new(("shell_result_table_cache", session))
}

pub(crate) fn tree_id(session: Uuid) -> egui::Id {
    egui::Id::new(("shell_result_tree_cache", session))
}

fn sessions_id() -> egui::Id {
    egui::Id::new("shell_result_cache_sessions")
}

/// The data for `state`'s value from `session`'s entry when it fits, else built and stored;
/// `None` without a value.
pub(crate) fn cached<T: Stored>(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
    tree: Option<&TreeExpansion>,
    build: impl FnOnce() -> T,
) -> Option<Arc<T>> {
    let result = state.result.result.clone()?;
    let run_id = state.result.run_id;
    let id = T::id(session);
    let hit = ctx
        .data(|d| d.get_temp::<Entry<T>>(id))
        .filter(|entry| entry.fits(run_id, &result, tree));
    if let Some(entry) = hit {
        return Some(entry.data);
    }
    let data = Arc::new(build());
    let entry = Entry {
        run_id,
        result,
        tree: tree.cloned(),
        data: Arc::clone(&data),
    };
    ctx.data_mut(|d| d.insert_temp(id, entry));
    registered(ctx, session);
    Some(data)
}

/// The stored data of `session`, as the last frame left it.
pub(crate) fn stored<T: Stored>(ctx: &egui::Context, session: Uuid) -> Option<Arc<T>> {
    ctx.data(|d| d.get_temp::<Entry<T>>(T::id(session)))
        .map(|entry| entry.data)
}

/// Drops `session`'s entry of `T` unless it belongs to the run and value `state` shows now.
fn evict_one<T: Stored>(ctx: &egui::Context, session: Uuid, state: &ShellState) {
    let id = T::id(session);
    let Some(entry) = ctx.data(|d| d.get_temp::<Entry<T>>(id)) else {
        return;
    };
    let current = state
        .result
        .result
        .as_ref()
        .is_some_and(|result| entry.is_current(state.result.run_id, result));
    if !current {
        ctx.data_mut(|d| d.remove::<Entry<T>>(id));
    }
}

/// Drops `session`'s Table and Tree entries that no longer belong to the value `state` shows
/// (a new run, a cleared value), whichever mode is on screen.
pub(crate) fn evict_stale<A: Stored, B: Stored>(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
) {
    evict_one::<A>(ctx, session, state);
    evict_one::<B>(ctx, session, state);
}

/// Drops both entries of `session`.
pub(crate) fn forget_both<A: Stored, B: Stored>(ctx: &egui::Context, session: Uuid) {
    ctx.data_mut(|d| {
        d.remove::<Entry<A>>(A::id(session));
        d.remove::<Entry<B>>(B::id(session));
    });
    let sessions = known(ctx);
    if sessions.contains(&session) {
        let rest: BTreeSet<Uuid> = sessions.iter().copied().filter(|s| *s != session).collect();
        ctx.data_mut(|d| d.insert_temp(sessions_id(), Arc::new(rest)));
    }
}

/// The sessions that have (or had) entries and were not forgotten.
pub(crate) fn known(ctx: &egui::Context) -> Arc<BTreeSet<Uuid>> {
    ctx.data(|d| d.get_temp::<Arc<BTreeSet<Uuid>>>(sessions_id()))
        .unwrap_or_default()
}

fn registered(ctx: &egui::Context, session: Uuid) {
    let sessions = known(ctx);
    if !sessions.contains(&session) {
        let more: BTreeSet<Uuid> = sessions.iter().copied().chain([session]).collect();
        ctx.data_mut(|d| d.insert_temp(sessions_id(), Arc::new(more)));
    }
}
