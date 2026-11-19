//! What happens when a tab has just become the shown one, and the limit notice (#159).
use super::actions::issue_find;
use super::reducer::{Input, Out};
use super::state::{AppState, ConnStatus};
use super::tab_strip::TabNotice;
use super::tabs::{Freshness, TabId};
use crate::viewmodel::{CollectionTab, SidebarAction};
use curlybrace_core::runtime::Command;

/// The one step every path takes when a tab has just become the shown one (a tab switch, a
/// connection picked in the tree, a closed connection's neighbour, a successor after a close): a
/// stale tab reads the page it shows again (and its index list on Indexes), a read it already ran;
/// a dormant tab sends nothing. Stale is cleared only by a read that was sent: when none could be
/// (the connection is down, or the tab's collection is gone) the mark stays, so the tab is not
/// taken for up to date.
pub(super) fn shown_now(state: AppState) -> Out {
    let Some(tab) = state.active_tab() else {
        return (state, Vec::new());
    };
    let stale = state.freshness_of(tab) == Freshness::Stale;
    if !stale || state.status != ConnStatus::Connected {
        return (state, Vec::new());
    }
    let (state, commands) = refetch(refreshed(state, tab, false));
    // Nothing was sent (no collection to read): still out of date.
    match commands.iter().any(|c| matches!(c, Command::Find { .. })) {
        true => (state, commands),
        false => (refreshed(state, tab, true), commands),
    }
}

/// A click in a connection's tree has made its session the shown one: its tab is read if stale,
/// unless the click itself replaces what the tab shows (Open), puts a new tab in front of it
/// (OpenInNewTab) or leaves for the Compare workspace (CompareTo, CompareDatabaseTo): a read
/// before any of those would be wasted, or parked behind the new tab.
pub(super) fn shown_for(state: AppState, click: &SidebarAction) -> Out {
    match click {
        SidebarAction::Open(_)
        | SidebarAction::OpenInNewTab(_)
        | SidebarAction::CompareTo(_)
        | SidebarAction::CompareDatabaseTo(_) => (state, Vec::new()),
        _ => shown_now(state),
    }
}

/// `tab` (current) is marked stale, or no longer is.
fn refreshed(state: AppState, tab: TabId, stale: bool) -> AppState {
    AppState {
        tabs: state.tabs.clone().with_stale(tab, stale),
        ..state
    }
}

/// The foreground tab reads its page again (and its indexes when they are the ones shown).
fn refetch(state: AppState) -> Out {
    let query = match state.page.is_none() && state.loading_page {
        true => state.issued_query.clone(),
        false => state.page_query.clone(),
    };
    let (state, mut commands) = issue_find(state, query);
    match state.tab {
        CollectionTab::Indexes => {
            let (state, more) = super::index_actions::switch_tab(state, CollectionTab::Indexes);
            commands.extend(more);
            (state, commands)
        }
        CollectionTab::Documents | CollectionTab::Aggregate => (state, commands),
    }
}

/// The limit notice stays until the person does something else: any input that is a person's
/// action and not a tab action dismisses it (a tab action decides for itself in `on_action`).
/// Inputs that are the answer to something in flight (a version check, a forget, a file picked)
/// leave it: an answer arriving just after the 21st tab was refused must not take the notice away
/// before it was read. Runtime events never pass here.
pub(super) fn notice_dismissed_by(state: AppState, input: &Input) -> AppState {
    use super::tools_state::ToolsAction;
    let answer = matches!(
        input,
        Input::ForgetFinished(_)
            | Input::Tools(ToolsAction::Probed { .. } | ToolsAction::Browsed { .. })
    );
    let person = !answer && !matches!(input, Input::Tabs(_));
    match state.tabs.notice == Some(TabNotice::LimitReached) && person {
        true => AppState {
            tabs: state.tabs.clone().with_notice(None),
            ..state
        },
        false => state,
    }
}
