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
#[derive(Default)]
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
    /// The edit dialog is open.
    pub editing: bool,
    /// The app banner as text (headline first, then details), copied in by the screen each frame
    /// so the dialog can show it: the real banner is dimmed behind the modal backdrop.
    pub dialog_banner: Vec<String>,
}

/// `Debug` hides `uri_input`: it is the pasted connection string and may hold a password.
impl std::fmt::Debug for ConnectionsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Exhaustive on purpose (no `..`): a new field breaks the build here and forces a
        // decision on whether it may be printed. `uri_input` is a pasted string that may hold a
        // password; `built_uri` is a `SecretString` and the forms redact their own secrets.
        let Self {
            connections,
            selected,
            form,
            error,
            stored,
            delete_confirm,
            notice,
            test,
            test_seq,
            test_form,
            uri_input: _,
            built_uri,
            built_for,
            built_with_password,
            filter,
            sort,
            folder_prompt,
            editing,
            dialog_banner,
        } = self;
        f.debug_struct("ConnectionsState")
            .field("connections", connections)
            .field("selected", selected)
            .field("form", form)
            .field("error", error)
            .field("stored", stored)
            .field("delete_confirm", delete_confirm)
            .field("notice", notice)
            .field("test", test)
            .field("test_seq", test_seq)
            .field("test_form", test_form)
            .field("uri_input", &"<redacted>")
            .field("built_uri", built_uri)
            .field("built_for", built_for)
            .field("built_with_password", built_with_password)
            .field("filter", filter)
            .field("sort", sort)
            .field("folder_prompt", folder_prompt)
            .field("editing", editing)
            .field("dialog_banner", dialog_banner)
            .finish()
    }
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
        // A result for an outdated form is dropped. A test that is still running keeps running:
        // its eventual `Done` lands here on a later frame and is discarded the same way, and a
        // new Test click supersedes it (the request counter only accepts the latest answer).
        if self.test_form.as_ref() != Some(&self.form)
            && matches!(self.test, TestState::Done { .. })
        {
            self.test = TestState::Idle;
            // The cloned form may hold a typed secret; do not keep it once there is no result.
            self.test_form = None;
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

    /// Opens the edit dialog for `cfg` (toolbar Edit).
    pub(super) fn begin_edit(&mut self, cfg: &ConnectionConfig) {
        self.select(cfg);
        self.editing = true;
    }

    /// Closes the dialog without saving (Cancel, Escape, outside click).
    pub fn close_edit(&mut self) {
        self.editing = false;
        self.error = None;
        self.reset_tools();
        // A cancelled form must not keep a typed password: reload it without secrets.
        let selected = self
            .selected
            .and_then(|id| self.connections.iter().find(|c| c.id == id));
        self.form = selected.map_or_else(ConnectionForm::default, ConnectionForm::from_config);
    }

    /// A save went through: the dialog closes, its tools reset and the notice is dropped.
    pub fn saved(&mut self) {
        self.close_edit();
        self.notice = None;
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
        self.editing = true;
    }
}

pub fn show_connections(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    state.invalidate_stale_test();
    ui.heading(i18n.t("connections.title"));
    let action = super::connections_list::show_list(ui, state, i18n);
    let edit = super::connections_edit_dialog::show_edit_dialog(ui.ctx(), state, i18n);
    let prompt = super::connections_folder_prompt::show_folder_prompt(ui.ctx(), state, i18n);
    show_delete_confirm(ui.ctx(), state, i18n)
        .or(prompt)
        .or(edit)
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

    fn valid_form() -> ConnectionForm {
        ConnectionForm {
            name: "n".into(),
            uri: "mongodb://localhost:27017".into(),
            ..ConnectionForm::default()
        }
    }

    fn report() -> TestReport {
        TestReport {
            server_version: None,
            tls: false,
            authenticated: false,
        }
    }

    #[test]
    fn editing_the_form_after_a_test_clears_the_result_and_the_secret_clone() {
        let mut conn = ConnectionsState {
            form: ConnectionForm {
                password: "typed".into(),
                ..valid_form()
            },
            ..ConnectionsState::default()
        };
        conn.test_form = Some(conn.form.clone());
        conn.test = TestState::Done {
            request_id: 1,
            outcome: Ok(report()),
        };
        conn.invalidate_stale_test();
        assert!(
            matches!(conn.test, TestState::Done { .. }),
            "unchanged form keeps it"
        );
        conn.form.uri = "mongodb://other".into();
        conn.invalidate_stale_test();
        assert_eq!(conn.test, TestState::Idle);
        assert_eq!(conn.test_form, None, "no typed secret lingers");
    }

    #[test]
    fn editing_the_form_clears_a_built_uri_but_not_a_running_test() {
        let mut conn = ConnectionsState {
            form: valid_form(),
            ..ConnectionsState::default()
        };
        conn.built_for = Some(conn.form.clone());
        conn.built_uri = Some(SecretString::new("x"));
        conn.built_with_password = true;
        conn.test = TestState::Running { request_id: 1 };
        conn.invalidate_stale_test();
        assert!(conn.built_uri.is_some());
        conn.form.name = "changed".into();
        conn.invalidate_stale_test();
        assert!(conn.built_uri.is_none() && conn.built_for.is_none());
        assert!(!conn.built_with_password);
        assert_eq!(conn.test, TestState::Running { request_id: 1 });
    }

    #[test]
    fn apply_uri_input_fills_the_form_and_clears_the_buffer() {
        let mut conn = ConnectionsState {
            uri_input: "mongodb://bob:pw@h/db".into(),
            error: Some(CoreError::AuthFailed),
            ..ConnectionsState::default()
        };
        conn.apply_uri_input();
        assert!(conn.uri_input.is_empty());
        assert_eq!(conn.form.uri, "mongodb://h/db");
        assert_eq!(conn.form.password, "pw");
        assert!(conn.error.is_none());
    }

    #[test]
    fn apply_uri_input_failure_clears_the_buffer_and_does_not_echo_it() {
        let mut conn = ConnectionsState {
            uri_input: "notmongo://bob:secretpw@h".into(),
            ..ConnectionsState::default()
        };
        let before = conn.form.clone();
        conn.apply_uri_input();
        assert!(conn.uri_input.is_empty());
        assert_eq!(conn.form, before);
        let error = conn.error.clone().expect("error");
        assert!(!format!("{error:?} {error}").contains("secretpw"));
        assert!(!format!("{conn:?}").contains("secretpw"));
    }

    #[test]
    fn opening_new_or_edit_opens_the_dialog_and_saved_closes_it() {
        let mut s = ConnectionsState::new(vec![ConnectionConfig::new("a", "mongodb://h")]);
        s.start_new();
        assert!(s.editing);
        s.editing = false;
        let cfg = s.connections[0].clone();
        s.select(&cfg);
        assert!(!s.editing, "a row click only selects");
        s.begin_edit(&cfg);
        assert!(s.editing && s.form.name == "a");
        s.saved();
        assert!(!s.editing);
    }

    #[test]
    fn opening_the_dialog_clears_a_stale_notice_and_saving_drops_it_too() {
        let mut s = ConnectionsState::new(vec![mock_connection()]);
        s.notice = Some(Notice::Exported { count: 1 });
        s.start_new();
        assert_eq!(s.notice, None);
        s.notice = Some(Notice::Exported { count: 1 });
        let cfg = s.connections[0].clone();
        s.begin_edit(&cfg);
        assert_eq!(s.notice, None);
        s.notice = Some(Notice::Imported {
            imported: 1,
            skipped: 0,
        });
        s.saved();
        assert_eq!(s.notice, None);
    }

    #[test]
    fn cancel_closes_the_dialog_and_resets_the_tools() {
        let mut s = ConnectionsState::new(vec![mock_connection()]);
        s.start_new();
        s.uri_input = "mongodb://u:p@h".into();
        s.test = TestState::Running { request_id: 3 };
        s.test_form = Some(s.form.clone());
        s.built_uri = Some(SecretString::new("x"));
        s.built_for = Some(s.form.clone());
        s.error = Some(CoreError::InvalidUri("x".into()));
        s.form.password = "typed".into();
        s.close_edit();
        assert!(
            s.form.password.is_empty(),
            "a cancelled password is dropped"
        );
        assert!(!s.editing && s.error.is_none());
        assert!(s.uri_input.is_empty() && s.test == TestState::Idle);
        assert!(s.test_form.is_none() && s.built_uri.is_none());
    }

    #[test]
    fn the_dialog_renders_with_a_test_result_and_a_built_uri() {
        let cfg = mock_connection();
        let mut state = ConnectionsState::new(vec![cfg.clone()]);
        state.begin_edit(&cfg);
        state.form.tls_enabled = true;
        state.form.allow_invalid_certs = true;
        state.test_form = Some(state.form.clone());
        state.test = TestState::Done {
            request_id: 1,
            outcome: Ok(TestReport {
                server_version: Some("7.0.1".into()),
                tls: true,
                authenticated: true,
            }),
        };
        state.built_for = Some(state.form.clone());
        state.built_uri = Some(SecretString::new("mongodb://u:pw@h"));
        state.built_with_password = true;
        assert_eq!(run_frame(&mut state), None);
        assert!(state.editing, "still open");
        assert!(state.built_uri.is_some(), "a frame keeps a current result");
        state.test = TestState::Done {
            request_id: 2,
            outcome: Err(CoreError::AuthFailed),
        };
        state.test_form = Some(state.form.clone());
        assert_eq!(run_frame(&mut state), None);
    }
}
