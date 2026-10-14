//! Application shell: wires the runtime, the reducer and the egui panels together.
//!
//! * `state` / `reducer` / `actions`: pure state transitions (all decisions)
//! * `connections` / `persistence` / `host_key`: side effects behind injectable seams
//! * `panels`: the thin egui layer
mod actions;
mod banner;
pub mod compare;
mod compare_apply;
mod compare_checks;
mod compare_duplicates;
mod compare_export;
mod compare_fields;
mod compare_pairs;
mod compare_results;
mod compare_run;
mod compare_select;
mod connection_io;
mod connection_ops;
mod connection_picker;
mod connection_save;
mod connection_test;
mod connection_uri;
pub mod connections;
pub mod file_picker;
pub mod host_key;
mod panels;
pub mod persistence;
pub mod reducer;
mod session;
mod session_actions;
pub mod state;

use crate::I18n;
use crate::error_text::failure_log_line;
use crate::viewmodel::compare::ExportUi;
use crate::views::connections::{ConnectionAction, ConnectionsState, TestState};
use compare::CompareAction;
use connection_picker::ConnPickKind;
use connections::{ConfigPath, load_startup, refresh_flags};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{
    COMMAND_TIMEOUT, Command, Event, Repaint, RuntimeConfig, RuntimeHandle,
    default_known_hosts_path, spawn_runtime_with_config,
};
use curlybrace_core::secrets::SecretStore;
use curlybrace_core::tunnel::HostKeyDecision;
use eframe::egui;
use file_picker::{FilePicker, NoFilePicker, PendingPick, PickPoll, start_pick, suggested_file};
use host_key::{HOST_KEY_PROMPT_TIMEOUT, HostKeyRequest, InteractivePrompt, PromptState};
use panels::UiEvent;
use persistence::default_connections_path;
use reducer::{Input, update};
use state::{AppState, Banner, ConnStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Slack on top of the prompt timeout so a user who answers just in time still connects
/// (the runtime's connect timeout bounds the whole connect, prompt included).
const CONNECT_SLACK: Duration = Duration::from_secs(30);

/// Runtime settings for the app: interactive host-key prompt, a connect timeout that outlasts
/// it, and the normal short bound for data commands.
pub fn runtime_config(
    known_hosts_path: Option<PathBuf>,
    prompt: Arc<InteractivePrompt>,
) -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: COMMAND_TIMEOUT,
        connect_timeout: HOST_KEY_PROMPT_TIMEOUT + CONNECT_SLACK,
        known_hosts_path,
        host_key_prompt: prompt,
        ..RuntimeConfig::default()
    }
}

pub struct CurlybraceApp {
    state: AppState,
    handle: RuntimeHandle,
    i18n: I18n,
    secrets: Arc<dyn SecretStore>,
    config_path: ConfigPath,
    known_hosts: Option<PathBuf>,
    prompt: PromptState,
    prompt_requests: Option<Receiver<HostKeyRequest>>,
    picker: Arc<dyn FilePicker>,
    repaint: Repaint,
    /// The save dialog being shown, if any: at most one at a time.
    pick: Option<PendingPick>,
    /// The import/export dialog being shown, if any: at most one at a time.
    conn_pick: Option<connection_picker::ConnPick>,
}

impl CurlybraceApp {
    pub fn new(
        handle: RuntimeHandle,
        i18n: I18n,
        connections: Vec<ConnectionConfig>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        let state = refresh_flags(AppState::new(connections), secrets.as_ref());
        Self {
            state,
            handle,
            i18n,
            secrets,
            config_path: default_connections_path(),
            known_hosts: default_known_hosts_path(),
            prompt: PromptState::default(),
            prompt_requests: None,
            picker: Arc::new(NoFilePicker),
            repaint: Arc::new(|| {}),
            pick: None,
            conn_pick: None,
        }
    }

    /// Builds everything the binary needs. Must be called inside a Tokio runtime context
    /// (the runtime spawns its dispatcher on `Handle::current()`), and from the eframe creator
    /// closure so `ctx` is available for repaint requests.
    pub fn launch(secrets: Arc<dyn SecretStore>, ctx: egui::Context) -> Self {
        let i18n = I18n::english();
        let config_path = default_connections_path();
        let (connections, guard, startup_banner) = load_startup(&config_path);
        let repaint: Repaint = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let known_hosts = default_known_hosts_path();
        let (prompt, requests) =
            InteractivePrompt::channel(repaint.clone(), HOST_KEY_PROMPT_TIMEOUT);
        let config = runtime_config(known_hosts.clone(), prompt);
        let handle =
            spawn_runtime_with_config(Some(secrets.clone()), None, repaint.clone(), config);
        Self::new(handle, i18n, connections, secrets)
            .with_config_path(config_path)
            .with_known_hosts_path(known_hosts)
            .with_host_key_requests(requests)
            .with_file_picker(Arc::new(file_picker::NativeFilePicker), repaint)
            .with_persist_guard(guard)
            .with_banner(startup_banner)
    }

    pub fn with_config_path(self, config_path: ConfigPath) -> Self {
        Self {
            config_path,
            ..self
        }
    }

    pub fn with_known_hosts_path(self, known_hosts: Option<PathBuf>) -> Self {
        Self {
            known_hosts,
            ..self
        }
    }

    pub fn with_host_key_requests(self, requests: Receiver<HostKeyRequest>) -> Self {
        Self {
            prompt_requests: Some(requests),
            ..self
        }
    }

    /// The save-dialog implementation and the wake-up its worker thread calls when it answers.
    pub fn with_file_picker(self, picker: Arc<dyn FilePicker>, repaint: Repaint) -> Self {
        Self {
            picker,
            repaint,
            ..self
        }
    }

    pub fn with_persist_guard(mut self, guard: persistence::PersistGuard) -> Self {
        self.state.persist = guard;
        self
    }

    pub fn with_banner(mut self, banner: Option<Banner>) -> Self {
        self.state.banner = banner.or(self.state.banner);
        self
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Direct state access for embedding and tests (typing into the editor, for example).
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// `(host, fingerprint)` of the SSH host-key question being shown, if any.
    pub fn pending_host_key(&self) -> Option<(&str, &str)> {
        self.prompt.current()
    }

    /// Answers the host-key question being shown (exactly once per question).
    pub fn answer_host_key(&mut self, decision: HostKeyDecision) {
        self.prompt = std::mem::take(&mut self.prompt).answered(decision);
    }

    /// Runs one input through the reducer and sends the resulting commands.
    pub fn dispatch(&mut self, input: Input) {
        let (state, commands) = update(std::mem::take(&mut self.state), input);
        self.state = state;
        // A dialog only belongs to an export that is waiting for it: when the person cancelled
        // (or the export is gone) the old dialog is forgotten, its late answer is never read, and
        // choosing again opens a fresh one.
        if !matches!(self.state.compare.export, ExportUi::Choosing(_)) {
            self.pick = None;
        }
        self.send(commands);
    }

    fn send(&mut self, commands: Vec<Command>) {
        for command in commands {
            if matches!(command, Command::Connect(_) | Command::Disconnect { .. }) {
                // Questions already queued by the runtime belong to the connection being
                // replaced: collect them first so none survives, then reject them all.
                self.drain_requests();
                self.prompt = std::mem::take(&mut self.prompt).rejected_all();
            }
            if self.handle.tx.send(command).is_err() {
                self.runtime_stopped();
            }
        }
    }

    /// The runtime is gone: the browsing session and the compare session (whose connections
    /// lived in it) are both over.
    fn runtime_stopped(&mut self) {
        let error = CoreError::Database("runtime stopped".into());
        let cleared = std::mem::take(&mut self.state).session_cleared();
        // A test that was waiting for its answer will never get one.
        let test = match &cleared.conn.test {
            TestState::Running { .. } => TestState::Idle,
            other => other.clone(),
        };
        self.state = AppState {
            compare: cleared.compare.clone().session_dropped(),
            conn: ConnectionsState {
                test,
                ..cleared.conn
            },
            status: ConnStatus::Disconnected,
            banner: Some(Banner::Failure {
                context: "runtime",
                error,
            }),
            ..cleared
        };
    }

    /// Applies one connection-manager action exactly as the view would (the entry point for
    /// tests that drive the app without drawing it).
    #[doc(hidden)]
    pub fn dispatch_connection(&mut self, action: ConnectionAction) {
        self.handle_connection_action(action);
    }

    fn handle_connection_action(&mut self, action: ConnectionAction) {
        let state = std::mem::take(&mut self.state);
        let secrets = self.secrets.as_ref();
        let now = curlybrace_core::clock::now_secs();
        match action {
            ConnectionAction::Connect(id) => {
                // Defensive: the modal blocks the list, but never connect with the dialog open.
                self.state = connection_ops::dialog_closed(state);
                self.dispatch(Input::Connect(id));
            }
            ConnectionAction::Save(form) => {
                self.state =
                    connection_save::save_and_close(state, &form, secrets, &self.config_path, now);
            }
            ConnectionAction::Delete(id) => {
                self.state =
                    connection_ops::delete_and_close(state, id, secrets, &self.config_path);
            }
            ConnectionAction::Duplicate(id) => {
                let source = state.connection(id).map(|c| c.name.clone());
                let name = self.i18n.t_with(
                    "connections.copy_name",
                    &[("name", source.as_deref().unwrap_or_default())],
                );
                self.state =
                    connection_ops::duplicate(state, id, name, now, secrets, &self.config_path);
            }
            ConnectionAction::MoveToFolder { id, folder } => {
                self.state =
                    connection_ops::move_to_folder(state, id, folder, now, &self.config_path);
            }
            ConnectionAction::RenameFolder { from, to } => {
                self.state = connection_ops::rename_folder(state, from, to, now, &self.config_path);
            }
            ConnectionAction::DeleteFolder(name) => {
                self.state = connection_ops::delete_folder(state, name, now, &self.config_path);
            }
            ConnectionAction::Test(form) => {
                self.state = state;
                self.dispatch(Input::TestConnection(form));
            }
            ConnectionAction::BuildUri {
                form,
                include_password,
            } => {
                self.state = connection_uri::build(state, &form, include_password, secrets);
            }
            ConnectionAction::StartImport => {
                self.state = state;
                self.start_connection_pick(ConnPickKind::Import);
            }
            ConnectionAction::StartExport => {
                self.state = state;
                self.start_connection_pick(ConnPickKind::Export);
            }
        }
    }

    /// Opens the import or export dialog unless one is already open.
    fn start_connection_pick(&mut self, kind: ConnPickKind) {
        if self.conn_pick.is_none() {
            self.conn_pick = Some(connection_picker::start(
                self.picker.clone(),
                kind,
                self.repaint.clone(),
            ));
        }
    }

    /// Runs the import or export once the dialog has answered; a cancelled dialog does nothing.
    fn poll_connection_picker(&mut self) {
        let Some(PickPoll::Done(path)) = self.conn_pick.as_ref().map(|p| p.poll()) else {
            return;
        };
        let Some(pick) = self.conn_pick.take() else {
            return;
        };
        let Some(path) = path else {
            return;
        };
        let state = std::mem::take(&mut self.state);
        self.state = match pick.kind {
            ConnPickKind::Import => connection_io::import_into(
                state,
                &path,
                curlybrace_core::clock::now_secs(),
                &self.config_path,
            ),
            ConnPickKind::Export => connection_io::export_to(state, &path),
        };
    }

    fn confirm_forget(&mut self) {
        let result = match &self.state.forget_confirm {
            Some(endpoint) => match &self.known_hosts {
                Some(path) => host_key::forget_host(path, &endpoint.host, endpoint.port),
                None => Err(persistence::PersistError::ConfigDirUnknown),
            },
            None => return,
        };
        if let Err(e) = &result {
            log::warn!("could not update known_hosts: {e:?}");
        }
        self.dispatch(Input::ForgetFinished(result));
    }

    fn apply_ui_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Input(input) => self.dispatch(input),
            UiEvent::Connection(action) => self.handle_connection_action(action),
            UiEvent::HostKey(decision) => self.answer_host_key(decision),
            UiEvent::ForgetConfirmed => self.confirm_forget(),
        }
    }

    fn drain_requests(&mut self) {
        let requests: Vec<HostKeyRequest> = self
            .prompt_requests
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for request in requests {
            self.prompt = std::mem::take(&mut self.prompt).offered(request);
        }
    }

    /// Moves host-key requests and runtime events into the state.
    fn pump(&mut self) {
        self.drain_requests();
        self.prompt = std::mem::take(&mut self.prompt).pruned();
        while let Ok(event) = self.handle.rx.try_recv() {
            if let Some(line) = failure_log_line(&event) {
                log::warn!("{line}");
            }
            // The session a Connected answer belongs to may be a background one.
            let connected_session = match &event {
                Event::Connected { session, .. } => Some(*session),
                _ => None,
            };
            let was_connected = |state: &AppState| {
                connected_session
                    .is_some_and(|id| state.status_of(id) == Some(ConnStatus::Connected))
            };
            let before = was_connected(&self.state);
            self.dispatch(Input::Event(event));
            if let Some(id) = connected_session.filter(|_| !before && was_connected(&self.state)) {
                // Whatever the persist guard says, a connect ends any open edit dialog so typed
                // secrets do not linger and it does not reappear after Disconnect.
                let state = std::mem::take(&mut self.state);
                self.state = connection_ops::dialog_closed(state);
                let state = std::mem::take(&mut self.state);
                self.state = connection_ops::mark_connected(
                    state,
                    id,
                    curlybrace_core::clock::now_secs(),
                    &self.config_path,
                );
            }
        }
        self.poll_picker();
        self.poll_connection_picker();
    }

    /// Opens the save dialog on a worker thread whenever an export waits in `Choosing` and no
    /// dialog is open (so a view only emits `ChooseExportFile`). The answer comes back through
    /// `poll_picker`; the reducer leaves `Choosing` on it, so a dialog is never reopened by
    /// itself after an answer.
    fn ensure_picker(&mut self) {
        let ExportUi::Choosing(draft) = &self.state.compare.export else {
            return;
        };
        if self.pick.is_some() {
            return;
        }
        let (name, extension) = suggested_file(draft.kind, draft.plan.format);
        self.pick = Some(start_pick(
            self.picker.clone(),
            draft.clone(),
            name,
            extension,
            self.repaint.clone(),
        ));
    }

    /// Turns the dialog's answer, once it is there, into the matching compare action, but only
    /// when the export it was opened for is still the one waiting; a stale answer is dropped.
    fn poll_picker(&mut self) {
        let Some(PickPoll::Done(path)) = self.pick.as_ref().map(|p| p.poll()) else {
            return;
        };
        let draft = self.pick.take().map(PendingPick::into_draft);
        let still_waiting = matches!(
            (&self.state.compare.export, &draft),
            (ExportUi::Choosing(current), Some(asked)) if current == asked
        );
        if !still_waiting {
            return;
        }
        let action = match path {
            Some(path) => CompareAction::ExportPathChosen(path.into()),
            None => CompareAction::ExportPathCancelled,
        };
        self.dispatch(Input::Compare(action));
    }

    /// Copies the clipboard report, if one is waiting, exactly once: the state keeps no copy.
    /// The text is never logged.
    fn flush_clipboard(&mut self, ctx: &egui::Context) {
        if !matches!(
            self.state.compare.export,
            ExportUi::Copied { text: Some(_), .. }
        ) {
            return;
        }
        let (state, text) = std::mem::take(&mut self.state).take_clipboard_text();
        self.state = state;
        if let Some(text) = text {
            ctx.copy_text(text);
        }
    }

    fn draw(&mut self, ctx: &egui::Context) -> Vec<UiEvent> {
        let mut out = Vec::new();
        panels::show_banner(ctx, &self.state, &self.i18n, &mut out);
        if self.state.status == ConnStatus::Connected {
            panels::show_browser(ctx, &mut self.state, &self.i18n, &mut out);
        } else {
            panels::show_connections_screen(ctx, &mut self.state, &self.i18n, &mut out);
        }
        panels::show_modals(ctx, &mut self.state, &self.prompt, &self.i18n, &mut out);
        out
    }

    /// One frame: pump inputs, draw, apply what the user did.
    pub fn run_frame(&mut self, ctx: &egui::Context) {
        self.pump();
        for event in self.draw(ctx) {
            self.apply_ui_event(event);
        }
        self.ensure_picker();
        self.flush_clipboard(ctx);
    }
}

impl eframe::App for CurlybraceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.prompt = std::mem::take(&mut self.prompt).rejected_all();
    }
}

#[cfg(test)]
mod file_picker_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_commands_keep_the_short_timeout_and_connect_outlasts_the_host_key_prompt() {
        let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), HOST_KEY_PROMPT_TIMEOUT);
        let config = runtime_config(Some(PathBuf::from("/kh")), prompt);
        assert_eq!(config.command_timeout, COMMAND_TIMEOUT);
        assert!(config.connect_timeout > HOST_KEY_PROMPT_TIMEOUT);
        assert_eq!(config.known_hosts_path, Some(PathBuf::from("/kh")));
    }

    /// An app whose runtime is two channels the test holds: it sees the commands and feeds events.
    fn app_with_open_dialog(
        guard: persistence::PersistGuard,
    ) -> (
        CurlybraceApp,
        tokio::sync::mpsc::UnboundedSender<Event>,
        tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Command>,
        tempfile::TempDir,
        uuid::Uuid,
    ) {
        let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, rx) = tokio::sync::mpsc::unbounded_channel();
        let a = ConnectionConfig::new("a", "mongodb://a");
        let id = a.id;
        let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            CurlybraceApp::new(RuntimeHandle { tx, rx }, I18n::english(), vec![a], secrets)
                .with_config_path(Ok(dir.path().join("connections.json")))
                .with_persist_guard(guard);
        let conn = &mut app.state_mut().conn;
        conn.editing = true;
        conn.selected = Some(id);
        conn.form.password = "typed".into();
        conn.uri_input = "mongodb://u:p@h".into();
        conn.built_uri = Some(curlybrace_core::secrets::SecretString::new(
            "mongodb://u:p@h",
        ));
        (app, events, commands, dir, id)
    }

    fn assert_dialog_closed_and_secrets_gone(app: &CurlybraceApp) {
        let conn = &app.state().conn;
        assert!(!conn.editing, "the dialog must not survive");
        assert!(conn.form.password.is_empty() && conn.uri_input.is_empty());
        assert!(conn.built_uri.is_none());
    }

    #[test]
    fn a_completed_connect_closes_the_edit_dialog_and_drops_its_secrets() {
        for guard in [
            persistence::PersistGuard::default(),
            persistence::PersistGuard::blocked(),
        ] {
            let (mut app, events, _commands, _dir, id) = app_with_open_dialog(guard);
            app.dispatch(Input::Connect(id));
            let epoch = app.state().latest_requested_epoch;
            events
                .send(Event::Connected {
                    session: id,
                    epoch,
                    topology: None,
                })
                .unwrap();
            app.pump();
            assert_eq!(app.state().status, ConnStatus::Connected);
            assert_dialog_closed_and_secrets_gone(&app);
        }
    }

    #[test]
    fn a_connect_action_with_the_dialog_open_closes_it() {
        let (mut app, _events, _commands, _dir, id) =
            app_with_open_dialog(persistence::PersistGuard::default());
        app.handle_connection_action(ConnectionAction::Connect(id));
        assert_dialog_closed_and_secrets_gone(&app);
    }

    #[test]
    fn a_stopped_runtime_also_ends_the_compare_session() {
        use crate::app::compare::CompareAction;
        use crate::viewmodel::compare::{RunOverview, RunView, SessionPhase};
        let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
        drop(commands);
        let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = RuntimeHandle { tx, rx };
        let a = ConnectionConfig::new("a", "mongodb://a");
        let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
        let mut app = CurlybraceApp::new(handle, I18n::english(), vec![a], secrets);
        app.state_mut().compare.session = Some(SessionPhase::Ready(4));
        app.state_mut().compare.next_session = 5;
        app.state_mut().compare.overview = RunOverview::default().started(2, &[(0, 1)]).0;
        // Any command now finds the runtime gone.
        app.dispatch(Input::Compare(CompareAction::Cancel));
        let compare = &app.state().compare;
        assert_eq!(compare.session, None);
        assert_eq!(compare.run_view(), RunView::Idle);
        assert_eq!(compare.next_session, 5, "ids are never reused");
        assert!(matches!(
            app.state().banner,
            Some(Banner::Failure {
                context: "runtime",
                ..
            })
        ));
    }

    #[test]
    fn a_stopped_runtime_does_not_leave_a_test_running_forever() {
        let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
        drop(commands);
        let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
        let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
        let mut app = CurlybraceApp::new(
            RuntimeHandle { tx, rx },
            I18n::english(),
            Vec::new(),
            secrets,
        );
        let form = crate::viewmodel::ConnectionForm {
            name: "n".into(),
            uri: "mongodb://localhost:27017".into(),
            ..Default::default()
        };
        app.dispatch(Input::TestConnection(Box::new(form)));
        assert_eq!(app.state().conn.test, TestState::Idle);
        assert!(matches!(
            app.state().banner,
            Some(Banner::Failure {
                context: "runtime",
                ..
            })
        ));
    }
}
