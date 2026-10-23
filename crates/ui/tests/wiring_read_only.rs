//! Read-only end to end with the real runtime and the in-memory store: a connection is saved as
//! read-only while its session is open, the lock is applied at once, and the next write is
//! refused with nothing changed, while reads keep working.
mod conn_harness;

use bson::doc;
use conn_harness::{Harness, people};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::store::Query;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::{Banner, ConnStatus};
use curlybrace_ui::viewmodel::{
    ConnectionForm, DocAction, EditorAction, FindAction, SidebarAction,
};
use curlybrace_ui::views::connections::ConnectionAction;

fn doc_count(h: &Harness) -> usize {
    h.app.state().page.as_ref().map_or(0, |p| p.docs.len())
}

fn insert(h: &mut Harness, id: i32) {
    h.app.dispatch(Input::Doc(DocAction::Insert));
    h.app.dispatch(Input::Editor(EditorAction::Save {
        original_id: None,
        text: format!(r#"{{"_id": {id}, "name": "n{id}"}}"#),
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_read_only_locks_the_open_session_and_the_next_write_is_refused() {
    let cfg = ConnectionConfig::new("Local", "mongodb://localhost");
    let id = cfg.id;
    let mut h = Harness::new(vec![cfg]);
    h.app.dispatch(Input::Connect(id));
    h.until("connected", |a| a.state().status == ConnStatus::Connected);
    h.app
        .dispatch(Input::Sidebar(SidebarAction::Open(people())));
    h.until("first page", |a| {
        a.state().page.as_ref().is_some_and(|p| p.docs.len() == 1)
    });

    // Writable: the insert goes through.
    insert(&mut h, 2);
    h.until("insert", |a| {
        a.state().page.as_ref().is_some_and(|p| p.docs.len() == 2) && a.state().pending.is_none()
    });
    assert!(!h.app.state().active_is_read_only());

    // Save the connection as read-only while the session is open.
    let mut form = ConnectionForm::from_config(h.app.state().connection(id).unwrap());
    form.password = String::new();
    form.read_only = true;
    h.app
        .dispatch_connection(ConnectionAction::Save(Box::new(form)));
    assert!(h.saved().iter().any(|c| c.id == id && c.read_only), "saved");
    assert!(h.app.state().active_is_read_only(), "locked at once");
    assert!(
        h.app.state().active_badge().read_only,
        "and the badge says so"
    );

    // The next write is refused: banner, nothing pending, nothing stored.
    insert(&mut h, 3);
    assert_eq!(h.app.state().banner, Some(Banner::ReadOnly));
    assert!(h.app.state().pending.is_none());
    h.frame();
    let refetch = Query {
        filter: doc! {},
        ..Query::default()
    };
    h.app.dispatch(Input::Find(FindAction::Run(refetch)));
    h.until("re-read", |a| {
        a.state().page.is_some() && a.state().pending.is_none()
    });
    assert_eq!(doc_count(&h), 2, "the refused insert wrote nothing");

    // Reads keep working on the locked session.
    h.app.dispatch(Input::Find(FindAction::Clear));
    h.until("read", |a| {
        a.state().page.is_some() && a.state().pending.is_none()
    });
    assert_eq!(doc_count(&h), 2);
}
