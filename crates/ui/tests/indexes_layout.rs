//! The Indexes tab's header stays dense: at the standard 1000x700 window the index list starts
//! within a fixed budget from the top, and the Indexes | Queries toggle shares its row with the
//! toolbar (range, Refresh, Create index) in both views. A regression guard for the layout,
//! measured from the accesskit bounds of the REAL app on an in-memory store. Fast, offline, no
//! GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::indexes_shots::{indexes_tab, queries_view};
use support::shots::{SHOT_SIZE, build_app};

/// Where the index list's heading may start at most (points from the window's top). Above it:
/// connection + tabs (one row), the toggle with the toolbar and the Sampling disclosure (one
/// row), the sampler's status (a text line: it does not fit beside them at this width) and the
/// separator. Before the toolbar row was folded the heading started at y = 176.
const LIST_BUDGET: f32 = 120.0;
/// Everything right of this is in the tab, not the sidebar (points).
const TAB_LEFT: f32 = 300.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page(prepare: fn(&mut AppState)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    prepare(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(SHOT_SIZE.0, SHOT_SIZE.1))
        .with_pixels_per_point(1.0)
        .build_state(
            move |ctx, app: &mut CurlybraceApp| {
                if !installed {
                    theme::install(ctx);
                    ctx.set_theme(egui::Theme::Light);
                    installed = true;
                }
                app.run_frame(ctx)
            },
            app,
        );
    harness.run_steps(6);
    drop(enter);
    Page {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// The node named `key` in the tab (the sidebar has a Refresh too, left of the tab).
fn in_tab(harness: &Harness<'static, CurlybraceApp>, key: &str) -> egui::Rect {
    harness
        .query_all_by_label(&t(key))
        .map(|node| node.rect())
        .filter(|rect| rect.left() > TAB_LEFT)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("{key} is not in the tab"))
}

fn same_row(harness: &Harness<'static, CurlybraceApp>, left: &str, right: &str) {
    let (a, b) = (in_tab(harness, left), in_tab(harness, right));
    assert!(
        a.right() < b.left(),
        "{left} {a:?} comes before {right} {b:?}"
    );
    assert!(
        (a.center().y - b.center().y).abs() < 2.0,
        "{left} {a:?} and {right} {b:?} are not on one row"
    );
}

#[test]
fn the_index_list_starts_within_the_budget() {
    let page = page(indexes_tab);
    let heading = page
        .harness
        .get_by_role_and_label(Role::Label, &t("indexes.list_heading"))
        .rect();
    assert!(
        heading.top() <= LIST_BUDGET,
        "the index list starts at y = {} (budget {LIST_BUDGET})",
        heading.top()
    );
}

#[test]
fn the_toggle_and_the_toolbar_share_one_row() {
    let page = page(indexes_tab);
    let harness = &page.harness;
    same_row(harness, "indexes.queries.view_queries", "indexes.range_day");
    same_row(harness, "indexes.range_day", "indexes.refresh");
    same_row(harness, "indexes.refresh", "indexes.create");
}

#[test]
fn in_the_queries_view_the_toggle_shares_the_row_with_range_and_refresh() {
    let page = page(queries_view);
    let harness = &page.harness;
    same_row(harness, "indexes.queries.view_queries", "indexes.range_day");
    same_row(harness, "indexes.range_day", "indexes.refresh");
}
