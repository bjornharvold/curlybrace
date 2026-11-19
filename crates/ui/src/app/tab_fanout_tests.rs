//! The sites that act on a whole session reach every tab of it (#159): a lost connection, a write
//! answered, a database that vanished, a collection dropped, a status reported (two or three tabs).
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus, Pending};
use crate::app::tabs::test_support::{
    Two, act, ev, orders, parked, two_tabs, users, with_second_session,
};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::index_write::IndexWrite;
use crate::viewmodel::{
    AdminAction, AdminKind, AdminRequest, CollectionTab, DocAction, EditorAction, SidebarAction,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, ProfilerStatus, SamplerStatus};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn lost(session: Uuid) -> Event {
    Event::Failed {
        session,
        context: "find",
        error: CoreError::Unreachable,
    }
}

/// A third tab of the session, parked, showing `ns`.
fn with_third_on(s: AppState, a: Uuid, ns: Namespace) -> (AppState, TabId) {
    let (s, third) = s.tab_opened(a);
    let third = third.expect("a third tab");
    let (s, _) = act(s, third, Input::Sidebar(SidebarAction::Open(ns)));
    (s, third)
}

/// `t` with a one-stage draft in its parked tab.
fn drafted(t: Two) -> Two {
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    Two { s, ..t }
}

#[test]
fn lost_connection_empties_every_tab_of_the_session_of_its_results_and_no_other() {
    let t = drafted(two_tabs());
    let s = t.s;
    let (s, b, b_tab) = with_second_session(s);
    let s = ev(s, lost(t.a));
    assert_eq!(s.status_of(t.a), Some(ConnStatus::Failed));
    let second = parked(&s, t.second);
    // Slice 2 (T8): a lost connection keeps what the person chose and typed, and drops what the
    // server showed. Before, the tab lost its collection too.
    assert_eq!(
        second.ns,
        Some(orders()),
        "the parked tab keeps its collection"
    );
    assert!(second.page.is_none() && !second.loading_page);
    assert_eq!(second.issued_request, 0);
    assert_eq!(
        second.aggregate.stages.len(),
        1,
        "the draft is the person's"
    );
    // The other session's tabs are untouched.
    assert_eq!(s.status_of(b), Some(ConnStatus::Connected));
    let other = parked(&s, b_tab);
    assert_eq!(other.ns, Some(orders()));
    assert_eq!(other.page.as_ref().map(|p| p.docs.len()), Some(1));
}

#[test]
fn reconnect_keeps_every_tab_s_draft() {
    let t = drafted(two_tabs());
    let s = t.s;
    let s = ev(s, lost(t.a));
    let (s, commands) = update(s, Input::Connect(t.a));
    assert!(matches!(commands.as_slice(), [Command::Connect(_)]));
    let epoch = s.epoch_counter;
    let s = ev(
        s,
        Event::Connected {
            session: t.a,
            epoch,
            topology: None,
        },
    );
    assert_eq!(s.tabs_of(t.a).len(), 2, "no tab was lost");
    assert_eq!(parked(&s, t.second).aggregate.stages.len(), 1);
}

/// An editor open in `tab`, saved there: the Insert is in flight with `pending` naming the tab.
fn saving_in(s: AppState, tab: Option<TabId>) -> AppState {
    let in_tab = |s: AppState, input| match tab {
        Some(tab) => act(s, tab, input),
        None => update(s, input),
    };
    let (s, _) = in_tab(s, Input::Doc(DocAction::Insert));
    let text = match tab {
        Some(tab) => parked(&s, tab).editor.as_ref().map(|e| e.text.clone()),
        None => s.editor.as_ref().map(|e| e.text.clone()),
    }
    .expect("an editor is open");
    let (s, commands) = in_tab(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(commands.as_slice(), [Command::Insert { .. }]));
    s
}

#[test]
fn a_write_answer_closes_the_editor_of_the_writing_tab_only() {
    let t = two_tabs();
    // Both tabs have an editor open with the very same text; only tab two saves.
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let s = saving_in(s, Some(t.second));
    assert_eq!(s.pending.as_ref().and_then(|p| p.tab), Some(t.second));
    let s = ev(
        s,
        Event::Mutated {
            session: t.a,
            ns: orders(),
        },
    );
    assert!(
        parked(&s, t.second).editor.is_none(),
        "the writing tab's editor closed"
    );
    assert!(s.editor.is_some(), "the current tab's editor stays");
    assert!(s.pending.is_none());
}

#[test]
fn a_write_answer_frees_pending_even_when_its_tab_closed() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let s = saving_in(s, Some(t.second));
    let s = s.tab_closed(t.second);
    let s = ev(
        s,
        Event::Mutated {
            session: t.a,
            ns: orders(),
        },
    );
    assert!(
        s.pending.is_none(),
        "the write is over whoever asked for it"
    );
    assert!(
        s.editor.is_some(),
        "no other tab's editor was closed for it"
    );
}

#[test]
fn a_write_marks_other_tabs_on_the_namespace_stale() {
    let t = two_tabs();
    let (s, third) = with_third_on(t.s, t.a, users());
    let s = saving_in(s, None);
    let (s, commands) = update(
        s,
        Input::Event(Event::Mutated {
            session: t.a,
            ns: users(),
        }),
    );
    assert_eq!(parked(&s, third).freshness, Freshness::Stale);
    assert_eq!(
        parked(&s, t.second).freshness,
        Freshness::Fresh,
        "another collection"
    );
    assert!(
        matches!(commands.as_slice(), [Command::Find { ns, .. }] if *ns == users()),
        "the current tab refetches at once: {commands:?}"
    );
}

#[test]
fn a_parked_writers_own_page_is_stale_after_its_write() {
    let t = two_tabs();
    let s = saving_in(t.s, Some(t.second));
    let (s, commands) = update(
        s,
        Input::Event(Event::Mutated {
            session: t.a,
            ns: orders(),
        }),
    );
    assert_eq!(parked(&s, t.second).freshness, Freshness::Stale);
    assert!(
        commands.is_empty(),
        "a parked tab refetches when it is shown: {commands:?}"
    );
}

#[test]
fn an_index_change_relists_other_tabs_on_the_namespace() {
    let t = two_tabs();
    // The current tab and a third both show `orders`, like the writing tab two.
    let (s, _) = update(t.s, Input::Sidebar(SidebarAction::Open(orders())));
    let (s, third) = with_third_on(s, t.a, orders());
    let mut s = s;
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
    let (s, commands) = update(
        s,
        Input::Event(Event::IndexChanged {
            session: t.a,
            ns: orders(),
        }),
    );
    assert!(
        parked(&s, t.second).indexes.writing.is_none(),
        "the write is settled in its tab"
    );
    assert!(s.pending.is_none());
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListIndexes { ns, .. } if *ns == orders())),
        "the current tab on the namespace lists again: {commands:?}"
    );
    assert_eq!(parked(&s, third).freshness, Freshness::Stale);
}

#[test]
fn a_dropped_collection_blanks_every_tab_that_showed_it() {
    let t = two_tabs();
    let (s, third) = with_third_on(t.s, t.a, orders());
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::Admin(AdminRequest {
            kind: AdminKind::DropCollection,
            db: "app".into(),
            collection: Some("orders".into()),
        })),
    );
    let admin = |s, a| update(s, Input::Sidebar(SidebarAction::AdminDialog(a)));
    let (s, _) = admin(s, AdminAction::Typed("orders".into()));
    let (s, commands) = admin(s, AdminAction::Confirmed);
    assert!(matches!(
        commands.as_slice(),
        [Command::DropCollection { .. }]
    ));
    let s = ev(
        s,
        Event::AdminChanged {
            session: t.a,
            db: "app".into(),
        },
    );
    // Blanked, and nobody can see a blank tab: they are swept.
    for tab in [t.second, third] {
        assert!(
            !s.tabs.owner.contains_key(&tab),
            "{tab:?} showed the dropped collection"
        );
    }
    assert_eq!(
        s.selected_ns(),
        Some(&users()),
        "the current tab showed another one"
    );
}

#[test]
fn a_vanished_database_blanks_its_tabs() {
    let t = two_tabs();
    let listed = |names: &[&str]| Event::Databases {
        session: t.a,
        names: names.iter().map(|n| n.to_string()).collect(),
    };
    let (s, _) = update(
        t.s,
        Input::Event(Event::Databases {
            session: t.a,
            names: vec!["app".into(), "more".into()],
        }),
    );
    assert_eq!(
        parked(&s, t.second).ns,
        Some(orders()),
        "a listed database keeps its tabs"
    );
    let s = ev(s, listed(&["other"]));
    assert!(!s.tabs.owner.contains_key(&t.second), "blanked and swept");
    assert_eq!(s.selected_ns(), None);
}

#[test]
fn sampler_status_reaches_every_tab() {
    let t = two_tabs();
    let s = ev(
        t.s,
        Event::Sampler {
            session: t.a,
            status: SamplerStatus::Running,
        },
    );
    assert_eq!(s.indexes.sampler, Some(SamplerStatus::Running));
    assert_eq!(
        parked(&s, t.second).indexes.sampler,
        Some(SamplerStatus::Running)
    );
}

#[test]
fn profiler_status_reaches_every_tab() {
    let t = two_tabs();
    let s = ev(
        t.s,
        Event::Profiler {
            session: t.a,
            status: ProfilerStatus::Idle,
        },
    );
    assert_eq!(s.indexes.queries.status, Some(ProfilerStatus::Idle));
    assert_eq!(
        parked(&s, t.second).indexes.queries.status,
        Some(ProfilerStatus::Idle)
    );
}

#[test]
fn a_profiling_answer_goes_to_the_tab_that_asked_and_reaches_the_tabs_on_the_database() {
    use crate::viewmodel::{IndexesAction, ProfilingView, QueriesAction};
    use curlybrace_core::store::ProfilingState;
    let t = two_tabs();
    let (s, third) = with_third_on(t.s, t.a, Namespace::new("app", "events"));
    // Tab two asks for the database's profiling level from its Queries view.
    let (s, _) = act(s, t.second, Input::Tab(CollectionTab::Indexes));
    let (s, commands) = act(
        s,
        t.second,
        Input::Indexes(IndexesAction::Queries(QueriesAction::Open)),
    );
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ProfilingState { db, .. } if db == "app")),
        "{commands:?}"
    );
    let level = ProfilingState {
        level: 1,
        slow_ms: 100,
    };
    let s = ev(
        s,
        Event::ProfilingState {
            session: t.a,
            db: "app".into(),
            outcome: Ok(level),
        },
    );
    assert_eq!(
        parked(&s, t.second).indexes.queries.profiling,
        ProfilingView::Known(level),
        "the tab that asked has the answer"
    );
    assert!(
        parked(&s, t.second).indexes.queries.in_flight.is_none(),
        "and its read is settled"
    );
    assert_eq!(
        parked(&s, third).indexes.queries.profiling,
        ProfilingView::Known(level),
        "a tab on the same database shows the level too"
    );
}

#[test]
fn a_new_tab_starts_with_the_sessions_sampler_status() {
    let t = two_tabs();
    let s = ev(
        t.s,
        Event::Sampler {
            session: t.a,
            status: SamplerStatus::Running,
        },
    );
    let (s, fresh) = s.tab_opened(t.a);
    let fresh = fresh.expect("a tab");
    assert_eq!(
        parked(&s, fresh).indexes.sampler,
        Some(SamplerStatus::Running)
    );
}
