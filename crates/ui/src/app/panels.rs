//! The thin egui layer: draws panels from the state and reports what the user did.
//! Every decision lives in the reducer and view-models; nothing here has logic worth testing
//! beyond "it renders" (covered by the headless frame tests in `mod.rs`).
use super::banner::{banner_detail, banner_prefix, banner_text};
use super::host_key::{HostKeyAnswer, PromptState, dialog_id};
use super::reducer::Input;
use super::state::{AppState, ConnStatus, Workspace};
use crate::I18n;
use crate::viewmodel::badge_of;
use crate::views::compare::{show_compare, show_workspace_switcher};
use crate::views::connections::{ConnectionAction, show_connections};
use crate::views::documents::{
    DocTarget, show_connection_header, show_documents_keyed, show_mode_switcher,
};
use crate::views::editor::show_editor_with;
use crate::views::findbar::show_findbar;
use crate::views::sidebar_roots::show_roots;
use crate::views::write_warning::show_write_warning;
use curlybrace_core::tunnel::HostKeyDecision;
use eframe::egui;

/// What one frame of drawing produced.
// Events are built and consumed once per frame; boxing `Input` would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum UiEvent {
    Input(Input),
    Connection(ConnectionAction),
    HostKey(HostKeyAnswer),
    ForgetConfirmed,
}

pub fn show_banner(ctx: &egui::Context, state: &AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    let Some(banner) = &state.banner else {
        return;
    };
    let (headline, detail) = banner_text(banner, i18n);
    let prefix = banner_prefix(state, i18n);
    egui::TopBottomPanel::top("banner").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            let color = ui.visuals().error_fg_color;
            if let Some(prefix) = prefix {
                ui.colored_label(color, prefix);
            }
            ui.colored_label(color, headline);
            if let Some(detail) = detail {
                ui.label(detail);
            }
            if let Some(raw) = banner_detail(banner) {
                ui.weak(raw);
            }
            if state.forgettable_host().is_some()
                && ui.button(i18n.t("banner.forget_host")).clicked()
            {
                out.push(UiEvent::Input(Input::ForgetRequested));
            }
            if ui.button(i18n.t("banner.dismiss")).clicked() {
                out.push(UiEvent::Input(Input::DismissBanner));
            }
        });
    });
}

/// The banner as plain lines for the edit dialog: headline, detail, raw server text. Empty
/// unless the dialog is open and the banner is new since it opened (see `dialog_banner`).
fn dialog_banner_lines(state: &AppState, i18n: &I18n) -> Vec<String> {
    let Some(banner) = state
        .banner
        .as_ref()
        .filter(|b| state.dialog_banner_watch.is_new(b))
    else {
        return Vec::new();
    };
    let (headline, detail) = banner_text(banner, i18n);
    let headline = match banner_prefix(state, i18n) {
        Some(prefix) => format!("{prefix} {headline}"),
        None => headline,
    };
    std::iter::once(headline)
        .chain(detail)
        .chain(banner_detail(banner).map(str::to_owned))
        .collect()
}

/// Whether the central area shows the connection manager: no session is connected in the
/// foreground, or the user asked for it from the tree.
pub fn shows_manager(state: &AppState) -> bool {
    state.workspace == Workspace::Connections || state.status != ConnStatus::Connected
}

pub fn show_connections_screen(
    ctx: &egui::Context,
    state: &mut AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    state.dialog_banner_watch = std::mem::take(&mut state.dialog_banner_watch)
        .observed(state.conn.editing, state.banner.as_ref());
    state.conn.dialog_banner = dialog_banner_lines(state, i18n);
    let opening = state
        .roots(i18n)
        .iter()
        .any(|root| root.status == ConnStatus::Connecting);
    egui::CentralPanel::default().show(ctx, |ui| {
        if opening {
            ui.label(i18n.t("connections.connecting"));
        }
        if let Some(action) = show_connections(ui, &mut state.conn, i18n) {
            out.push(UiEvent::Connection(action));
        }
    });
}

/// The tree of open sessions with the workspace switcher; drawn whenever a session is open.
pub fn show_sessions_panel(
    ctx: &egui::Context,
    state: &AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    egui::SidePanel::left("sidebar")
        .resizable(true)
        .show(ctx, |ui| {
            if let Some(action) = show_workspace_switcher(ui, state.workspace, i18n) {
                out.push(UiEvent::Input(Input::Compare(action)));
            }
            egui::ScrollArea::vertical()
                .id_salt("sidebar_roots")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if let Some(input) = show_roots(ui, state, i18n) {
                        out.push(UiEvent::Input(input));
                    }
                });
        });
}

/// The workspace of the foreground session (the manager is drawn by `show_connections_screen`).
pub fn show_workspace(
    ctx: &egui::Context,
    state: &mut AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    egui::CentralPanel::default().show(ctx, |ui| match state.workspace {
        Workspace::Browse | Workspace::Connections => show_collection(ui, state, i18n, out),
        Workspace::Compare => {
            if let Some(action) = show_compare(ui, &state.compare, &state.conn, i18n) {
                out.push(UiEvent::Input(Input::Compare(action)));
            }
        }
    });
}

fn show_collection(ui: &mut egui::Ui, state: &mut AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    if state.selected_ns().is_none() {
        ui.label(i18n.t("browser.select_collection"));
        return;
    }
    let connection = state.active_connection_label(i18n);
    let badge = state.active_badge();
    show_connection_header(ui, &connection, &badge, i18n);
    let (has_prev, has_next) = state.paging();
    if let Some(action) = show_findbar(ui, &mut state.find, has_prev, has_next, i18n) {
        out.push(UiEvent::Input(Input::Find(action)));
    }
    state.view_mode = show_mode_switcher(ui, state.view_mode, i18n);
    ui.separator();
    let (Some(page), Some(key)) = (&state.page, &state.page_key) else {
        let empty = if state.loading_page {
            "browser.loading"
        } else {
            "browser.load_failed"
        };
        ui.label(i18n.t(empty));
        return;
    };
    state.tree.projected = state.page_is_projected();
    let action = show_documents_keyed(
        ui,
        state.view_mode,
        page,
        key,
        &mut state.tree,
        DocTarget {
            connection: &connection,
            production: badge.production,
        },
        i18n,
    );
    if let Some(action) = action {
        out.push(UiEvent::Input(Input::Doc(action)));
    }
}

pub fn show_modals(
    ctx: &egui::Context,
    state: &mut AppState,
    prompt: &PromptState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    let save_enabled = state.pending.is_none();
    let connection = state.active_connection_label(i18n);
    let saved = state
        .editor
        .as_mut()
        .and_then(|editor| show_editor_with(ctx, editor, &connection, i18n, save_enabled));
    out.extend(saved.map(|a| UiEvent::Input(Input::Editor(a))));
    let decided = prompt.current().and_then(|(host, fingerprint)| {
        let asking = state.connecting_name_for_host(host, i18n);
        show_host_key_prompt(ctx, host, fingerprint, asking.as_deref(), i18n)
    });
    out.extend(decided.map(UiEvent::HostKey));
    show_write_warning_modal(ctx, state, i18n, out);
    if let Some(endpoint) = &state.forget_confirm {
        let port = endpoint.port.to_string();
        let message = i18n.t_with(
            "forget.message",
            &[("host", &endpoint.host), ("port", &port)],
        );
        show_forget_confirm(ctx, &message, i18n, out);
    }
}

/// Draws the production write warning whenever a write is parked, whichever session is active:
/// a parked write blocks every session's writes until it is answered. The connection is named
/// from the parked write's own session, not the active one.
fn show_write_warning_modal(
    ctx: &egui::Context,
    state: &AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    let Some(parked) = &state.write_warning else {
        return;
    };
    let config = parked.command.session().and_then(|id| state.connection(id));
    let name = config.map_or_else(|| i18n.t("doc.connection_unknown"), |c| c.name.clone());
    let badge = config.map(badge_of).unwrap_or_default();
    if let Some(action) = show_write_warning(ctx, parked, &name, &badge, i18n) {
        out.push(UiEvent::Input(Input::WriteWarning(action)));
    }
}

/// Unknown-host-key modal. Escape or clicking outside is a Reject (fail closed).
fn show_host_key_prompt(
    ctx: &egui::Context,
    host: &str,
    fingerprint: &str,
    asking: Option<&str>,
    i18n: &I18n,
) -> Option<HostKeyAnswer> {
    let mut decision = None;
    let modal = egui::Modal::new(dialog_id(host, fingerprint)).show(ctx, |ui| {
        ui.heading(i18n.t("hostkey.title"));
        if let Some(name) = asking {
            ui.label(i18n.t_with("hostkey.for_connection", &[("name", name)]));
        }
        ui.label(i18n.t_with("hostkey.message", &[("host", host)]));
        ui.label(i18n.t("hostkey.fingerprint"));
        ui.monospace(fingerprint);
        ui.horizontal(|ui| {
            // Reject comes first and owns the initial focus, so Enter can never trust a key.
            let reject = ui.button(i18n.t("hostkey.reject"));
            focus_once(ui, &reject, (host, fingerprint));
            if reject.clicked() {
                decision = Some(HostKeyDecision::Reject);
            }
            if ui.button(i18n.t("hostkey.trust_once")).clicked() {
                decision = Some(HostKeyDecision::Trust);
            }
            if ui.button(i18n.t("hostkey.trust_remember")).clicked() {
                decision = Some(HostKeyDecision::TrustAndRemember);
            }
        });
    });
    let decision = decision.or_else(|| modal.should_close().then_some(HostKeyDecision::Reject));
    // The answer names the question that was drawn, so it cannot answer a different one.
    decision.map(|decision| HostKeyAnswer::new(host, fingerprint, decision))
}

/// Gives `response` keyboard focus the first time the prompt for `question` is drawn.
fn focus_once(ui: &egui::Ui, response: &egui::Response, question: (&str, &str)) {
    // Keyed by host and fingerprint only: two questions with the same pair are the same trust
    // decision and intentionally share an id, so the second gets no initial focus.
    let id = egui::Id::new(("host_key_focus", question));
    let seen = ui.data_mut(|d| d.get_temp::<bool>(id).is_some());
    if !seen {
        response.request_focus();
        ui.data_mut(|d| d.insert_temp(id, true));
    }
}

fn show_forget_confirm(ctx: &egui::Context, message: &str, i18n: &I18n, out: &mut Vec<UiEvent>) {
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::new("forget_host")).show(ctx, |ui| {
        ui.heading(i18n.t("forget.title"));
        ui.label(message);
        ui.horizontal(|ui| {
            confirmed = ui.button(i18n.t("forget.confirm")).clicked();
            cancelled = ui.button(i18n.t("common.cancel")).clicked();
        });
    });
    if confirmed {
        out.push(UiEvent::ForgetConfirmed);
    } else if cancelled || modal.should_close() {
        out.push(UiEvent::Input(Input::ForgetCancelled));
    }
}
