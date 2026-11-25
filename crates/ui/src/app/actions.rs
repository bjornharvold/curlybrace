//! Pure handlers for user actions. Each consumes the state and returns the next state plus the
//! runtime commands to send. Data commands are only produced while connected (carry item I).
use super::reducer::Out;
use super::state::{AppState, Banner, ConnStatus, EditorKey, Pending, Workspace};
use super::write_guard::{Guard, ParkedWrite, WriteKind};
use crate::viewmodel::{
    DbNode, DocAction, EditorAction, EditorState, FindAction, FindBarState, SidebarAction,
};
use curlybrace_core::crud::id_of;
use curlybrace_core::runtime::Command;
use curlybrace_core::runtime::SessionId;
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

/// Runs `f` while connected; otherwise drops the request with a banner.
pub(super) fn gated(state: AppState, f: impl FnOnce(AppState) -> Out) -> Out {
    if state.status == ConnStatus::Connected {
        return f(state);
    }
    let banner = Some(Banner::NotConnected);
    (AppState { banner, ..state }, Vec::new())
}

pub(super) fn sidebar(state: AppState, action: SidebarAction) -> Out {
    match action {
        SidebarAction::Toggle(db) => {
            let sidebar = state.sidebar.toggled(&db);
            (AppState { sidebar, ..state }, Vec::new())
        }
        SidebarAction::LoadCollections(db) => gated(state, |s| load_collections(s, db)),
        SidebarAction::Open(ns) => gated(state, |s| open(s, ns)),
        SidebarAction::OpenInNewTab(ns) => {
            let session = state.session_id();
            super::tab_actions::on_action(
                state,
                super::tab_actions::TabAction::OpenInNew { session, ns },
            )
        }
        SidebarAction::ExportDocuments(ns) => {
            let action = super::doc_export::DocExportAction::Open {
                ns,
                whole_collection: true,
            };
            super::doc_export::on_action(state, action)
        }
        SidebarAction::Refresh => gated(state, refresh),
        SidebarAction::CompareTo(ns) => gated(state, |s| super::compare::compare_to(s, ns)),
        SidebarAction::CompareDatabaseTo(db) => {
            gated(state, |s| super::compare::compare_database_to(s, db))
        }
        SidebarAction::AdminDialog(action) => super::admin_dialog_actions::on_action(state, action),
        SidebarAction::Admin(request) => super::admin_dialog_actions::requested(state, request),
        SidebarAction::Select { node, mode } => super::copy_actions::select(state, node, mode),
        SidebarAction::Copy => super::copy_actions::copy(state),
        SidebarAction::Paste(target) => super::copy_actions::paste(state, target),
        SidebarAction::Drop { clipboard, target } => {
            super::copy_drag::drop_onto(state, *clipboard, target)
        }
        SidebarAction::InSession { session, action } => in_session(state, session, *action),
    }
}

/// An action under the root of `session`. Expanding a database works on that session's own view
/// and leaves the focus where it is; everything else focuses the session first.
fn in_session(state: AppState, session: Uuid, action: SidebarAction) -> Out {
    if !state.is_open(session) {
        return (state, Vec::new());
    }
    match action {
        SidebarAction::Toggle(_) | SidebarAction::LoadCollections(_) => {
            let state = super::copy_actions::clicked(state, session, &action);
            state.with_session(session, |s| sidebar(s, action))
        }
        // Selecting, copying, pasting and dropping name their own sessions: no focus change.
        SidebarAction::Select { .. }
        | SidebarAction::Copy
        | SidebarAction::Paste(_)
        | SidebarAction::Drop { .. } => sidebar(state, action),
        other => {
            let state = super::copy_actions::clicked(state, session, &other);
            let shown = super::aggregate_counts_actions::switched_to(state, session);
            let (shown, reads) = super::tab_shown::shown_for(shown, &other);
            let (next, commands) = sidebar(shown, other);
            (next, [reads, commands].concat())
        }
    }
}

/// Marks the database expanded and fetches its collections unless a fetch is in flight.
fn load_collections(state: AppState, db: String) -> Out {
    let session = state.session_id();
    let expanded = state
        .sidebar
        .databases
        .iter()
        .any(|d| d.name == db && d.expanded);
    let sidebar = if expanded {
        state.sidebar
    } else {
        state.sidebar.toggled(&db)
    };
    let mut loading = state.loading;
    let fresh = loading.insert(db.clone());
    let commands = if fresh {
        vec![Command::ListCollections { session, db }]
    } else {
        Vec::new()
    };
    (
        AppState {
            sidebar,
            loading,
            ..state
        },
        commands,
    )
}

/// Selects the collection, resets paging to the default query and fetches the first page.
fn open(state: AppState, ns: Namespace) -> Out {
    let query = Query::default();
    let request = super::request_numbers::next();
    let command = Command::Find {
        session: state.session_id(),
        ns: ns.clone(),
        query: query.clone(),
        request,
    };
    // `pending` is deliberately kept: a mutation still in flight blocks a second submit even
    // after the user switched collections.
    let next = AppState {
        // Opening a collection means looking at it; the compare session is kept.
        workspace: Workspace::Browse,
        sidebar: state.sidebar.selecting(ns),
        find: FindBarState::default(),
        issued_query: query.clone(),
        issued_request: request,
        page_query: query,
        loading_page: true,
        page: None,
        page_key: None,
        verdict: Default::default(),
        guard: Default::default(),
        tree: state.tree.results_replaced(),
        // The draft stays; its preview ran against the collection that was open before.
        aggregate: Box::new(state.aggregate.on_collection_changed()),
        editor: None,
        ..state
    }
    .awake_current();
    // The Indexes tab belongs to one collection: it starts over for the new one (and asks for its
    // data when that tab is the one shown).
    let (next, requests) = super::index_actions::collection_opened(next);
    (next, [vec![command], requests].concat())
}

/// Re-lists databases and forgets every loaded collection list (collapsing the nodes).
fn refresh(state: AppState) -> Out {
    let mut sidebar = state.sidebar;
    sidebar.databases = sidebar
        .databases
        .into_iter()
        .map(|d| DbNode {
            expanded: false,
            collections: None,
            ..d
        })
        .collect();
    let next = AppState {
        sidebar,
        loading: Default::default(),
        ..state
    };
    let session = next.session_id();
    (next, vec![Command::ListDatabases { session }])
}

pub(super) fn find(state: AppState, action: FindAction) -> Out {
    // A global setting, not a query: it needs no connection.
    if let FindAction::SetAutomaticVerdict(automatic) = action {
        return super::explain_settings::set_automatic(state, automatic);
    }
    if let FindAction::SetGuard(guard) = action {
        return super::explain_settings::set_guard(state, guard);
    }
    // Dropping a notice sends nothing, so it needs no connection either.
    if let FindAction::GuardCancel = action {
        return (super::guard_actions::dropped(state), Vec::new());
    }
    // Pure and offline too: the dialog only reads the query bar.
    if let FindAction::CopyAsCode = action {
        let open = super::CodeAction::Open(super::CodeSource::Find);
        return super::code_export_actions::on_action(state, open);
    }
    // The History popup works on the stored queries, so it needs no connection either.
    if let FindAction::History(action) = action {
        return super::query_history_app::on_action(state, action);
    }
    // Opens the index builder; `open_with` gates it on a connection and refuses a read-only one.
    if let FindAction::CreateSuggestedIndex = action {
        return super::verdict_create::create_index(state);
    }
    // The index list for the hint picker: without a connection it is "could not list", not a
    // banner (the same as the aggregate section).
    match action {
        FindAction::HintListWanted => return super::aggregate_hint_actions::wanted(state),
        FindAction::HintListRetry => return super::aggregate_hint_actions::retry(state),
        _ => {}
    }
    gated(state, |s| match action {
        FindAction::CopyAsCode | FindAction::HintListWanted | FindAction::HintListRetry => {
            (s, Vec::new())
        }
        FindAction::Run(query) => super::guard_actions::run(s, query),
        FindAction::GuardRunAnyway => super::guard_actions::run_anyway(s),
        // Answered above, before the connection gate.
        FindAction::SetAutomaticVerdict(_)
        | FindAction::SetGuard(_)
        | FindAction::GuardCancel
        | FindAction::History(_)
        | FindAction::CreateSuggestedIndex => (s, Vec::new()),
        FindAction::Clear => {
            let s = AppState {
                find: FindBarState::default(),
                verdict: Default::default(),
                ..super::guard_actions::dropped(s)
            };
            issue_find(s, Query::default())
        }
        FindAction::Prev if s.paging().0 => {
            let query = curlybrace_core::query::prev_page(&s.page_query);
            issue_find(with_skip_text(s, query.skip), query)
        }
        FindAction::Next if s.paging().1 => {
            let query = curlybrace_core::query::next_page(&s.page_query);
            issue_find(with_skip_text(s, query.skip), query)
        }
        FindAction::Prev | FindAction::Next => (s, Vec::new()),
    })
}

/// Keeps the find bar's skip box in step with paging so a later "Find" does not jump back.
fn with_skip_text(state: AppState, skip: u64) -> AppState {
    let mut find = state.find;
    find.text.skip = skip.to_string();
    AppState { find, ..state }
}

/// Records `query` as the latest requested one and fetches it for the selected collection.
/// The displayed page (and its `page_query`) stay as they are until results are accepted.
pub(super) fn issue_find(state: AppState, query: Query) -> Out {
    // A tab keeps its collection through a lost connection: nothing is read until it is back.
    let Some(ns) = state
        .selected_ns()
        .cloned()
        .filter(|_| state.status == ConnStatus::Connected)
    else {
        return (state, Vec::new());
    };
    let request = super::request_numbers::next();
    let command = Command::Find {
        session: state.session_id(),
        ns,
        query: query.clone(),
        request,
    };
    let next = AppState {
        issued_query: query,
        issued_request: request,
        loading_page: true,
        ..state
    }
    .awake_current();
    (next, vec![command])
}

/// The documents toolbar's Export: the shown collection, narrowed by the applied query.
fn export_shown(state: AppState) -> Out {
    let Some(ns) = state.selected_ns().cloned() else {
        return (state, Vec::new());
    };
    let action = super::doc_export::DocExportAction::Open {
        ns,
        whole_collection: false,
    };
    super::doc_export::on_action(state, action)
}

pub(super) fn doc(state: AppState, action: DocAction) -> Out {
    // A projected page holds partial copies: writing one back (Edit/Clone) would drop fields.
    if state.page_is_projected() && matches!(action, DocAction::Edit(_) | DocAction::Clone(_)) {
        let banner = Some(Banner::ProjectionReadOnly);
        return (AppState { banner, ..state }, Vec::new());
    }
    match action {
        DocAction::Copy(what) => super::copy_documents::copy(state, what),
        DocAction::Bulk(kind) => {
            super::bulk_actions::on_action(state, super::BulkAction::Open(kind))
        }
        DocAction::Export => export_shown(state),
        DocAction::Edit(d) => with_editor(state, EditorState::for_edit(&d)),
        DocAction::Insert => with_editor(state, EditorState::for_new()),
        DocAction::Clone(source) => {
            mutation(state, None, WriteKind::Clone, Guard::Warn, |session, ns| {
                Command::Clone {
                    session,
                    ns,
                    source,
                }
            })
        }
        DocAction::Delete(id) => {
            let on_page = state
                .page
                .as_ref()
                .is_some_and(|p| p.docs.iter().any(|d| id_of(d).is_ok_and(|x| *x == id)));
            if !on_page {
                return (state, Vec::new());
            }
            // The delete confirmation dialog is the user's answer: no second warning.
            mutation(
                state,
                None,
                WriteKind::Delete,
                Guard::Confirmed,
                |session, ns| Command::Delete { session, ns, id },
            )
        }
    }
}

fn with_editor(state: AppState, editor: EditorState) -> Out {
    let editor = Some(editor);
    (AppState { editor, ..state }, Vec::new())
}

pub(super) fn editor(state: AppState, action: EditorAction) -> Out {
    match action {
        EditorAction::Cancel => (
            AppState {
                editor: None,
                ..state
            },
            Vec::new(),
        ),
        EditorAction::Save {
            original_id: Some(id),
            text,
        } => {
            let key = EditorKey {
                original_id: Some(id.clone()),
                text: text.clone(),
            };
            mutation(
                state,
                Some(key),
                WriteKind::Replace,
                Guard::Warn,
                |session, ns| Command::Replace {
                    session,
                    ns,
                    id,
                    json: text,
                },
            )
        }
        EditorAction::Save {
            original_id: None,
            text,
        } => {
            let key = EditorKey {
                original_id: None,
                text: text.clone(),
            };
            mutation(
                state,
                Some(key),
                WriteKind::Insert,
                Guard::Warn,
                |session, ns| Command::Insert {
                    session,
                    ns,
                    json: text,
                },
            )
        }
    }
}

/// Sends one mutating command; ignored while another mutation is in flight or a production
/// write warning is open. With `Guard::Warn` on a production connection the command is parked
/// for confirmation instead of sent.
pub(super) fn mutation(
    state: AppState,
    editor: Option<EditorKey>,
    kind: WriteKind,
    guard: Guard,
    build: impl FnOnce(SessionId, Namespace) -> Command,
) -> Out {
    gated(state, |s| {
        // Ahead of the production guard: a read-only connection never reaches the confirm modal.
        if s.active_is_read_only() {
            let banner = Some(Banner::ReadOnly);
            return (AppState { banner, ..s }, Vec::new());
        }
        let Some(ns) = s
            .selected_ns()
            .cloned()
            .filter(|_| s.pending.is_none() && s.write_warning.is_none())
        else {
            return (s, Vec::new());
        };
        let pending = Pending {
            ns: ns.clone(),
            editor,
            // The tab the write is sent from: its editor is the one its answer closes.
            tab: s.current_tab_of(s.session_id()),
        };
        let command = build(s.session_id(), ns);
        if guard == Guard::Warn && s.active_is_production() {
            let write_warning = Some(ParkedWrite {
                command,
                pending: Some(pending),
                kind,
                epoch: s.latest_requested_epoch,
            });
            return (AppState { write_warning, ..s }, Vec::new());
        }
        let next = AppState {
            pending: Some(pending),
            ..s
        };
        (
            super::index_write_actions::sent(next, &command),
            vec![command],
        )
    })
}
