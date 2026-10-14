//! Reducer tests for host-key forgetting and superseded sessions.
use super::*;
use curlybrace_core::runtime::EMBEDDED_SESSION;

// ---- host key forgetting ----

fn host_key_changed_state() -> AppState {
    let mut s = AppState::new(vec![]);
    s.ssh_endpoint = Some(HostEndpoint {
        host: "bastion".into(),
        port: 22,
    });
    s.banner = Some(Banner::Failure {
        context: "connect",
        error: CoreError::SshHostKeyChanged,
    });
    s
}

#[test]
fn forget_is_offered_only_for_a_changed_host_key() {
    assert!(host_key_changed_state().forgettable_host().is_some());
    let mut other = host_key_changed_state();
    other.banner = Some(Banner::Failure {
        context: "connect",
        error: CoreError::Ssh("the ssh host key was rejected".into()),
    });
    assert!(other.forgettable_host().is_none());
    other.banner = Some(Banner::Failure {
        context: "find",
        error: CoreError::SshHostKeyChanged,
    });
    assert!(
        other.forgettable_host().is_none(),
        "only a failed connect qualifies"
    );
}

#[test]
fn a_changed_host_key_from_a_real_connect_offers_forget_for_that_connects_ssh_endpoint() {
    let cfg = ssh_cfg();
    let id = cfg.id;
    let (s, _) = run(AppState::new(vec![cfg]), Input::Connect(id));
    let (s, _) = event(
        s,
        Event::ConnectFailed {
            session: id,
            epoch: 1,
            error: CoreError::SshHostKeyChanged,
        },
    );
    let target = s.forgettable_host().expect("forget offered");
    assert_eq!((target.host.as_str(), target.port), ("bastion", 2222));
}

#[test]
fn forget_needs_a_confirmation_and_clears_the_banner_on_success() {
    let (s, _) = run(host_key_changed_state(), Input::ForgetRequested);
    assert_eq!(
        s.forget_confirm.as_ref().map(|e| e.host.as_str()),
        Some("bastion")
    );
    let (s, _) = run(s, Input::ForgetCancelled);
    assert!(s.forget_confirm.is_none() && s.banner.is_some());
    let (s, _) = run(s, Input::ForgetRequested);
    let (s, _) = run(s, Input::ForgetFinished(Ok(())));
    assert!(s.forget_confirm.is_none());
    assert_eq!(s.banner, Some(Banner::HostForgotten));
}

#[test]
fn forget_failure_shows_a_persist_banner() {
    let (s, _) = run(host_key_changed_state(), Input::ForgetRequested);
    let (s, _) = run(s, Input::ForgetFinished(Err(PersistError::Corrupt)));
    assert_eq!(s.banner, Some(Banner::Persist(PersistError::Corrupt)));
}

#[test]
fn forget_request_without_a_changed_key_does_nothing() {
    let (s, _) = run(AppState::new(vec![]), Input::ForgetRequested);
    assert!(s.forget_confirm.is_none());
}

// ---- superseded sessions ----

fn superseded(context: &'static str) -> Event {
    Event::Failed {
        session: EMBEDDED_SESSION,
        context,
        error: CoreError::Superseded,
    }
}

#[test]
fn superseded_data_failures_are_dropped_silently() {
    for context in [
        "list_databases",
        "list_collections",
        "find",
        "insert",
        "delete",
    ] {
        let mut s = browsing();
        s.loading.insert("other".into());
        s.pending = pending_from(None);
        let (s, cmds) = event(s, superseded(context));
        assert!(cmds.is_empty() && s.banner.is_none(), "{context}");
        // The new session's own in-flight markers are not touched by an old session's answer.
        assert!(
            s.loading.contains("other") && s.pending.is_some(),
            "{context}"
        );
        assert!(
            s.page.is_some() && s.status == ConnStatus::Connected,
            "{context}"
        );
    }
}

#[test]
fn connect_gives_the_new_session_a_fresh_view_and_parks_the_previous_one() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let mut s = browsing();
    s.conn.connections = vec![cfg];
    s.loading.insert("app".into());
    s.pending = pending_from(None);
    s.editor = Some(EditorState::for_new());
    s.issued_query = Query {
        skip: 50,
        ..Query::default()
    };
    s.page_query = s.issued_query.clone();
    s.find.text.filter = "{\"a\":1}".into();
    let (s, _) = run(s, Input::Connect(id));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert!(s.sidebar.databases.is_empty() && s.sidebar.selected.is_none());
    assert!(s.page.is_none() && s.page_key.is_none() && s.editor.is_none());
    assert!(s.loading.is_empty() && s.pending.is_none() && !s.loading_page);
    assert_eq!(s.issued_query, Query::default());
    assert_eq!(s.page_query, Query::default());
    assert_eq!(s.find.text, QueryText::default());
    // The previous session is parked with its data; its answers go to it, never to the new one.
    assert!(s.parked[&EMBEDDED_SESSION].page.is_some());
    let (s, _) = event(
        s,
        Event::Databases {
            session: EMBEDDED_SESSION,
            names: vec!["old".into()],
        },
    );
    assert!(s.sidebar.databases.is_empty());
    assert_eq!(s.parked[&EMBEDDED_SESSION].sidebar.databases.len(), 1);
}

// ---- projected pages are read-only ----

fn projected_browsing() -> AppState {
    let mut s = browsing();
    s.page_query = Query {
        projection: Some(doc! {"name": 1}),
        ..Query::default()
    };
    s
}

#[test]
fn edit_and_clone_are_refused_on_a_projected_page() {
    let d = mock_doc_with_id(2_i32);
    for action in [DocAction::Edit(d.clone()), DocAction::Clone(d.clone())] {
        let (s, cmds) = run(projected_browsing(), Input::Doc(action));
        assert!(cmds.is_empty());
        assert!(s.editor.is_none() && s.pending.is_none());
        assert_eq!(s.banner, Some(Banner::ProjectionReadOnly));
    }
}

#[test]
fn delete_and_insert_stay_allowed_on_a_projected_page() {
    let (s, cmds) = run(
        projected_browsing(),
        Input::Doc(DocAction::Delete(Bson::Int32(2))),
    );
    assert!(s.pending.is_some() && cmds.len() == 1);
    let (s, _) = run(projected_browsing(), Input::Doc(DocAction::Insert));
    assert!(s.editor.is_some());
}

#[test]
fn an_empty_or_missing_projection_is_not_projected() {
    for projection in [None, Some(doc! {})] {
        let mut s = browsing();
        s.page_query.projection = projection;
        let (s, _) = run(s, Input::Doc(DocAction::Edit(mock_doc_with_id(2_i32))));
        assert!(s.editor.is_some() && s.banner.is_none());
    }
}
