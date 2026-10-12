//! The sidebar only draws the rows inside its viewport, so the UI test helpers must scroll to a
//! row. This proves it without any database: an in-memory app with 200 databases sorting before
//! the target. Fast, offline, no GPU (runs in the default `cargo test`).
mod support;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mongostudio_ui::app::MongoStudioApp;
use mongostudio_ui::app::state::ConnStatus;
use mongostudio_ui::viewmodel::SidebarState;
use mongostudio_ui::views::sidebar::db_row_label;
use support::shots::build_app;
use support::sidebar::{click_sidebar_row, scroll_sidebar_until_shown};

const FILLERS: usize = 200;
const TARGET: &str = "msit_target";

fn browser_with_many_databases() -> (
    Harness<'static, MongoStudioApp>,
    tokio::runtime::Runtime,
    tempfile::TempDir,
) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    let mut names: Vec<String> = (0..FILLERS).map(|i| format!("a_filler_{i:03}")).collect();
    names.push(TARGET.to_string());
    let state = app.state_mut();
    state.status = ConnStatus::Connected;
    state.sidebar = SidebarState::default().with_databases(names);
    let harness = Harness::builder()
        .with_size(eframe::egui::vec2(1000.0, 700.0))
        .build_state(|ctx, app: &mut MongoStudioApp| app.run_frame(ctx), app);
    (harness, rt, dir)
}

#[test]
fn the_helper_scrolls_to_a_row_far_below_the_viewport_and_clicks_it() {
    let (mut harness, _rt, _dir) = browser_with_many_databases();
    let row = db_row_label(TARGET, false);
    harness.run();
    assert!(
        harness.query_all_by_label(&row).next().is_none(),
        "precondition: the row is outside the viewport, so not in the accessibility tree"
    );

    scroll_sidebar_until_shown(&mut harness, &row);
    assert!(harness.query_all_by_label(&row).next().is_some());

    click_sidebar_row(&mut harness, &row);
    let sidebar = &harness.state().state().sidebar;
    let target = sidebar.databases.iter().find(|d| d.name == TARGET).unwrap();
    assert!(target.expanded, "the click reached the target row");
    assert!(
        sidebar
            .databases
            .iter()
            .filter(|d| d.name != TARGET)
            .all(|d| !d.expanded),
        "no filler row was clicked by mistake"
    );
}

#[test]
#[should_panic(expected = "never appeared in the sidebar")]
fn a_row_that_does_not_exist_fails_clearly_instead_of_hanging() {
    let (mut harness, _rt, _dir) = browser_with_many_databases();
    scroll_sidebar_until_shown(&mut harness, &db_row_label("no_such_database", false));
}
