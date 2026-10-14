//! Connection manager view: saved connections list plus an editable form.
use crate::I18n;
use crate::viewmodel::ConnectionForm;
use crate::viewmodel::StoredSecrets;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::TestReport;
use curlybrace_core::secrets::SecretString;
use curlybrace_core::uri::parse_uri;
use eframe::egui;
use std::collections::HashMap;
use uuid::Uuid;

/// What the user asked for; the caller performs it.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionAction {
    Connect(Uuid),
    Save(Box<ConnectionForm>),
    Delete(Uuid),
    Duplicate(Uuid),
    MoveToFolder {
        id: Uuid,
        folder: String,
    },
    RenameFolder {
        from: String,
        to: String,
    },
    DeleteFolder(String),
    StartImport,
    StartExport,
    Test(Box<ConnectionForm>),
    BuildUri {
        form: Box<ConnectionForm>,
        include_password: bool,
    },
}

/// Progress of the Test Connection button. `tls` and `authenticated` in a finished report mean
/// "configured", not "verified".
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TestState {
    #[default]
    Idle,
    Running {
        request_id: u64,
    },
    Done {
        request_id: u64,
        outcome: Result<TestReport, CoreError>,
    },
}

/// The outcome of an import or export, shown by the view (never carries paths).
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Imported { imported: usize, skipped: usize },
    Exported { count: usize },
}

/// View state: saved connections, the form being edited and the last validation error.
#[derive(Debug, Default)]
pub struct ConnectionsState {
    pub connections: Vec<ConnectionConfig>,
    pub selected: Option<Uuid>,
    pub form: ConnectionForm,
    pub error: Option<CoreError>,
    /// Which secrets exist per connection; filled by the app via `stored_secret_flags`.
    pub stored: HashMap<Uuid, StoredSecrets>,
    /// Connection whose delete awaits confirmation.
    pub delete_confirm: Option<Uuid>,
    /// Result of the last import or export.
    pub notice: Option<Notice>,
    /// Test Connection progress and the form it was started for.
    pub test: TestState,
    pub test_seq: u64,
    pub test_form: Option<ConnectionForm>,
    /// Text pasted for "From URI"; cleared as soon as it has been parsed (it may hold a password).
    pub uri_input: String,
    /// "To URI" result (may hold a real password; `Debug` redacts it), the form it was built
    /// for, and whether it contains the real password.
    pub built_uri: Option<SecretString>,
    pub built_for: Option<ConnectionForm>,
    pub built_with_password: bool,
}

impl ConnectionsState {
    pub fn new(connections: Vec<ConnectionConfig>) -> Self {
        Self {
            connections,
            ..Self::default()
        }
    }

    /// First step of deleting: ask for confirmation (nothing is deleted yet).
    pub fn delete_requested(&mut self, id: Uuid) {
        self.delete_confirm = Some(id);
    }

    /// Second step: emits `Delete` once for the confirmed connection.
    pub fn delete_confirmed(&mut self) -> Option<ConnectionAction> {
        self.delete_confirm.take().map(ConnectionAction::Delete)
    }

    pub fn delete_cancelled(&mut self) {
        self.delete_confirm = None;
    }

    /// Drops a test result and a built URI once the form no longer matches what they belong to.
    pub fn invalidate_stale_test(&mut self) {
        if self.built_for.as_ref() != Some(&self.form) {
            self.clear_built_uri();
        }
        if self.test_form.as_ref() != Some(&self.form)
            && matches!(self.test, TestState::Done { .. })
        {
            self.test = TestState::Idle;
        }
    }

    fn clear_built_uri(&mut self) {
        self.built_uri = None;
        self.built_for = None;
        self.built_with_password = false;
    }

    /// Forgets the test result, the pasted text and the built URI (selection change, close).
    pub fn reset_tools(&mut self) {
        self.test = TestState::Idle;
        self.test_form = None;
        self.uri_input.clear();
        self.clear_built_uri();
    }

    /// Parses the pasted connection string into the form. The buffer is cleared either way and
    /// a failure never echoes it.
    pub fn apply_uri_input(&mut self) {
        let input = std::mem::take(&mut self.uri_input);
        match parse_uri(&input) {
            Ok(parsed) => {
                self.form = self.form.clone().apply_parsed(parsed);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn select(&mut self, cfg: &ConnectionConfig) {
        self.reset_tools();
        self.delete_confirm = None;
        self.notice = None;
        self.selected = Some(cfg.id);
        self.form = ConnectionForm::from_config(cfg);
        self.error = None;
    }

    fn start_new(&mut self) {
        self.reset_tools();
        self.delete_confirm = None;
        self.notice = None;
        self.selected = None;
        self.form = ConnectionForm::default();
        self.error = None;
    }
}

pub fn show_connections(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    ui.heading(i18n.t("connections.title"));
    let mut action = show_list(ui, state, i18n);
    ui.separator();
    if let Some(saved) = super::connections_edit::show_form(ui, state, i18n) {
        action = Some(saved);
    }
    show_delete_confirm(ui.ctx(), state, i18n).or(action)
}

/// Modal asking to confirm a delete; Escape or clicking outside cancels.
fn show_delete_confirm(
    ctx: &egui::Context,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let id = state.delete_confirm?;
    let name = state
        .connections
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let (mut confirmed, mut cancelled) = (false, false);
    let modal = egui::Modal::new(egui::Id::new("connection_delete_confirm")).show(ctx, |ui| {
        ui.label(i18n.t_with("connections.confirm_delete", &[("name", &name)]));
        ui.horizontal(|ui| {
            cancelled = ui.button(i18n.t("common.cancel")).clicked();
            confirmed = ui.button(i18n.t("common.delete")).clicked();
        });
    });
    if confirmed {
        return state.delete_confirmed();
    }
    if cancelled || modal.should_close() {
        state.delete_cancelled();
    }
    None
}

fn show_list(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let mut action = None;
    if ui.button(i18n.t("connections.new")).clicked() {
        state.start_new();
    }
    let rows = state.connections.clone();
    for cfg in &rows {
        ui.horizontal(|ui| {
            let selected = state.selected == Some(cfg.id);
            if ui.selectable_label(selected, &cfg.name).clicked() {
                state.select(cfg);
            }
            if ui.button(i18n.t("connections.connect")).clicked() {
                action = Some(ConnectionAction::Connect(cfg.id));
            }
            if ui.button(i18n.t("common.delete")).clicked() {
                state.delete_requested(cfg.id);
            }
        });
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::test_support::mock_objects::mock_connection;

    fn run_frame(state: &mut ConnectionsState) -> Option<ConnectionAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show_connections(ui, state, &i18n);
            });
        });
        out
    }

    #[test]
    fn renders_empty_and_populated_state_without_panicking() {
        assert_eq!(run_frame(&mut ConnectionsState::default()), None);
        let cfg = mock_connection();
        let mut state = ConnectionsState::new(vec![cfg.clone()]);
        state.select(&cfg);
        state.form.ssh_enabled = true;
        state.form.tls_enabled = true;
        state.error = Some(CoreError::InvalidUri("x".into()));
        state.stored.insert(
            cfg.id,
            StoredSecrets {
                password: true,
                ssh_secret: true,
            },
        );
        assert_eq!(run_frame(&mut state), None);
    }

    #[test]
    fn select_loads_form_without_secrets_and_new_resets() {
        let cfg = mock_connection();
        let mut state = ConnectionsState::new(vec![cfg.clone()]);
        state.form.password = "leftover".into();
        state.select(&cfg);
        assert_eq!(state.form.id, cfg.id);
        assert!(state.form.password.is_empty());
        assert!(!state.form.clear_password);
        state.start_new();
        assert_ne!(state.form.id, cfg.id);
        assert!(state.selected.is_none());
    }

    #[test]
    fn delete_is_only_emitted_after_the_confirmation_and_only_once() {
        let cfg = mock_connection();
        let mut state = ConnectionsState::new(vec![cfg.clone()]);
        assert_eq!(state.delete_confirmed(), None, "nothing asked yet");
        state.delete_requested(cfg.id);
        assert_eq!(state.delete_confirm, Some(cfg.id));
        assert_eq!(
            state.delete_confirmed(),
            Some(ConnectionAction::Delete(cfg.id))
        );
        assert_eq!(state.delete_confirmed(), None, "emitted exactly once");
    }

    #[test]
    fn cancelling_or_selecting_another_connection_clears_the_confirmation() {
        let (a, b) = (mock_connection(), mock_connection());
        let mut state = ConnectionsState::new(vec![a.clone(), b.clone()]);
        state.delete_requested(a.id);
        state.delete_cancelled();
        assert_eq!(state.delete_confirm, None);
        state.delete_requested(a.id);
        state.select(&b);
        assert_eq!(state.delete_confirm, None);
        state.delete_requested(a.id);
        state.start_new();
        assert_eq!(state.delete_confirm, None);
    }

    #[test]
    fn the_confirmation_modal_renders_and_names_the_connection() {
        let cfg = mock_connection();
        let mut state = ConnectionsState::new(vec![cfg.clone()]);
        state.delete_requested(cfg.id);
        assert_eq!(run_frame(&mut state), None);
        assert_eq!(state.delete_confirm, Some(cfg.id), "still waiting");
    }
}
