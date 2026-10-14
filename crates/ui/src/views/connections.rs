//! Connection manager view: saved connections list plus an editable form.
use crate::I18n;
use crate::viewmodel::{ConnectionForm, Sort, StoredSecrets};
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
    /// Text typed in the filter box above the table.
    pub filter: String,
    pub sort: Sort,
    /// The folder name prompt (move a connection / rename a folder), when open.
    pub folder_prompt: Option<FolderPrompt>,
}

/// The one-field folder prompt. `input` is the text being typed.
#[derive(Debug, Clone, PartialEq)]
pub enum FolderPrompt {
    Move { id: Uuid, input: String },
    Rename { from: String, input: String },
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

    /// The filter box was edited: a stale import/export notice no longer applies.
    pub fn filter_edited(&mut self) {
        self.notice = None;
    }

    /// Opens the "move to folder" prompt for `id`, pre-filled with its current folder.
    pub fn open_move_prompt(&mut self, id: Uuid) {
        let input = self
            .connections
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.folder.clone())
            .unwrap_or_default();
        self.error = None;
        self.folder_prompt = Some(FolderPrompt::Move { id, input });
    }

    /// Opens the rename prompt for the folder `from`, pre-filled with its name.
    pub fn open_rename_prompt(&mut self, from: &str) {
        self.error = None;
        self.folder_prompt = Some(FolderPrompt::Rename {
            from: from.to_owned(),
            input: from.to_owned(),
        });
    }

    /// Cancels the prompt (Escape, outside click, Cancel); a rejected-name error goes with it.
    pub fn close_folder_prompt(&mut self) {
        self.folder_prompt = None;
        self.error = None;
    }

    /// The action the prompt stands for. The prompt stays open: the handler closes it on success
    /// and, on a rejected name, leaves it open and reports through `error`. Nothing is validated
    /// here.
    pub fn folder_prompt_confirmed(&mut self) -> Option<ConnectionAction> {
        self.folder_prompt.clone().map(|prompt| match prompt {
            FolderPrompt::Move { id, input } => {
                ConnectionAction::MoveToFolder { id, folder: input }
            }
            FolderPrompt::Rename { from, input } => {
                ConnectionAction::RenameFolder { from, to: input }
            }
        })
    }

    /// Distinct folder names in use, sorted.
    pub fn folder_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .connections
            .iter()
            .filter_map(|c| c.folder.clone())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    pub(super) fn select(&mut self, cfg: &ConnectionConfig) {
        self.reset_tools();
        self.delete_confirm = None;
        self.notice = None;
        self.selected = Some(cfg.id);
        self.form = ConnectionForm::from_config(cfg);
        self.error = None;
    }

    pub(super) fn start_new(&mut self) {
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
    state.invalidate_stale_test();
    ui.heading(i18n.t("connections.title"));
    let mut action = super::connections_list::show_list(ui, state, i18n);
    ui.separator();
    if let Some(saved) = super::connections_edit::show_form(ui, state, i18n) {
        action = Some(saved);
    }
    let prompt = super::connections_folder_prompt::show_folder_prompt(ui.ctx(), state, i18n);
    show_delete_confirm(ui.ctx(), state, i18n)
        .or(prompt)
        .or(action)
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
