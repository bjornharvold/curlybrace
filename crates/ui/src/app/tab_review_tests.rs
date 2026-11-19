//! Findings of the review of slice 1 (#159), each with two tabs: a lost server keeps the session
//! while any tab holds an editor, a routed answer keeps a tab's stale mark, a refused write is
//! shown in the tab that sent it, and an answer for a request nobody awaits touches no tab.
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, Banner, ConnStatus, Pending};
use crate::app::tabs::Freshness;
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, ev, orders, parked, results, two_tabs, users};
use crate::app::write_guard::WriteWarningAction;
use crate::viewmodel::aggregate_run::RunStatus;
use crate::viewmodel::index_write::IndexWrite;
use crate::viewmodel::{CollectionTab, DocAction, EditorAction, FindAction, IndexesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;

fn find_failed(a: uuid::Uuid, request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: a,
        context: "find",
        request,
        error,
    }
}

fn failed(a: uuid::Uuid, context: &'static str, error: CoreError) -> Event {
    Event::Failed {
        session: a,
        context,
        error,
    }
}

/// An insert saved in the parked tab `tab`: its editor is open and the write is in flight.
fn saving_in_parked(s: AppState, tab: TabId) -> AppState {
    let (s, _) = act(s, tab, Input::Doc(DocAction::Insert));
    let text = parked(&s, tab)
        .editor
        .as_ref()
        .expect("an editor")
        .text
        .clone();
    let (s, commands) = act(
        s,
        tab,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(commands.as_slice(), [Command::Insert { .. }]));
    s
}

// H1: one blip does not discard what is typed in any tab.

#[test]
fn a_lost_server_keeps_the_session_while_the_current_tab_has_an_editor() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let text = s.editor.as_ref().expect("an editor").text.clone();
    let s = ev(s, find_failed(t.a, t.second_find, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(s.editor.as_ref().map(|e| e.text.clone()), Some(text));
}

#[test]
fn a_lost_server_keeps_the_session_while_a_parked_tab_has_an_editor() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Doc(DocAction::Insert));
    let s = ev(s, find_failed(t.a, t.first_find, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(
        parked(&s, t.second).editor.is_some(),
        "the typed text survives"
    );
}

#[test]
fn a_lost_write_of_a_parked_tab_keeps_every_editor() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let s = saving_in_parked(s, t.second);
    let s = ev(
        s,
        Event::Failed {
            session: t.a,
            context: "insert",
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.editor.is_some() && parked(&s, t.second).editor.is_some());
}

#[test]
fn a_lost_server_with_no_editor_anywhere_still_fails_the_root() {
    let t = two_tabs();
    let s = ev(t.s, find_failed(t.a, t.second_find, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Failed);
}

// M1: a routed answer does not make a stale tab fresh.

#[test]
fn a_routed_answer_keeps_the_stale_mark() {
    let t = two_tabs();
    let s = saving_in_parked(t.s, t.second);
    let s = ev(
        s,
        Event::Mutated {
            session: t.a,
            ns: orders(),
        },
    );
    assert_eq!(parked(&s, t.second).freshness, Freshness::Stale);
    let s = ev(s, results(t.a, orders(), t.second_find, 2));
    assert!(
        parked(&s, t.second).page.is_some(),
        "the answer was applied"
    );
    assert_eq!(parked(&s, t.second).freshness, Freshness::Stale);
    let (s, commands) = act(s, t.second, Input::Tab(CollectionTab::Indexes));
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::ListIndexes { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a list request");
    assert_eq!(
        parked(&s, t.second).freshness,
        Freshness::Stale,
        "acting in it keeps it"
    );
    let s = ev(
        s,
        Event::Indexes {
            session: t.a,
            ns: orders(),
            indexes: Vec::new(),
            request,
        },
    );
    assert_eq!(parked(&s, t.second).freshness, Freshness::Stale);
}

// M2: a write's failure follows the tab it was sent from.

#[test]
fn a_refused_save_from_a_parked_tab_shows_its_error_there() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let s = saving_in_parked(s, t.second);
    let s = ev(
        s,
        Event::Failed {
            session: t.a,
            context: "insert",
            error: CoreError::Database("refused".into()),
        },
    );
    let error = parked(&s, t.second)
        .editor
        .as_ref()
        .and_then(|e| e.error.clone());
    assert_eq!(error, Some(CoreError::Database("refused".into())));
    assert!(
        s.editor.as_ref().is_some_and(|e| e.error.is_none()),
        "not in tab one"
    );
    assert!(s.pending.is_none());
}

#[test]
fn a_failed_index_write_of_a_parked_tab_settles_there() {
    let t = two_tabs();
    let mut s = t.s;
    if let Some(view) = s.parked_tabs.get_mut(&t.second) {
        view.indexes.writing = Some(IndexWrite::Create {
            name: "a_1".into(),
            replaces: None,
        });
    }
    s.pending = Some(Pending {
        ns: orders(),
        editor: None,
        tab: Some(t.second),
    });
    let s = ev(
        s,
        failed(t.a, "create_index", CoreError::Database("dup".into())),
    );
    assert!(parked(&s, t.second).indexes.writing.is_none());
    assert!(s.pending.is_none());
}

#[test]
fn a_failed_run_of_a_parked_tab_does_not_stay_running() {
    let t = two_tabs();
    let mut s = t.s;
    if let Some(view) = s.parked_tabs.get_mut(&t.second) {
        view.aggregate.run.status = RunStatus::Running;
    }
    s.pending = Some(Pending {
        ns: orders(),
        editor: None,
        tab: Some(t.second),
    });
    let s = ev(
        s,
        failed(t.a, "aggregate_run", CoreError::Database("x".into())),
    );
    assert_ne!(
        parked(&s, t.second).aggregate.run.status,
        RunStatus::Running
    );
}

// M3: an answer for a request no tab awaits is dropped by the current tab.

#[test]
fn a_closed_tabs_find_failure_touches_nothing() {
    let t = two_tabs();
    let like = t.a;
    let s = t.s.tab_closed(t.second);
    let s = ev(
        s,
        find_failed(like, t.second_find, CoreError::Database("boom".into())),
    );
    assert_eq!(s.banner, None, "no banner for a request nobody awaits");
    assert!(
        s.loading_page,
        "the current tab still waits for its own find"
    );
}

#[test]
fn a_closed_tabs_lost_server_still_fails_the_root() {
    let t = two_tabs();
    let like = t.a;
    let s = t.s.tab_closed(t.second);
    let s = ev(s, find_failed(like, t.second_find, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Failed);
}

/// Tab one (current) runs a query with an explain in flight; returns the explain's number.
fn explaining(s: AppState) -> (AppState, u64) {
    let mut s = s;
    s.explain.guard = false;
    let (s, commands) = update(
        s,
        Input::Find(FindAction::Run(Query {
            filter: bson::doc! { "n": 1 },
            ..Query::default()
        })),
    );
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::ExplainFind { request, .. } => Some(*request),
            _ => None,
        })
        .expect("an explain");
    (s, request)
}

#[test]
fn a_closed_tabs_explain_touches_nothing() {
    let t = two_tabs();
    let a = t.a;
    let s = t.s.tab_closed(t.second);
    let (s, request) = explaining(s);
    let query = s.issued_query.clone();
    let s = ev(
        s,
        Event::Explain {
            session: a,
            ns: users(),
            query,
            outcome: Err(CoreError::Database("no plan".into())),
            request: request + 1000,
        },
    );
    assert!(matches!(
        s.verdict,
        crate::app::VerdictState::Pending { .. }
    ));
    let s = ev(
        s,
        Event::RequestFailed {
            session: a,
            context: "explain_find",
            request: request + 1000,
            error: CoreError::Database("x".into()),
        },
    );
    assert!(matches!(
        s.verdict,
        crate::app::VerdictState::Pending { .. }
    ));
}

#[test]
fn a_closed_tabs_count_touches_nothing() {
    let t = two_tabs();
    let a = t.a;
    let s = t.s.tab_closed(t.second);
    let (s, commands) = update(s, Input::Indexes(IndexesAction::OpenBuilder));
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::EstimatedCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a count");
    let s = ev(
        s,
        Event::EstimatedCount {
            session: a,
            ns: users(),
            count: 9,
            request: request + 1000,
        },
    );
    let estimated = |s: &AppState| {
        s.indexes
            .builder
            .as_ref()
            .and_then(|b| b.estimated_documents)
    };
    assert_eq!(estimated(&s), None);
    let s = ev(
        s,
        Event::EstimatedCount {
            session: a,
            ns: users(),
            count: 9,
            request,
        },
    );
    assert_eq!(estimated(&s), Some(9), "its own answer still lands");
}

// L6: a confirmed write whose tab closed says so.

#[test]
fn a_confirmed_write_whose_tab_closed_says_the_tab_closed() {
    let t = two_tabs();
    let mut s = t.s;
    s.production = true;
    let (s, _) = act(s, t.second, Input::Doc(DocAction::Insert));
    let text = parked(&s, t.second)
        .editor
        .as_ref()
        .expect("an editor")
        .text
        .clone();
    let (s, _) = act(
        s,
        t.second,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    let s = s.tab_closed(t.second);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert!(commands.is_empty());
    assert_eq!(s.banner, Some(Banner::WriteNotSentTabClosed));
}
