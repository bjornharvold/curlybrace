//! Connection manager view: saved connections list plus an editable form.
use crate::I18n;
use crate::viewmodel::StoredSecrets;
use crate::viewmodel::{ConnectionForm, auth_label_key, auth_options};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
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
    MoveToFolder { id: Uuid, folder: String },
    RenameFolder { from: String, to: String },
    DeleteFolder(String),
    StartImport,
    StartExport,
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

    fn select(&mut self, cfg: &ConnectionConfig) {
        self.delete_confirm = None;
        self.notice = None;
        self.selected = Some(cfg.id);
        self.form = ConnectionForm::from_config(cfg);
        self.error = None;
    }

    fn start_new(&mut self) {
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
    if let Some(saved) = show_form(ui, state, i18n) {
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

fn show_form(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let form = &mut state.form;
    text_row(ui, &i18n.t("connections.name"), &mut form.name, false);
    text_row(ui, &i18n.t("connections.uri"), &mut form.uri, false);
    auth_combo(ui, form, i18n);
    text_row(
        ui,
        &i18n.t("connections.username"),
        &mut form.username,
        false,
    );
    text_row(
        ui,
        &i18n.t("connections.auth_source"),
        &mut form.auth_source,
        false,
    );
    let stored = state.stored.get(&form.id).copied().unwrap_or_default();
    secret_row(
        ui,
        SecretField {
            label: &i18n.t("connections.password"),
            value: &mut form.password,
            clear: &mut form.clear_password,
            stored: stored.password,
        },
        i18n,
    );
    tls_section(ui, form, i18n);
    ssh_section(ui, form, stored.ssh_secret, i18n);
    if let Some(e) = &state.error {
        super::error_label::show_error(ui, e, i18n);
    }
    if ui.button(i18n.t("common.save")).clicked() {
        return match state.form.to_config() {
            Ok(_) => {
                state.error = None;
                Some(ConnectionAction::Save(Box::new(state.form.clone())))
            }
            Err(e) => {
                state.error = Some(e);
                None
            }
        };
    }
    None
}

struct SecretField<'a> {
    label: &'a str,
    value: &'a mut String,
    clear: &'a mut bool,
    stored: bool,
}

/// Password input plus saved-secret hint and an explicit clear toggle.
fn secret_row(ui: &mut egui::Ui, field: SecretField<'_>, i18n: &I18n) {
    text_row(ui, field.label, field.value, true);
    if !field.value.is_empty() {
        *field.clear = false;
    } else if field.stored {
        ui.horizontal(|ui| {
            let hint = if *field.clear {
                "connections.saved_secret_will_clear"
            } else {
                "connections.password_saved_hint"
            };
            ui.label(i18n.t(hint));
            ui.checkbox(field.clear, i18n.t("connections.clear_saved_password"));
        });
    }
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    ui.horizontal(|ui| {
        let caption = ui.label(label);
        // Expose the caption as the box's accessible name (screen readers and UI tests).
        ui.add(egui::TextEdit::singleline(value).password(password))
            .labelled_by(caption.id);
    });
}

fn auth_combo(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    ui.horizontal(|ui| {
        ui.label(i18n.t("connections.auth"));
        egui::ComboBox::from_id_salt("connection_auth")
            .selected_text(i18n.t(auth_label_key(form.auth)))
            .show_ui(ui, |ui| {
                for option in auth_options() {
                    let label = i18n.t(auth_label_key(option));
                    ui.selectable_value(&mut form.auth, option, label);
                }
            });
    });
}

fn tls_section(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    ui.checkbox(&mut form.tls_enabled, i18n.t("connections.tls"));
    if form.tls_enabled {
        text_row(ui, &i18n.t("connections.tls_ca"), &mut form.tls_ca, false);
        let cert = i18n.t("connections.tls_client_cert");
        text_row(ui, &cert, &mut form.tls_client_cert, false);
        ui.checkbox(
            &mut form.allow_invalid_certs,
            i18n.t("connections.allow_invalid_certs"),
        );
        if form.ssh_enabled && form.allow_invalid_certs {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                i18n.t("connections.allow_invalid_certs_ssh_warning"),
            );
        }
    }
}

fn ssh_section(ui: &mut egui::Ui, form: &mut ConnectionForm, stored: bool, i18n: &I18n) {
    ui.checkbox(&mut form.ssh_enabled, i18n.t("connections.ssh"));
    if form.ssh_enabled {
        text_row(
            ui,
            &i18n.t("connections.ssh_host"),
            &mut form.ssh_host,
            false,
        );
        text_row(
            ui,
            &i18n.t("connections.ssh_port"),
            &mut form.ssh_port,
            false,
        );
        text_row(
            ui,
            &i18n.t("connections.ssh_user"),
            &mut form.ssh_user,
            false,
        );
        text_row(
            ui,
            &i18n.t("connections.ssh_key"),
            &mut form.ssh_key_path,
            false,
        );
        secret_row(
            ui,
            SecretField {
                label: &i18n.t("connections.ssh_secret"),
                value: &mut form.ssh_secret,
                clear: &mut form.clear_ssh_secret,
                stored,
            },
            i18n,
        );
    }
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
