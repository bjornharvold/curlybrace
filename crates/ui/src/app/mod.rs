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
pub mod copy;
mod copy_actions;
mod copy_documents;
pub mod copy_state;
pub mod copy_structure_report;
mod dialog_banner;
pub mod file_picker;
pub mod host_key;
mod index_actions;
mod index_edit_actions;
mod index_write_actions;
mod panels;
pub mod persistence;
pub mod reducer;
pub mod sampling_settings;
pub mod session;
mod session_actions;
pub mod state;
pub mod write_guard;

use crate::I18n;
use crate::error_text::failure_log_line;
use crate::viewmodel::compare::{ExportUi, SessionPhase};
use crate::views::connections::{ConnectionAction, TestState};
use compare::CompareAction;
use connection_picker::ConnPickKind;
use connections::{ConfigPath, load_startup, refresh_flags};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{
    COMMAND_TIMEOUT, Command, Event, Repaint, RuntimeConfig, RuntimeHandle, TryRecvError,
    default_history_path, default_known_hosts_path, spawn_runtime_with_config,
};
use curlybrace_core::secrets::SecretStore;
use eframe::egui;
use file_picker::{FilePicker, NoFilePicker, PendingPick, PickPoll, start_pick, suggested_file};
use host_key::{
    HOST_KEY_PROMPT_TIMEOUT, HostKeyAnswer, HostKeyRequest, InteractivePrompt, PromptState,
};
use panels::UiEvent;
use persistence::default_connections_path;
use reducer::{Input, update};
use sampling_settings::{SaveError, default_sampling_path};
use state::{AppState, Banner, ConnStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Slack on top of the prompt timeout so a user who answers just in time still connects
/// (the runtime's connect timeout bounds the whole connect, prompt included).
const CONNECT_SLACK: Duration = Duration::from_secs(30);

/// Runtime settings for the app: interactive host-key prompt, a connect timeout that outlasts
/// it, and the normal short bound for data commands. `history_path` is where index usage is
/// recorded (`None` turns history off): the shipped app passes `default_history_path()`, anything
/// else that spawns a runtime passes a throwaway location so it never writes to the user's real
/// config directory.
pub fn runtime_config(
    known_hosts_path: Option<PathBuf>,
    history_path: Option<PathBuf>,
    prompt: Arc<InteractivePrompt>,
) -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: COMMAND_TIMEOUT,
        connect_timeout: HOST_KEY_PROMPT_TIMEOUT + CONNECT_SLACK,
        known_hosts_path,
        host_key_prompt: prompt,
        history_path,
        ..RuntimeConfig::default()
    }
}

/// The history purges that follow a delete: none unless the delete was saved to disk.
fn purges_after(saved: bool, before: &[uuid::Uuid], state: &AppState) -> Vec<Command> {
    match saved {
        true => connection_ops::purge_removed(before, state),
        false => Vec::new(),
    }
}

pub struct CurlybraceApp {
    state: AppState,
    handle: RuntimeHandle,
    i18n: I18n,
    secrets: Arc<dyn SecretStore>,
    config_path: ConfigPath,
    /// Where the sampling settings are saved. `new` has no place, so a test never writes the
    /// real config directory; `launch` and `with_sampling_path` set it.
    sampling_path: ConfigPath,
    /// Whether `sampling.json` may be replaced (not when a bad one could not be backed up).
    sampling_guard: persistence::PersistGuard,
    known_hosts: Option<PathBuf>,
    prompt: PromptState,
    prompt_requests: Option<Receiver<HostKeyRequest>>,
    picker: Arc<dyn FilePicker>,
    repaint: Repaint,
    /// The save dialog being shown, if any: at most one at a time.
    pick: Option<PendingPick>,
    /// The import/export dialog being shown, if any: at most one at a time.
    conn_pick: Option<connection_picker::ConnPick>,
    /// The runtime was found gone (never reset: it is not restarted); keeps its banner to one.
    runtime_gone: bool,
    /// Unix seconds for the views; the wall clock, replaced by tests that need a fixed frame.
    clock: fn() -> i64,
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
            sampling_path: Err(persistence::PersistError::ConfigDirUnknown),
            sampling_guard: persistence::PersistGuard::default(),
            known_hosts: default_known_hosts_path(),
            prompt: PromptState::default(),
            prompt_requests: None,
            picker: Arc::new(NoFilePicker),
            repaint: Arc::new(|| {}),
            pick: None,
            conn_pick: None,
            runtime_gone: false,
            clock: curlybrace_core::clock::now_secs,
        }
    }

    /// Replaces the clock the views are drawn with (tests that need a fixed frame).
    pub fn with_clock(self, clock: fn() -> i64) -> Self {
        Self { clock, ..self }
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
        let config = runtime_config(known_hosts.clone(), default_history_path(), prompt);
        let handle =
            spawn_runtime_with_config(Some(secrets.clone()), None, repaint.clone(), config);
        Self::new(handle, i18n, connections, secrets)
            .with_config_path(config_path)
            .with_known_hosts_path(known_hosts)
            .with_host_key_requests(requests)
            .with_file_picker(Arc::new(file_picker::NativeFilePicker), repaint)
            .with_persist_guard(guard)
            .with_banner(startup_banner)
            .with_sampling_path(default_sampling_path())
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

    /// Loads the sampling settings from `path` and hands them to the runtime, once, before
    /// anything else is sent. A file that cannot be used is backed up and the defaults apply,
    /// with the recovery banner (the connection list's own banner, if any, keeps its place).
    pub fn with_sampling_path(mut self, path: ConfigPath) -> Self {
        let started = match &path {
            Ok(p) => sampling_settings::load(p, &persistence::back_up),
            Err(_) => sampling_settings::SamplingStartup {
                settings: Default::default(),
                guard: Default::default(),
                recovery: None,
            },
        };
        self.state.sampling = started.settings;
        // One banner slot: the connection list's startup banner keeps it and the sampling notice
        // waits behind it, shown when that one is dismissed.
        if let Some(recovery) = started.recovery {
            self.state = std::mem::take(&mut self.state).with_recovery_notice(recovery);
        }
        self.sampling_path = path;
        self.sampling_guard = started.guard;
        let command = self.state.sampling.command();
        self.send(vec![command]);
        self
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

    /// `(host, fingerprint, connection name)` of the SSH host-key question being shown, if any.
    /// The name is the one connecting session going through that host; `None` when no or
    /// several sessions do, since the prompt cannot say who is asking then.
    pub fn pending_host_key(&self) -> Option<(&str, &str, Option<String>)> {
        self.prompt.current().map(|(host, fingerprint)| {
            (
                host,
                fingerprint,
                self.state.connecting_name_for_host(host, &self.i18n),
            )
        })
    }

    /// Answers the host-key question `answer` was made for (exactly once per question); an
    /// answer for a question that is no longer in front is dropped.
    pub fn answer_host_key(&mut self, answer: HostKeyAnswer) {
        let HostKeyAnswer {
            host,
            fingerprint,
            decision,
        } = answer;
        self.prompt = std::mem::take(&mut self.prompt).answered_for(&host, &fingerprint, decision);
    }

    /// Runs one input through the reducer and sends the resulting commands.
    pub fn dispatch(&mut self, input: Input) {
        // The closing session's host is read before the reducer forgets the session.
        let closing_host = match &input {
            Input::Disconnect(id) => self.state.ssh_host_of(*id),
            _ => None,
        };
        let sampling_before = self.state.sampling.clone();
        let (state, commands) = update(std::mem::take(&mut self.state), input);
        self.state = state;
        if self.state.sampling != sampling_before {
            self.save_sampling();
        }
        self.state = std::mem::take(&mut self.state).with_queued_recovery_shown();
        self.reject_closed_hosts_question(closing_host);
        // A dialog only belongs to an export that is waiting for it: when the person cancelled
        // (or the export is gone) the old dialog is forgotten, its late answer is never read, and
        // choosing again opens a fresh one.
        if !matches!(self.state.compare.export, ExportUi::Choosing(_)) {
            self.pick = None;
        }
        self.send(commands);
    }

    /// Writes the sampling settings after a change. A failed write is a banner like a failed save
    /// of the connection list; the change itself stays applied for this run.
    fn save_sampling(&mut self) {
        let result = match &self.sampling_path {
            Ok(path) => sampling_settings::saved(path, &self.sampling_guard, &self.state.sampling),
            Err(e) => Err(SaveError::Persist(e.clone())),
        };
        if let Err(e) = result {
            log::warn!("could not save the sampling settings: {e:?}");
            self.state.banner = Some(e.banner());
        }
    }

    /// A closed session's queued host-key question has nobody left to answer for, so it is
    /// rejected. It stays only when another session still connecting through the same host, or
    /// a running test or opening compare session known to use that host, may be the asker.
    fn reject_closed_hosts_question(&mut self, closing_host: Option<String>) {
        let Some(host) = closing_host else { return };
        if self.state.any_connecting_via(&host) || self.test_or_compare_uses(&host) {
            return;
        }
        self.drain_requests();
        self.prompt = std::mem::take(&mut self.prompt).rejected_for_host(&host);
    }

    /// Whether a connection test that is running, or a compare session that is opening its
    /// tunnels, is known to connect through SSH host `host`: both ask through the same prompt as
    /// the browsing sessions. The test's host is the one of the form it was started for (only when SSH is enabled on it), the
    /// compare's that of its two saved connections. Residual: a running test or compare whose
    /// host cannot be read counts as using none, so a closing session's question is rejected
    /// (fail closed: if that test or compare was the asker it fails and is run again).
    fn test_or_compare_uses(&self, host: &str) -> bool {
        let same = |h: &str| h.trim().eq_ignore_ascii_case(host);
        let testing = matches!(self.state.conn.test, TestState::Running { .. })
            && self
                .state
                .conn
                .test_form
                .as_ref()
                .is_some_and(|form| form.ssh_enabled && same(&form.ssh_host));
        let comparing = match (&self.state.compare.session, self.state.compare.connections) {
            (Some(SessionPhase::Opening(_)), Some((source, target))) => [source, target]
                .into_iter()
                .filter_map(|id| self.state.ssh_host_of_saved(id))
                .any(|h| same(&h)),
            _ => false,
        };
        testing || comparing
    }

    /// Whether something that belongs to no open session may be the asker of a queued host-key
    /// question. A connection test asks through the same prompt, and so does a compare session
    /// while it opens its SSH tunnels. When the last session closes while one runs, nothing is
    /// rejected wholesale; the closed session's own host was already handled by
    /// `reject_closed_hosts_question`, which spares only a host those are known to use.
    fn other_prompt_users_pending(&self) -> bool {
        matches!(self.state.conn.test, TestState::Running { .. })
            || matches!(self.state.compare.session, Some(SessionPhase::Opening(_)))
    }

    fn send(&mut self, commands: Vec<Command>) {
        for command in commands {
            // Prompts belong to sessions that coexist: a closing session's own question was already
            // rejected at dispatch time and the others' are left alone. Only when the last session
            // has closed does nobody remain to answer for: collect what the runtime queued, then
            // reject it all, unless a test or compare session may be the asker.
            if matches!(command, Command::Disconnect { .. })
                && self.state.open.is_empty()
                && !self.other_prompt_users_pending()
            {
                self.drain_requests();
                self.prompt = std::mem::take(&mut self.prompt).rejected_all();
            }
            if self.handle.tx.send(command).is_err() {
                self.runtime_stopped();
            }
        }
    }

    /// The runtime is gone: the browsing session and the compare session (whose connections
    /// lived in it) are both over, and the failure banner says so. A command sent after the loss
    /// raises it again: starting a connect withdraws a banner nobody owns, and the failed send
    /// must not leave the person without an explanation. `runtime_gone` only keeps the per-frame
    /// `pump` from repeating the notice once a failed send has already given it.
    fn runtime_stopped(&mut self) {
        self.runtime_gone = true;
        let cleared = std::mem::take(&mut self.state).session_cleared();
        self.state = AppState {
            compare: cleared.compare.clone().session_dropped(),
            // A test that was waiting for its answer will never get one.
            conn: connection_test::abandoned(cleared.conn),
            status: ConnStatus::Disconnected,
            banner: Some(Banner::Failure {
                context: "runtime",
                error: CoreError::Database("runtime stopped".into()),
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
                let was_read_only = state.connection(form.id).is_some_and(|c| c.read_only);
                self.state =
                    connection_save::save_and_close(state, &form, secrets, &self.config_path, now);
                // Open sessions keep the flag they connected with, so a newly ticked one is also
                // sent to the core, which then refuses their writes at once (until a reconnect,
                // so their snapshots are raised to match).
                let locks = connection_save::lock_commands(&self.state, form.id, was_read_only);
                if !locks.is_empty() {
                    let state = std::mem::take(&mut self.state);
                    self.state = connection_save::locked_snapshots(state, form.id);
                }
                self.send(locks);
            }
            ConnectionAction::Delete(id) => {
                let was_open = state.is_open(id);
                let before = connection_ops::saved_ids(&state);
                let (next, saved) =
                    connection_ops::delete_and_close_saved(state, id, secrets, &self.config_path);
                self.state = next;
                // A session must not outlive the config and credentials it was opened from;
                // a failed delete leaves both in place, so it closes nothing.
                if was_open && self.state.connection(id).is_none() {
                    self.dispatch(Input::Disconnect(id));
                }
                // After the disconnect, so the session's sampler is stopped before its history
                // goes. A purge cannot be undone, so it follows only a delete that reached the
                // disk: if the write failed the connection returns at the next launch.
                self.send(purges_after(saved, &before, &self.state));
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
                let before = connection_ops::saved_ids(&state);
                let (next, saved) =
                    connection_ops::delete_folder_saved(state, name, now, &self.config_path);
                self.state = next;
                // Today a folder delete only moves its connections to the top level, so this
                // purges nothing; it keeps the history of any connection a delete removes,
                // under the same rule: only once the list was written.
                self.send(purges_after(saved, &before, &self.state));
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
            UiEvent::HostKey(answer) => self.answer_host_key(answer),
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
        loop {
            let event = match self.handle.rx.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Repeats every frame while closed; only the first notice acts.
                    if !self.runtime_gone {
                        self.runtime_stopped();
                    }
                    break;
                }
            };
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
                // Whatever the persist guard says, the connect the person asked for (the active
                // session) ends any open edit dialog so typed secrets do not linger and it does
                // not reappear after Disconnect; a background session connecting does not.
                let state = std::mem::take(&mut self.state);
                self.state = if state.active == Some(id) {
                    connection_ops::dialog_closed(state)
                } else {
                    state
                };
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
        crate::views::connections_from_uri::wipe_if_pending(ctx, &mut self.state.conn);
        panels::show_banner(ctx, &self.state, &self.i18n, &mut out);
        if !self.state.open.is_empty() {
            panels::show_sessions_panel(ctx, &self.state, &self.i18n, &mut out);
        }
        if panels::shows_manager(&self.state) {
            panels::show_connections_screen(ctx, &mut self.state, &self.i18n, &mut out);
        } else {
            // The one place a frame reads the wall clock for the views.
            let now_secs = (self.clock)();
            panels::show_workspace(ctx, &mut self.state, &self.i18n, &mut out, now_secs);
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
mod sampling_settings_tests;

#[cfg(test)]
mod index_tests;

#[cfg(test)]
mod index_edit_tests;

#[cfg(test)]
mod index_write_tests;

#[cfg(test)]
mod connection_import_tests;

#[cfg(test)]
mod connection_stamp_tests;

#[cfg(test)]
mod persistence_load_tests;

#[cfg(test)]
mod runtime_closed_tests;

#[cfg(test)]
mod session_labels_tests;

#[cfg(test)]
mod session_wiring_tests;

#[cfg(test)]
mod write_warning_modal_tests;

#[cfg(test)]
mod production_snapshot_tests;

#[cfg(test)]
mod read_only_snapshot_tests;

#[cfg(test)]
mod connection_lock_tests;

#[cfg(test)]
mod copy_documents_tests;

#[cfg(test)]
mod copy_menu_tests;

#[cfg(test)]
mod copy_run_tests;

#[cfg(test)]
mod copy_structure_report_tests;

#[cfg(test)]
mod copy_tests;

#[cfg(test)]
mod tests;
