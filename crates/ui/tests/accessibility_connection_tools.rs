//! The tool paths section of the connection dialog in the accesskit tree: a saved connection
//! gets a "use global settings" checkbox per tool, an unsaved one is told to save first, and a
//! tool older than the server the person just tested against is called out. Fast, offline, no
//! GPU.
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::TestReport;
use curlybrace_core::tools::{ToolChoice, ToolKind, ToolVersion};
use curlybrace_ui::I18n;
use curlybrace_ui::app::tools_state::{ProbeScope, ToolsAction, ToolsState};
use curlybrace_ui::viewmodel::ConnectionForm;
use curlybrace_ui::views::connections::{
    ConnectionAction, ConnectionsState, TestState, show_connections_with_tools,
};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

struct Run {
    conn: ConnectionsState,
    tools: ToolsState,
    actions: Vec<ConnectionAction>,
}

fn harness(conn: ConnectionsState, tools: ToolsState) -> Harness<'static, Run> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 1200.0))
        .build_ui_state(
            |ui, run: &mut Run| {
                let action =
                    show_connections_with_tools(ui, &mut run.conn, &run.tools, &I18n::english());
                run.actions.extend(action);
            },
            Run {
                conn,
                tools,
                actions: Vec::new(),
            },
        );
    harness.run();
    harness
}

fn editing(cfg: &ConnectionConfig) -> ConnectionsState {
    let mut state = ConnectionsState::new(vec![cfg.clone()]);
    state.editing = true;
    state.form = ConnectionForm::from_config(cfg);
    state
}

/// Clicks the first "use global settings" box the way a person does: scrolled into view first,
/// because the dialog body is a scroll area and the section sits below its fold.
fn click_first_use_global(harness: &mut Harness<'static, Run>) {
    let label = use_global_label(ToolKind::Mongosh);
    harness
        .get_by_role_and_label(Role::CheckBox, &label)
        .scroll_to_me();
    harness.run();
    harness
        .get_by_role_and_label(Role::CheckBox, &label)
        .click();
    harness.run();
}

fn use_global_label(kind: ToolKind) -> String {
    let name = t(&format!("tools.name_{}", kind.binary_name()));
    I18n::english().t_with("tools.use_global_named", &[("tool", &name)])
}

fn open_section(harness: &mut Harness<'static, Run>) {
    harness.get_by_label(&t("tools.heading")).click();
    harness.run();
}

#[test]
fn a_saved_connection_follows_the_global_settings_by_default() {
    let cfg = ConnectionConfig::new("Alpha", "mongodb://alpha.example.com");
    let mut harness = harness(editing(&cfg), ToolsState::default());
    open_section(&mut harness);
    for kind in ToolKind::ALL {
        harness.get_by_role_and_label(Role::CheckBox, &use_global_label(kind));
    }
}

#[test]
fn unticking_use_global_gives_the_connection_its_own_choice() {
    let cfg = ConnectionConfig::new("Alpha", "mongodb://alpha.example.com");
    let mut harness = harness(editing(&cfg), ToolsState::default());
    open_section(&mut harness);
    click_first_use_global(&mut harness);
    assert_eq!(
        harness.state().actions,
        vec![ConnectionAction::Tools(ToolsAction::ChooseOverride {
            connection: cfg.id,
            kind: ToolKind::Mongosh,
            choice: ToolChoice::Path,
        })]
    );
}

#[test]
fn ticking_use_global_again_drops_the_override() {
    let cfg = ConnectionConfig::new("Alpha", "mongodb://alpha.example.com");
    let tools = ToolsState::default().choose_override(
        cfg.id,
        ToolKind::Mongosh,
        ToolChoice::Executable("/mine".into()),
    );
    let (tools, _) = tools.take_queue();
    let mut harness = harness(editing(&cfg), tools);
    open_section(&mut harness);
    click_first_use_global(&mut harness);
    assert_eq!(
        harness.state().actions,
        vec![ConnectionAction::Tools(ToolsAction::UseGlobal {
            connection: cfg.id,
            kind: ToolKind::Mongosh,
        })]
    );
}

#[test]
fn an_unsaved_connection_is_told_to_save_first() {
    let mut state = ConnectionsState::new(Vec::new());
    state.editing = true;
    state.form = ConnectionForm::default();
    let mut harness = harness(state, ToolsState::default());
    open_section(&mut harness);
    harness.get_by_label(&t("tools.save_first"));
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, &use_global_label(ToolKind::Mongosh))
            .is_none()
    );
}

#[test]
fn a_tool_older_than_the_tested_server_is_called_out() {
    let cfg = ConnectionConfig::new("Alpha", "mongodb://alpha.example.com");
    let mut conn = editing(&cfg);
    conn.test_form = Some(conn.form.clone());
    conn.test = TestState::Done {
        request_id: 1,
        outcome: Ok(TestReport {
            server_version: Some("8.0.3".into()),
            tls: false,
            authenticated: false,
        }),
    };
    let scope = ProbeScope::Connection(cfg.id);
    let tools = ToolsState::default().choose_override(
        cfg.id,
        ToolKind::Mongosh,
        ToolChoice::Executable("/old".into()),
    );
    let (tools, queue) = tools.take_queue();
    let tools = tools.probed(
        scope,
        ToolKind::Mongosh,
        queue[0].request,
        Ok(ToolVersion::new(1, 10, 0)),
    );
    let mut harness = harness(conn, tools);
    open_section(&mut harness);
    harness.get_by_label(
        &I18n::english().t_with("tools.too_old", &[("version", "1.10.0"), ("server", "8.0")]),
    );
}
