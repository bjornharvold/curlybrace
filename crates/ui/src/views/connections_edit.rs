//! The connection edit form: fields, secrets and the TLS and SSH sections.
use super::connections::{ConnectionAction, ConnectionsState};
use crate::I18n;
use crate::viewmodel::{ConnectionForm, auth_label_key, auth_options};
use eframe::egui;

pub(super) fn show_form(
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
