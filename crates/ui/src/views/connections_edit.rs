//! The connection edit form: fields, secrets and the TLS and SSH sections.
use super::connections::ConnectionsState;
use crate::I18n;
use crate::viewmodel::{ConnectionForm, auth_label_key, auth_options, color_tag};
use curlybrace_core::connection::{ConnectionColor, SavedColor};
use eframe::egui;

/// The form body below the name and folder rows: connection string, authentication, secrets,
/// TLS and SSH. Saving is the dialog footer's job.
pub(super) fn show_fields(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) {
    let form = &mut state.form;
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
    ui.horizontal(|ui| {
        let caption = ui.label(label);
        // Expose the caption as the box's accessible name (screen readers and UI tests).
        ui.add(egui::TextEdit::singleline(value).password(password))
            .labelled_by(caption.id);
    });
}

fn auth_combo(ui: &mut egui::Ui, form: &mut ConnectionForm, i18n: &I18n) {
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("connections.auth"));
        let selected = i18n.t(auth_label_key(form.auth));
        let combo = egui::ComboBox::from_id_salt("connection_auth")
            .selected_text(selected.clone())
            .show_ui(ui, |ui| {
                for option in auth_options() {
                    let label = i18n.t(auth_label_key(option));
                    ui.selectable_value(&mut form.auth, option, label);
                }
            });
        // A combo's accessible name is its selected text; name it by its caption instead, and
        // keep the selected value as the widget's value.
        let name = i18n.t("connections.auth");
        combo.response.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, name.clone());
            info.current_text_value = Some(selected.clone());
            info
        });
        combo.response.labelled_by(caption.id);
    });
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
    ui.horizontal(|ui| {
        ui.checkbox(&mut form.production, i18n.t("connections.production"));
        ui.label(i18n.t("connections.production_hint"));
    });
    ui.horizontal(|ui| {
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
