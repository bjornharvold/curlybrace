//! The connection manager list exposes its controls to the accesskit tree: a labelled filter
//! box, header buttons for every column, a named entry per row and labelled toolbar buttons.
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn three_connections() -> ConnectionsState {
    let mut in_folder = ConnectionConfig::new("Billing", "mongodb://billing.example.com");
    in_folder.folder = Some("Work".into());
    ConnectionsState::new(vec![
        ConnectionConfig::new("Alpha", "mongodb://alpha.example.com"),
        ConnectionConfig::new("Beta", "mongodb://beta.example.com"),
        in_folder,
    ])
}

fn harness() -> Harness<'static, ConnectionsState> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            three_connections(),
        );
    harness.run();
    harness
}

#[test]
fn the_filter_box_is_labelled_and_filters_the_rows() {
    let i18n = I18n::english();
    let mut harness = harness();
    let label = i18n.t("connections.filter");
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text("alpha");
    harness.run();
    assert_eq!(harness.state().filter, "alpha");
    harness.get_by_label("1 of 3");
}

#[test]
fn every_column_header_is_a_button_with_its_text() {
    let i18n = I18n::english();
    let harness = harness();
    let buttons: Vec<String> = harness
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    for key in [
        "connections.col_name",
        "connections.col_server",
        "connections.col_security",
        "connections.col_last_connected",
        "connections.col_last_modified",
    ] {
        let text = i18n.t(key);
        assert!(
            buttons.iter().any(|b| b.starts_with(&text)),
            "no header button for {text}: {buttons:?}"
        );
    }
}

#[test]
fn clicking_a_header_sorts_and_marks_the_active_column() {
    let i18n = I18n::english();
    let mut harness = harness();
    let server = i18n.t("connections.col_server");
    harness.get_by_role_and_label(Role::Button, &server).click();
    harness.run();
    assert_eq!(
        harness.state().sort.column,
        curlybrace_ui::viewmodel::SortColumn::Server
    );
    harness.get_by_role_and_label(Role::Button, &format!("{server} \u{25B2}"));
}

#[test]
fn each_row_exposes_its_name_including_rows_inside_a_folder() {
    let harness = harness();
    for name in ["Alpha", "Beta", "Billing"] {
        harness.get_by_role_and_label(Role::Button, name);
    }
}

#[test]
fn the_toolbar_buttons_are_labelled() {
    let i18n = I18n::english();
    let harness = harness();
    for key in [
        "connections.new",
        "connections.edit",
        "connections.duplicate",
        "common.delete",
        "connections.import",
        "connections.export",
        "connections.move_to_folder",
        "connections.rename_folder",
        "connections.delete_folder",
    ] {
        harness.get_by_role_and_label(Role::Button, &i18n.t(key));
    }
}

#[test]
fn a_blank_rename_cannot_be_confirmed_and_a_rejection_shows_inside_the_prompt() {
    use curlybrace_core::error::CoreError;
    use curlybrace_ui::views::connections::FolderPrompt;
    let i18n = I18n::english();
    let mut state = three_connections();
    state.folder_prompt = Some(FolderPrompt::Rename {
        from: "Work".into(),
        input: "  ".into(),
    });
    state.error = Some(CoreError::InvalidFolderName);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            state,
        );
    harness.run();
    let ok_label = i18n.t("common.ok");
    let ok = harness.get_by_role_and_label(Role::Button, &ok_label);
    assert!(ok.accesskit_node().is_disabled(), "blank rename is blocked");
    harness.get_by_label(&i18n.t("errors.invalid_folder_name"));
    assert!(harness.state().folder_prompt.is_some());
}
