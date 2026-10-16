//! The connection manager list: toolbar, filter, sortable table grouped by folder.
use super::connections::{ConnectionAction, ConnectionsState, Notice};
use crate::I18n;
use crate::viewmodel::{
    SortColumn, auth_label_key, format_utc, grouped, security, servers, visible,
};
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use eframe::egui;

const SORT_ASC: &str = "\u{23F6}";
const SORT_DESC: &str = "\u{23F7}";
/// Share of the table width per column, in `COLUMNS` order.
const SHARES: [f32; 5] = [0.25, 0.22, 0.15, 0.19, 0.19];
const COLUMNS: [(SortColumn, &str); 5] = [
    (SortColumn::Name, "connections.col_name"),
    (SortColumn::Server, "connections.col_server"),
    (SortColumn::Security, "connections.col_security"),
    (SortColumn::LastConnected, "connections.col_last_connected"),
    (SortColumn::LastModified, "connections.col_last_modified"),
];
const CONNECT_BUTTON_WIDTH: f32 = 90.0;
const TABLE_MAX_HEIGHT: f32 = 320.0;

pub(super) fn show_list(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    // The list is cloned, filtered and sorted once per frame, and once more in the frame where
    // the filter box is edited. The clone lets the rows borrow something other than `state`.
    let list = state.connections.clone();
    let mut rows = visible(&list, &state.filter, state.sort);
    let mut action = show_toolbar(ui, state, i18n, &rows);
    show_filter(ui, state, i18n, &list, &mut rows);
    // While the edit dialog or the folder prompt is open the error is shown inside it.
    if let (Some(error), None, false) = (&state.error, &state.folder_prompt, state.editing) {
        super::error_label::show_error(ui, error, i18n);
    }
    if let Some(notice) = &state.notice {
        ui.label(notice_text(notice, i18n));
    }
    if let Some(row_action) = show_table(ui, state, i18n, &rows) {
        action = Some(row_action);
    }
    action
}

fn notice_text(notice: &Notice, i18n: &I18n) -> String {
    match notice {
        Notice::Imported { imported, skipped } => i18n.t_with(
            "connections.import_done",
            &[
                ("imported", &imported.to_string()),
                ("skipped", &skipped.to_string()),
            ],
        ),
        Notice::Exported { count } => {
            i18n.t_with("connections.export_done", &[("count", &count.to_string())])
        }
        Notice::AppliesOnReconnect => i18n.t("connections.edit_open_notice"),
    }
}

fn show_toolbar(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
    rows: &[&ConnectionConfig],
) -> Option<ConnectionAction> {
    let mut action = None;
    // Only a selection the current filter still shows can be acted on from the toolbar. The rows
    // are those from before this frame's filter edit, so the buttons follow an edit one frame late.
    let selected = state
        .selected
        .and_then(|id| rows.iter().find(|c| c.id == id))
        .copied()
        .cloned();
    ui.horizontal_wrapped(|ui| {
        let tool = |ui: &mut egui::Ui, enabled: bool, key: &str| {
            ui.add_enabled(enabled, egui::Button::new(i18n.t(key)))
                .clicked()
        };
        let current = selected.as_ref();
        if tool(ui, true, "connections.new") {
            state.start_new();
        }
        if tool(ui, current.is_some(), "connections.edit") {
            current.iter().for_each(|cfg| state.begin_edit(cfg));
        }
        if tool(ui, current.is_some(), "connections.duplicate") {
            action = current.map(|cfg| ConnectionAction::Duplicate(cfg.id));
        }
        if tool(ui, current.is_some(), "common.delete") {
            current
                .iter()
                .for_each(|cfg| state.delete_requested(cfg.id));
        }
        if tool(ui, true, "connections.import") {
            action = Some(ConnectionAction::StartImport);
        }
        if tool(ui, true, "connections.export") {
            action = Some(ConnectionAction::StartExport);
        }
        if tool(ui, current.is_some(), "connections.move_to_folder") {
            current
                .iter()
                .for_each(|cfg| state.open_move_prompt(cfg.id));
        }
    });
    action
}

/// Draws the filter box and the "shown of total" count. When the text was edited, `rows` is
/// recomputed first, so the count and the table (drawn from `rows` afterwards) agree this frame.
fn show_filter<'a>(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
    list: &'a [ConnectionConfig],
    rows: &mut Vec<&'a ConnectionConfig>,
) {
    ui.horizontal(|ui| {
        // The caption is the box's accessible name (a hint is not exposed to screen readers).
        let caption = ui.label(i18n.t("connections.filter"));
        let response = ui
            .add(egui::TextEdit::singleline(&mut state.filter).desired_width(240.0))
            .labelled_by(caption.id);
        if response.changed() {
            state.filter_edited();
            *rows = visible(list, &state.filter, state.sort);
        }
        let total = state.connections.len();
        ui.label(i18n.t_with(
            "connections.filter_count",
            &[
                ("shown", &rows.len().to_string()),
                ("total", &total.to_string()),
            ],
        ));
    });
}

fn show_table(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
    rows: &[&ConnectionConfig],
) -> Option<ConnectionAction> {
    let widths = column_widths(ui);
    show_header(ui, state, i18n, &widths);
    if rows.is_empty() && !state.filter.trim().is_empty() {
        ui.label(i18n.t("connections.no_match"));
        return None;
    }
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("connection_table")
        .max_height(TABLE_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for group in grouped(rows) {
                let found = match group.folder {
                    None => show_rows(ui, state, i18n, &group.rows, &widths),
                    Some(folder) => show_folder(ui, state, i18n, folder, &group.rows, &widths),
                };
                action = found.or(action.take());
            }
        });
    action
}

fn column_widths(ui: &egui::Ui) -> [f32; 5] {
    let spacing = ui.spacing().item_spacing.x;
    let usable = (ui.available_width() - CONNECT_BUTTON_WIDTH - spacing * 6.0).max(200.0);
    SHARES.map(|share| usable * share)
}

fn show_header(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n, widths: &[f32; 5]) {
    ui.horizontal(|ui| {
        for ((column, key), width) in COLUMNS.into_iter().zip(widths) {
            let mut label = i18n.t(key);
            if state.sort.column == column {
                label.push(' ');
                label.push_str(if state.sort.descending {
                    SORT_DESC
                } else {
                    SORT_ASC
                });
            }
            cell(ui, *width, |ui| {
                if ui
                    .add(egui::Button::new(label).frame(false).truncate())
                    .clicked()
                {
                    state.sort = state.sort.toggled(column);
                }
            });
        }
    });
}

fn show_folder(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
    folder: &str,
    rows: &[&ConnectionConfig],
    widths: &[f32; 5],
) -> Option<ConnectionAction> {
    let mut action = None;
    let id = ui.make_persistent_id(("connection_folder", folder));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(folder);
            if ui
                .small_button(i18n.t("connections.rename_folder"))
                .clicked()
            {
                state.open_rename_prompt(folder);
            }
            if ui
                .small_button(i18n.t("connections.delete_folder"))
                .clicked()
            {
                action = Some(ConnectionAction::DeleteFolder(folder.to_owned()));
            }
        })
        .body_unindented(|ui| {
            if let Some(row_action) = show_rows(ui, state, i18n, rows, widths) {
                action = Some(row_action);
            }
        });
    action
}

fn show_rows(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
    rows: &[&ConnectionConfig],
    widths: &[f32; 5],
) -> Option<ConnectionAction> {
    let mut action = None;
    for cfg in rows {
        // Keyed by connection so widget state follows it across a sort or filter change.
        ui.push_id(cfg.id, |ui| {
            ui.horizontal(|ui| {
                let selected = state.selected == Some(cfg.id);
                cell(ui, widths[0], |ui| {
                    let name = ui.add(egui::Button::selectable(selected, &cfg.name).truncate());
                    if name.double_clicked() {
                        action = Some(ConnectionAction::Connect(cfg.id));
                    } else if name.clicked() {
                        state.select(cfg);
                    }
                });
                let texts = [
                    servers(cfg),
                    security_text(cfg, i18n),
                    time_text(cfg.last_connected, i18n),
                    time_text(cfg.updated_at, i18n),
                ];
                for (text, width) in texts.into_iter().zip(&widths[1..]) {
                    cell(ui, *width, |ui| {
                        ui.add(egui::Label::new(text).truncate());
                    });
                }
                let connect = ui.button(i18n.t("connections.connect"));
                // The visible text is the same on every row; the accessible name says which.
                // Built inside the closure: egui calls it only on interaction or while AccessKit is on.
                connect.widget_info(|| {
                    let name = i18n.t_with("connections.connect_named", &[("name", &cfg.name)]);
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                });
                if connect.clicked() {
                    action = Some(ConnectionAction::Connect(cfg.id));
                }
            })
        });
    }
    action
}

/// A fixed-width table cell so columns line up across folder groups.
fn cell(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(width);
            add(ui);
        },
    );
}

/// "TLS, SSH tunnel, SCRAM-SHA-256", or the "none" text when nothing is configured.
pub(super) fn security_text(cfg: &ConnectionConfig, i18n: &I18n) -> String {
    let sec = security(cfg);
    let mut parts = Vec::new();
    if sec.tls {
        parts.push(i18n.t("connections.security_tls"));
    }
    if sec.ssh {
        parts.push(i18n.t("connections.security_ssh"));
    }
    if sec.auth != AuthMechanism::None {
        parts.push(i18n.t(auth_label_key(sec.auth)));
    }
    if parts.is_empty() {
        i18n.t("connections.security_none")
    } else {
        parts.join(", ")
    }
}

/// A timestamp labelled as UTC, or the "never" text when unset or out of the displayable range.
pub(super) fn time_text(secs: Option<i64>, i18n: &I18n) -> String {
    match secs.map(format_utc) {
        Some(text) if text != "-" => i18n.t_with("connections.time_utc", &[("time", &text)]),
        _ => i18n.t("connections.never"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_text_is_never_for_none_and_for_out_of_range_values() {
        let i18n = I18n::english();
        assert_eq!(time_text(None, &i18n), "Never");
        assert_eq!(time_text(Some(i64::MAX), &i18n), "Never");
        assert_eq!(time_text(Some(0), &i18n), "1970-01-01 00:00 UTC");
        assert_eq!(time_text(Some(1_700_000_000), &i18n), "2023-11-14 22:13 UTC");
    }

    #[test]
    fn security_text_lists_what_is_configured_or_says_none() {
        let i18n = I18n::english();
        let plain = ConnectionConfig::new("a", "mongodb://h");
        assert_eq!(security_text(&plain, &i18n), "None");
        let mut tls = plain.clone();
        tls.tls.enabled = true;
        assert_eq!(security_text(&tls, &i18n), "TLS");
    }
}
