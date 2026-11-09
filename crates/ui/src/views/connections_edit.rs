//! The connection edit form: fields, secrets and the TLS and SSH sections.
use super::connections::ConnectionsState;
use crate::I18n;
use crate::theme::{FieldOptions, outlined_field_opts, segmented};
use crate::viewmodel::{ConnectionForm, auth_label_key, auth_options, color_tag};
use curlybrace_core::connection::{AuthMechanism, ConnectionColor, SavedColor};
use eframe::egui;

/// The form body below the name and folder rows: connection string, authentication, secrets,
/// TLS and SSH. Saving is the dialog footer's job.
pub(super) fn show_fields(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) {
    let form = &mut state.form;
    text_row(ui, &i18n.t("connections.uri"), &mut form.uri, false);
    auth_choice(ui, form, i18n);
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
    tag_section(ui, form, i18n);
    tls_section(ui, form, i18n);
    ssh_section(ui, form, stored.ssh_secret, i18n);
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

pub(super) fn text_row(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    let options = FieldOptions {
        password,
        ..FieldOptions::default()
    };
    // The label floats on the field's border and is its accessible name.
    outlined_field_opts(ui, label, value, "", options);
}

/// The authentication mechanisms as a segmented control under a caption, so every choice is
/// visible at once and the current one is marked.
fn auth_choice(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    ui.label(i18n.t("connections.auth"));
    let items: Vec<(AuthMechanism, String)> = auth_options()
        .into_iter()
        .map(|option| (option, i18n.t(auth_label_key(option))))
        .collect();
    if let Some(picked) = segmented(ui, &items, form.auth) {
        form.auth = picked;
    }
}

/// Text of a colour option: its glyph and translated name, so colour is never the only signal.
fn color_text(color: Option<ConnectionColor>, i18n: &I18n) -> String {
    match color {
        Some(c) => {
            let tag = color_tag(c);
            format!("{} {}", tag.glyph, i18n.t(tag.name_key))
        }
        None => i18n.t("connections.color_none"),
    }
}

/// Colour tag picker, production flag and read-only flag.
fn tag_section(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    color_combo(ui, form, i18n);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut form.production, i18n.t("connections.production"));
        ui.label(i18n.t("connections.production_hint"));
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut form.read_only, i18n.t("connections.read_only"));
        ui.label(i18n.t("connections.read_only_hint"));
    });
}

fn color_combo(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    ui.horizontal(|ui| {
        let name = i18n.t("connections.color");
        let caption = ui.label(name.clone());
        let shown = form.color.as_ref().and_then(SavedColor::known);
        let selected = color_text(shown, i18n);
        let combo = egui::ComboBox::from_id_salt("connection_color")
            .selected_text(selected.clone())
            .show_ui(ui, |ui| {
                let options = std::iter::once(None).chain(ConnectionColor::ALL.map(Some));
                for option in options {
                    let label = color_text(option, i18n);
                    // A click is detected even on the entry already shown, so "No colour" can
                    // clear a stored colour this build does not recognise.
                    if ui.selectable_label(option == shown, label).clicked() {
                        form.color = option.map(SavedColor::Known);
                    }
                }
            });
        // Name the combo by its caption and keep the selected value as its value.
        combo.response.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, name.clone());
            info.current_text_value = Some(selected.clone());
            info
        });
        combo.response.labelled_by(caption.id);
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
        if form.allow_invalid_certs {
            let key = if form.ssh_enabled {
                "connections.allow_invalid_certs_ssh_warning"
            } else {
                "connections.allow_invalid_certs_warning"
            };
            ui.colored_label(ui.visuals().warn_fg_color, i18n.t(key));
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
