//! Several open sessions in the shell: per-session `last_connected`, the edit dialog, and the
//! host-key queue (a prompt answers only for the connection that asked).
use super::*;
use curlybrace_core::connection::{SshAuth, SshConfig};
use curlybrace_core::runtime::Command;
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::tunnel::HostKeyDecision;
use host_key::Waiter;
use std::sync::mpsc::Sender;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

struct Rig {
    app: CurlybraceApp,
    events: UnboundedSender<Event>,
    commands: UnboundedReceiver<Command>,
    requests: Sender<HostKeyRequest>,
    ids: Vec<Uuid>,
    /// The epoch the runtime numbered each connect with, by connection index.
    epochs: Vec<u64>,
    _dir: tempfile::TempDir,
}

fn via(name: &str, ssh_host: &str) -> ConnectionConfig {
    ConnectionConfig {
        ssh: Some(SshConfig {
            host: ssh_host.into(),
            port: 22,
            username: "u".into(),
            auth: SshAuth::Password,
        }),
        ..ConnectionConfig::new(name, "mongodb://db")
    }
}

fn rig(connections: Vec<ConnectionConfig>) -> Rig {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    let (requests, request_rx) = std::sync::mpsc::channel();
    let ids: Vec<Uuid> = connections.iter().map(|c| c.id).collect();
    let dir = tempfile::tempdir().unwrap();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        connections,
        Arc::new(MemorySecretStore::default()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")))
    .with_host_key_requests(request_rx);
    Rig {
        app,
        events,
        commands,
        requests,
        epochs: vec![0; ids.len()],
        ids,
        _dir: dir,
    }
}

type Shown = Option<(String, String, Option<String>)>;

impl Rig {
    fn connect(&mut self, i: usize) {
        self.app.dispatch(Input::Connect(self.ids[i]));
        self.epochs[i] = self.app.state().epoch_counter;
    }

    fn answer_connected(&mut self, i: usize) {
        self.events
            .send(Event::Connected {
                session: self.ids[i],
                epoch: self.epochs[i],
                topology: None,
            })
            .unwrap();
        self.app.pump();
    }

    fn ask(&self, host: &str, fingerprint: &str) -> Waiter {
        let (request, waiter) = HostKeyRequest::new(host, fingerprint);
        self.requests.send(request).unwrap();
        waiter
    }

    fn shown(&mut self) -> Shown {
        self.app.pump();
        self.app
            .pending_host_key()
            .map(|(h, f, n)| (h.to_owned(), f.to_owned(), n))
    }

    fn sent_connects(&mut self) -> usize {
        let mut n = 0;
        while let Ok(command) = self.commands.try_recv() {
            n += usize::from(matches!(command, Command::Connect(_)));
        }
        n
    }
}

fn named(host: &str, fp: &str, name: &str) -> Shown {
    Some((host.into(), fp.into(), Some(name.into())))
}

#[test]
fn a_second_connect_on_an_open_connection_sends_no_command() {
    let mut r = rig(vec![via("a", "ha")]);
    r.connect(0);
    r.connect(0);
    assert_eq!(r.sent_connects(), 1);
}

#[test]
fn each_connected_session_stamps_only_its_own_last_connected() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    r.answer_connected(0);
    let stamped = |r: &Rig| -> Vec<bool> {
        r.app
            .state()
            .conn
            .connections
            .iter()
            .map(|c| c.last_connected.is_some())
            .collect()
    };
    assert_eq!(stamped(&r), vec![true, false]);
    r.answer_connected(1);
    assert_eq!(stamped(&r), vec![true, true]);
}

#[test]
fn a_background_connect_leaves_the_edit_dialog_open_and_the_active_one_closes_it() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    r.app.state_mut().conn.editing = true;
    r.answer_connected(0);
    assert!(r.app.state().conn.editing, "a background session connected");
    r.answer_connected(1);
    assert!(!r.app.state().conn.editing, "the active session connected");
}

#[test]
fn the_prompt_names_the_connection_that_asked_and_answers_only_for_it() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    assert_eq!(r.shown(), named("ha", "fa", "a"));
    r.app.answer_host_key(HostKeyDecision::Trust);
    assert_eq!(first.try_recv(), Ok(HostKeyDecision::Trust));
    assert!(second.try_recv().is_err(), "the second is still waiting");
    assert_eq!(r.shown(), named("hb", "fb", "b"));
}

#[test]
fn rejecting_the_first_leaves_the_second_answerable() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.app.answer_host_key(HostKeyDecision::Reject);
    assert_eq!(first.try_recv(), Ok(HostKeyDecision::Reject));
    assert_eq!(r.shown(), named("hb", "fb", "b"));
    r.app.answer_host_key(HostKeyDecision::TrustAndRemember);
    assert_eq!(second.try_recv(), Ok(HostKeyDecision::TrustAndRemember));
}

#[test]
fn connecting_another_connection_rejects_no_pending_prompt() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb"), via("c", "hc")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.connect(2);
    assert!(first.try_recv().is_err() && second.try_recv().is_err());
    assert_eq!(r.shown(), named("ha", "fa", "a"));
}

#[test]
fn closing_one_session_keeps_the_others_prompts_and_closing_the_last_rejects_all() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(first.try_recv().is_err() && second.try_recv().is_err());
    r.app.dispatch(Input::Disconnect(r.ids[1]));
    assert_eq!(first.try_recv(), Ok(HostKeyDecision::Reject));
    assert_eq!(second.try_recv(), Ok(HostKeyDecision::Reject));
    assert!(r.app.pending_host_key().is_none());
}

#[test]
fn an_abandoned_request_of_a_closed_session_drops_out_and_the_next_is_shown() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let _second = r.ask("hb", "fb");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    drop(first);
    assert_eq!(r.shown(), named("hb", "fb", "b"));
}

#[test]
fn two_connecting_sessions_sharing_a_host_leave_the_prompt_unnamed() {
    let mut r = rig(vec![via("a", "shared"), via("b", "shared")]);
    r.connect(0);
    r.connect(1);
    let _w = r.ask("shared", "fs");
    assert_eq!(r.shown(), Some(("shared".into(), "fs".into(), None)));
}

#[test]
fn a_connected_session_is_not_a_candidate_for_the_name() {
    let mut r = rig(vec![via("a", "shared"), via("b", "shared")]);
    r.connect(0);
    r.answer_connected(0);
    r.connect(1);
    let _w = r.ask("shared", "fs");
    assert_eq!(r.shown(), named("shared", "fs", "b"));
}

#[test]
fn a_prompt_for_an_unknown_host_has_no_name() {
    let mut r = rig(vec![via("a", "ha")]);
    r.connect(0);
    let _w = r.ask("elsewhere", "fe");
    assert_eq!(r.shown(), Some(("elsewhere".into(), "fe".into(), None)));
}

fn drain(r: &mut Rig) -> Vec<Command> {
    std::iter::from_fn(|| r.commands.try_recv().ok()).collect()
}

#[test]
fn deleting_an_open_connection_closes_its_session_and_drops_its_root() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    r.answer_connected(0);
    r.answer_connected(1);
    drain(&mut r);
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[0]));
    let sent = drain(&mut r);
    assert!(
        matches!(sent.as_slice(), [Command::Disconnect { session }] if *session == r.ids[0]),
        "{sent:?}"
    );
    assert!(!r.app.state().is_open(r.ids[0]));
    assert!(r.app.state().is_open(r.ids[1]), "the other is untouched");
    assert!(r.app.state().connection(r.ids[0]).is_none());
}

#[test]
fn deleting_a_closed_connection_sends_nothing() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    drain(&mut r);
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[1]));
    assert!(drain(&mut r).is_empty());
    assert!(r.app.state().is_open(r.ids[0]));
}

#[test]
fn a_failed_delete_closes_nothing() {
    let mut r = rig(vec![via("a", "ha")]);
    r.connect(0);
    r.answer_connected(0);
    drain(&mut r);
    r.app.state_mut().persist = persistence::PersistGuard::blocked();
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[0]));
    assert!(drain(&mut r).is_empty());
    assert!(r.app.state().is_open(r.ids[0]));
    assert!(r.app.state().connection(r.ids[0]).is_some(), "still saved");
}
