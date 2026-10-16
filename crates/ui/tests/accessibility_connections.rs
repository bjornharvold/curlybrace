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
    harness.get_by_role_and_label(Role::Button, &format!("{server} \u{23F6}"));
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

fn dialog_harness(
    prepare: impl FnOnce(&mut ConnectionsState),
) -> Harness<'static, ConnectionsState> {
    let mut state = three_connections();
    let first = state.connections[0].clone();
    state.form = curlybrace_ui::viewmodel::ConnectionForm::from_config(&first);
    state.editing = true;
    prepare(&mut state);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            state,
        );
    harness.run();
    harness
}

#[test]
fn every_field_of_the_edit_dialog_has_a_label() {
    let harness = dialog_harness(|state| {
        state.form.tls_enabled = true;
        state.form.ssh_enabled = true;
        state.built_for = Some(state.form.clone());
        state.built_uri = Some(curlybrace_core::secrets::SecretString::new("mongodb://h"));
    });
    let mut fields = 0;
    for role in [Role::TextInput, Role::PasswordInput] {
        for node in harness.query_all_by_role(role) {
            fields += 1;
            let label = node.accesskit_node().label().unwrap_or_default();
            assert!(!label.trim().is_empty(), "an unlabelled field: {role:?}");
        }
    }
    // filter + name, folder, from-uri, uri, username, auth source, password, tls ca, tls cert,
    // ssh host/port/user/key/secret, built uri.
    assert!(fields >= 15, "only {fields} fields found");
}

#[test]
fn the_dialog_actions_are_buttons() {
    let i18n = I18n::english();
    let harness = dialog_harness(|_| {});
    for key in [
        "connections.test",
        "connections.to_uri",
        "connections.include_password",
        "connections.from_uri",
        "common.cancel",
        "common.save",
    ] {
        harness.get_by_role_and_label(Role::Button, &i18n.t(key));
    }
}

#[test]
fn a_test_result_and_a_built_uri_show_their_text_and_the_password_warning() {
    use curlybrace_core::runtime::TestReport;
    use curlybrace_ui::views::connections::TestState;
    let i18n = I18n::english();
    let harness = dialog_harness(|state| {
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
        state.built_uri = Some(curlybrace_core::secrets::SecretString::new("mongodb://h"));
        state.built_with_password = true;
    });
    harness.get_by_label(&i18n.t_with("connections.test_ok", &[("version", "7.0.1")]));
    harness.get_by_label(&i18n.t("connections.test_configured_tls"));
    harness.get_by_label(&i18n.t("connections.test_configured_auth"));
    harness.get_by_label(&i18n.t("connections.uri_password_warning"));
    harness.get_by_role_and_label(Role::Button, &i18n.t("connections.copy"));
}

#[test]
fn a_long_test_error_is_shown_in_full_outside_the_scrolling_body() {
    use curlybrace_core::error::CoreError;
    use curlybrace_ui::views::connections::TestState;
    let detail = "connection refused by the server after a very long wait; ".repeat(6);
    let harness = dialog_harness(move |state| {
        state.test_form = Some(state.form.clone());
        state.test = TestState::Done {
            request_id: 1,
            outcome: Err(CoreError::Database(detail.clone())),
        };
    });
    harness.get_by_label_contains("connection refused by the server");
}

#[test]
fn the_validation_error_shows_inside_the_dialog() {
    use curlybrace_core::error::CoreError;
    let i18n = I18n::english();
    let harness = dialog_harness(|state| state.error = Some(CoreError::InvalidName));
    harness.get_by_label(&i18n.t("errors.invalid_name"));
}

#[test]
fn the_folder_drop_down_has_the_folder_label() {
    let i18n = I18n::english();
    let harness = dialog_harness(|_| {});
    harness.get_by_role_and_label(Role::ComboBox, &i18n.t("connections.folder"));
}

#[test]
fn the_authentication_drop_down_has_the_authentication_label() {
    let i18n = I18n::english();
    let harness = dialog_harness(|_| {});
    harness.get_by_role_and_label(Role::ComboBox, &i18n.t("connections.auth"));
}

#[test]
fn a_banner_is_repeated_inside_the_open_dialog() {
    let harness = dialog_harness(|state| {
        state.dialog_banner = vec!["Could not save".into(), "keychain said no".into()];
    });
    harness.get_by_label("Could not save");
    harness.get_by_label("keychain said no");
}

const SELECTION_TOOLS: [&str; 4] = [
    "connections.edit",
    "connections.duplicate",
    "common.delete",
    "connections.move_to_folder",
];

fn tools_enabled(harness: &Harness<'static, ConnectionsState>) -> Vec<bool> {
    let i18n = I18n::english();
    SELECTION_TOOLS
        .iter()
        .map(|key| {
            let label = i18n.t(key);
            let node = harness.get_by_role_and_label(Role::Button, &label);
            !node.accesskit_node().is_disabled()
        })
        .collect()
}

#[test]
fn selection_tools_follow_whether_the_selected_row_is_visible() {
    let mut harness = harness();
    let alpha = harness.state().connections[0].clone();
    harness.state_mut().selected = Some(alpha.id);
    harness.run();
    assert_eq!(
        tools_enabled(&harness),
        [true; 4],
        "no filter: selected row shown"
    );

    harness.state_mut().filter = "beta".into();
    harness.run();
    assert_eq!(
        tools_enabled(&harness),
        [false; 4],
        "the filter hides the selected connection"
    );

    harness.state_mut().filter = "alpha".into();
    harness.run();
    assert_eq!(tools_enabled(&harness), [true; 4], "a matching filter");

    harness.state_mut().filter = "beta".into();
    harness.run();
    harness.state_mut().filter.clear();
    harness.run();
    assert_eq!(tools_enabled(&harness), [true; 4], "clearing the filter");
}

#[test]
fn new_import_and_export_stay_enabled_when_the_selection_is_hidden() {
    let i18n = I18n::english();
    let mut harness = harness();
    let alpha = harness.state().connections[0].clone();
    harness.state_mut().selected = Some(alpha.id);
    harness.state_mut().filter = "beta".into();
    harness.run();
    for key in [
        "connections.new",
        "connections.import",
        "connections.export",
    ] {
        let label = i18n.t(key);
        let node = harness.get_by_role_and_label(Role::Button, &label);
        assert!(!node.accesskit_node().is_disabled(), "{key}");
    }
}

#[test]
fn each_row_connect_button_is_named_after_its_connection() {
    let i18n = I18n::english();
    let harness = harness();
    for name in ["Alpha", "Beta", "Billing"] {
        let label = i18n.t_with("connections.connect_named", &[("name", name)]);
        harness.get_by_role_and_label(Role::Button, &label);
    }
    assert_eq!(
        harness
            .query_all_by_role(Role::Button)
            .filter(|n| n
                .accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Connect ")))
            .count(),
        3,
        "one distinctly named Connect button per row"
    );
}

fn prompt_harness(open: impl FnOnce(&mut ConnectionsState)) -> Harness<'static, ConnectionsState> {
    let mut state = three_connections();
    open(&mut state);
    Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            state,
        )
}

#[test]
fn the_folder_prompt_field_has_focus_when_it_opens_and_typing_reaches_it() {
    use curlybrace_ui::views::connections::FolderPrompt;
    let i18n = I18n::english();
    for rename in [false, true] {
        let mut harness = prompt_harness(|state| {
            if rename {
                state.open_rename_prompt("Work");
            } else {
                let id = state.connections[0].id;
                state.open_move_prompt(id);
            }
        });
        harness.step();
        harness.step();
        let title = i18n.t(if rename {
            "connections.folder_rename_title"
        } else {
            "connections.folder_move_title"
        });
        let field = harness.get_by_role_and_label(Role::TextInput, &title);
        assert!(field.accesskit_node().is_focused(), "rename={rename}");
        harness
            .get_by_role_and_label(Role::TextInput, &title)
            .type_text("X");
        harness.run();
        let typed = match harness.state().folder_prompt.clone() {
            Some(FolderPrompt::Move { input, .. } | FolderPrompt::Rename { input, .. }) => input,
            None => String::new(),
        };
        assert!(typed.ends_with('X'), "typing reached the field: {typed:?}");
    }
}

#[test]
fn the_folder_prompt_does_not_take_focus_back_after_the_person_tabs_away() {
    let i18n = I18n::english();
    let mut harness = prompt_harness(|state| state.open_rename_prompt("Work"));
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.run();
    let title = i18n.t("connections.folder_rename_title");
    let field = harness.get_by_role_and_label(Role::TextInput, &title);
    assert!(
        !field.accesskit_node().is_focused(),
        "focus must stay where Tab put it"
    );
}
