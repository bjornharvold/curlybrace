//! Several open sessions in the shell: per-session `last_connected`, the edit dialog, and the
//! host-key queue (a prompt answers only for the connection that asked).
use super::*;
use crate::viewmodel::compare::SessionPhase;
use crate::views::connections::TestState;
use curlybrace_core::connection::{SshAuth, SshConfig};
use curlybrace_core::runtime::Command;
use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};
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
fn closing_one_session_keeps_the_others_prompt_and_closing_the_last_rejects_all() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert_eq!(
        first.try_recv(),
        Ok(HostKeyDecision::Reject),
        "its own goes"
    );
    assert!(second.try_recv().is_err(), "the other session's stays");
    r.app.dispatch(Input::Disconnect(r.ids[1]));
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

#[test]
fn a_delete_whose_config_write_fails_still_closes_the_open_session() {
    // The secrets are removed BEFORE the config is written, so by the time the write fails the
    // credentials the session was opened from are already gone. Keeping that session alive
    // would outlive its credentials; closing it is the safe outcome. The connection leaves the
    // in-memory list and the banner tells the user the file could not be saved.
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.answer_connected(0);
    drain(&mut r);
    for kind in SecretKind::ALL {
        let key = secret_key(r.ids[0], kind);
        r.app.secrets.set(&key, "secret").unwrap();
    }
    r.app.config_path = Err(persistence::PersistError::Write(
        std::io::ErrorKind::PermissionDenied,
    ));
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[0]));
    assert!(r.app.state().connection(r.ids[0]).is_none());
    assert!(matches!(r.app.state().banner, Some(Banner::Persist(_))));
    let sent = drain(&mut r);
    assert!(
        matches!(sent.as_slice(), [Command::Disconnect { session }] if *session == r.ids[0]),
        "{sent:?}"
    );
    assert!(!r.app.state().is_open(r.ids[0]));
    for kind in SecretKind::ALL {
        let key = secret_key(r.ids[0], kind);
        assert_eq!(r.app.secrets.get(&key).unwrap(), None, "{kind:?} is gone");
    }
}

#[test]
fn deleting_the_active_session_activates_its_neighbour_and_disconnects_once() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb"), via("c", "hc")]);
    for i in 0..3 {
        r.connect(i);
        r.answer_connected(i);
    }
    r.app.dispatch(Input::Activate(r.ids[1]));
    drain(&mut r);
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[1]));
    assert_eq!(r.app.state().active, Some(r.ids[2]), "the next open one");
    let sent = drain(&mut r);
    assert!(
        matches!(sent.as_slice(), [Command::Disconnect { session }] if *session == r.ids[1]),
        "{sent:?}"
    );
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[2]));
    assert_eq!(r.app.state().active, Some(r.ids[0]), "else the previous");
    assert_eq!(drain(&mut r).len(), 1);
}

#[test]
fn deleting_a_connecting_connection_disconnects_and_ignores_its_late_answers() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    drain(&mut r);
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[0]));
    let sent = drain(&mut r);
    assert!(
        matches!(sent.as_slice(), [Command::Disconnect { session }] if *session == r.ids[0]),
        "{sent:?}"
    );
    assert!(!r.app.state().is_open(r.ids[0]));
    r.answer_connected(0);
    r.events
        .send(Event::ConnectFailed {
            session: r.ids[0],
            epoch: r.epochs[0],
            error: CoreError::Database("refused".into()),
        })
        .unwrap();
    r.app.pump();
    assert!(!r.app.state().is_open(r.ids[0]), "no resurrected root");
    assert!(r.app.state().is_open(r.ids[1]));
    assert!(r.app.state().banner.is_none());
    assert!(drain(&mut r).is_empty());
}

#[test]
fn closing_a_parked_session_rejects_only_its_question_and_the_next_is_shown() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1); // b is active, a is parked
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    assert_eq!(r.shown(), named("ha", "fa", "a"));
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert_eq!(first.try_recv(), Ok(HostKeyDecision::Reject));
    assert_eq!(r.shown(), named("hb", "fb", "b"));
    r.app.answer_host_key(HostKeyDecision::Trust);
    assert_eq!(second.try_recv(), Ok(HostKeyDecision::Trust));
}

#[test]
fn closing_the_active_session_rejects_only_its_question() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1); // b is active
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[1]));
    assert_eq!(second.try_recv(), Ok(HostKeyDecision::Reject));
    assert!(first.try_recv().is_err());
    assert_eq!(r.shown(), named("ha", "fa", "a"));
}

#[test]
fn a_question_for_a_host_two_sessions_share_survives_until_the_last_one_closes() {
    let mut r = rig(vec![via("a", "Shared"), via("b", "shared")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("shared", "fs");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(w.try_recv().is_err(), "the other may be the asker");
    assert_eq!(r.shown(), named("shared", "fs", "b"));
    r.answer_connected(1);
    // b is connected now, so closing it (not connecting) no longer protects the question.
    let w2 = r.ask("shared", "fs2");
    r.app.dispatch(Input::Disconnect(r.ids[1]));
    assert_eq!(w2.try_recv(), Ok(HostKeyDecision::Reject));
}

#[test]
fn closing_the_last_connecting_user_of_a_shared_host_rejects_its_question() {
    let mut r = rig(vec![via("a", "shared"), via("b", "shared"), via("c", "hc")]);
    r.connect(0);
    r.connect(1);
    r.connect(2);
    let w = r.ask("shared", "fs");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(w.try_recv().is_err());
    r.app.dispatch(Input::Disconnect(r.ids[1]));
    assert_eq!(w.try_recv(), Ok(HostKeyDecision::Reject));
    assert!(r.app.pending_host_key().is_none());
}

#[test]
fn a_request_not_yet_polled_is_rejected_when_its_session_closes() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("ha", "fa"); // queued on the channel, never pumped
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert_eq!(w.try_recv(), Ok(HostKeyDecision::Reject));
}

#[test]
fn deleting_an_open_connection_rejects_its_queued_question() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let first = r.ask("ha", "fa");
    let second = r.ask("hb", "fb");
    r.shown();
    r.app
        .dispatch_connection(ConnectionAction::Delete(r.ids[0]));
    assert_eq!(first.try_recv(), Ok(HostKeyDecision::Reject));
    assert!(second.try_recv().is_err());
    assert_eq!(r.shown(), named("hb", "fb", "b"));
}

#[test]
fn closing_a_session_keeps_the_question_of_a_running_connection_test() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("ha", "fa"); // the test's question, same host as session a
    r.shown();
    r.app.state_mut().conn.test = TestState::Running { request_id: 7 };
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(w.try_recv().is_err(), "the test may be the asker");
    assert_eq!(r.shown().map(|s| s.0), Some("ha".into()));
}

#[test]
fn closing_a_session_keeps_the_question_of_an_opening_compare_session() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("ha", "fa"); // the compare tunnel's question, same bastion as session a
    r.shown();
    r.app.state_mut().compare.session = Some(SessionPhase::Opening(1));
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(
        w.try_recv().is_err(),
        "the compare session may be the asker"
    );
    assert_eq!(r.shown().map(|s| s.0), Some("ha".into()));
}

#[test]
fn closing_a_session_rejects_its_question_once_compare_is_not_opening() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("ha", "fa");
    r.shown();
    r.app.state_mut().compare.session = Some(SessionPhase::Ready(1));
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert_eq!(w.try_recv(), Ok(HostKeyDecision::Reject));
}

#[test]
fn closing_a_session_rejects_its_question_once_no_test_is_running() {
    let mut r = rig(vec![via("a", "ha"), via("b", "hb")]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("ha", "fa");
    r.shown();
    r.app.state_mut().conn.test = TestState::Running { request_id: 7 };
    r.app.state_mut().conn.test = TestState::Idle;
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert_eq!(w.try_recv(), Ok(HostKeyDecision::Reject));
}

#[test]
fn closing_a_session_without_ssh_rejects_nothing() {
    let mut r = rig(vec![
        ConnectionConfig::new("plain", "mongodb://db"),
        via("b", "hb"),
    ]);
    r.connect(0);
    r.connect(1);
    let w = r.ask("hb", "fb");
    r.shown();
    r.app.dispatch(Input::Disconnect(r.ids[0]));
    assert!(w.try_recv().is_err());
    assert_eq!(r.shown(), named("hb", "fb", "b"));
}
