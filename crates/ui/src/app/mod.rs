//! Application shell: wires the runtime, the reducer and the egui panels together.
//!
//! * `state` / `reducer` / `actions`: pure state transitions (all decisions)
//! * `connections` / `persistence` / `host_key`: side effects behind injectable seams
//! * `panels`: the thin egui layer
mod actions;
mod banner;
pub mod connections;
pub mod host_key;
mod panels;
pub mod persistence;
pub mod reducer;
pub mod state;

use crate::I18n;
use crate::views::connections::ConnectionAction;
use connections::{ConfigPath, load_startup, refresh_flags};
use eframe::egui;
use host_key::{HOST_KEY_PROMPT_TIMEOUT, HostKeyRequest, InteractivePrompt, PromptState};
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::CoreError;
use mongostudio_core::runtime::{
    Command, Repaint, RuntimeConfig, RuntimeHandle, default_known_hosts_path,
    spawn_runtime_with_config,
};
use mongostudio_core::secrets::SecretStore;
use mongostudio_core::tunnel::HostKeyDecision;
use panels::UiEvent;
use persistence::default_connections_path;
use reducer::{Input, update};
use state::{AppState, Banner, ConnStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Slack on top of the prompt timeout so a user who answers just in time still connects
/// (the runtime's command timeout bounds the whole connect, prompt included).
const CONNECT_SLACK: Duration = Duration::from_secs(30);

/// Runtime settings for the app: interactive host-key prompt and a command timeout that
/// outlasts it.
pub fn runtime_config(known_hosts_path: PathBuf, prompt: Arc<InteractivePrompt>) -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: HOST_KEY_PROMPT_TIMEOUT + CONNECT_SLACK,
        known_hosts_path,
        host_key_prompt: prompt,
        ..RuntimeConfig::default()
    }
}

pub struct MongoStudioApp {
    state: AppState,
    handle: RuntimeHandle,
    i18n: I18n,
    secrets: Arc<dyn SecretStore>,
    config_path: ConfigPath,
    known_hosts: PathBuf,
    prompt: PromptState,
    prompt_requests: Option<Receiver<HostKeyRequest>>,
}

impl MongoStudioApp {
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
        }
    }

    /// Builds everything the binary needs. Must be called inside a Tokio runtime context
    /// (the runtime spawns its dispatcher on `Handle::current()`), and from the eframe creator
    /// closure so `ctx` is available for repaint requests.
    pub fn launch(secrets: Arc<dyn SecretStore>, ctx: egui::Context) -> Self {
        let i18n = I18n::english();
        let config_path = default_connections_path();
        let (connections, startup_banner) = load_startup(&config_path);
        let repaint: Repaint = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        let known_hosts = default_known_hosts_path();
        let (prompt, requests) =
            InteractivePrompt::channel(repaint.clone(), HOST_KEY_PROMPT_TIMEOUT);
        let config = runtime_config(known_hosts.clone(), prompt);
        let handle = spawn_runtime_with_config(Some(secrets.clone()), None, repaint, config);
        Self::new(handle, i18n, connections, secrets)
            .with_config_path(config_path)
            .with_known_hosts_path(known_hosts)
            .with_host_key_requests(requests)
            .with_banner(startup_banner)
    }

    pub fn with_config_path(self, config_path: ConfigPath) -> Self {
        Self {
            config_path,
            ..self
        }
    }

    pub fn with_known_hosts_path(self, known_hosts: PathBuf) -> Self {
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

    pub fn with_banner(mut self, banner: Option<Banner>) -> Self {
        self.state.banner = banner.or(self.state.banner);
        self
    }

    pub fn state(&self) -> &AppState {
        &self.state
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
        self.send(commands);
    }

    fn send(&mut self, commands: Vec<Command>) {
        for command in commands {
            if matches!(command, Command::Connect(_) | Command::Disconnect) {
                // A pending host-key question belongs to the connection being replaced.
                self.prompt = std::mem::take(&mut self.prompt).rejected_all();
            }
            if self.handle.tx.send(command).is_err() {
                self.runtime_stopped();
            }
        }
    }

    fn runtime_stopped(&mut self) {
        let error = CoreError::Database("runtime stopped".into());
        let state = std::mem::take(&mut self.state);
        self.state = AppState {
            status: ConnStatus::Disconnected,
            banner: Some(Banner::Failure {
                context: "runtime",
                error,
            }),
            ..state.session_cleared()
        };
    }

    fn handle_connection_action(&mut self, action: ConnectionAction) {
        let state = std::mem::take(&mut self.state);
        let secrets = self.secrets.as_ref();
        match action {
            ConnectionAction::Connect(id) => {
                self.state = state;
                self.dispatch(Input::Connect(id));
            }
            ConnectionAction::Save(form) => {
                self.state = connections::save(state, &form, secrets, &self.config_path);
            }
            ConnectionAction::Delete(id) => {
                self.state = connections::delete(state, id, secrets, &self.config_path);
            }
        }
    }

    fn confirm_forget(&mut self) {
        let result = match &self.state.forget_confirm {
            Some(endpoint) => {
                host_key::forget_host(&self.known_hosts, &endpoint.host, endpoint.port)
            }
            None => return,
        };
        if let Err(e) = &result {
            eprintln!("could not update known_hosts: {e:?}");
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

    /// Moves host-key requests and runtime events into the state.
    fn pump(&mut self) {
        let requests: Vec<HostKeyRequest> = self
            .prompt_requests
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for request in requests {
            self.prompt = std::mem::take(&mut self.prompt).offered(request);
        }
        while let Ok(event) = self.handle.rx.try_recv() {
            self.dispatch(Input::Event(event));
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
    }
}

impl eframe::App for MongoStudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.prompt = std::mem::take(&mut self.prompt).rejected_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_timeout_outlasts_the_host_key_prompt() {
        let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), HOST_KEY_PROMPT_TIMEOUT);
        let config = runtime_config(PathBuf::from("kh"), prompt);
        assert!(config.command_timeout > HOST_KEY_PROMPT_TIMEOUT);
        assert_eq!(config.known_hosts_path, PathBuf::from("kh"));
    }
}
