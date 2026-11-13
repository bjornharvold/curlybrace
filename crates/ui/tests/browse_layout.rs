//! The Browse page's header stays dense: at the standard 1000x700 window the first document
//! starts within a fixed budget from the top, and the connection header shares its row with the
//! Documents | Indexes tabs. A regression guard for the layout, measured from the accesskit
//! bounds of the REAL app on an in-memory store. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::{CollectionTab, ViewMode};
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::shots::{SHOT_SIZE, browsing, build_app};

/// Where the first document row may start at most (points from the window's top). The header
/// above it is: connection + tabs (one row), the Filter row, the switches with the verdict's
/// reserved row (it wraps at this width), the toolbar (two rows at this width) and the table's
/// column header. Before the dense layout the first row started at about y = 633.
const FIRST_ROW_BUDGET: f32 = 300.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn browse(mode: ViewMode) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    browsing(mode)(app.state_mut());
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

#[test]
fn the_first_document_row_starts_within_the_budget() {
    let page = browse(ViewMode::Table);
    let first = I18n::english().t_with("doc.select_row", &[("n", "1")]);
    let row = page.harness.get_by_label(&first).rect();
    assert!(
        row.top() <= FIRST_ROW_BUDGET,
        "the first document starts at y = {} (budget {FIRST_ROW_BUDGET})",
        row.top()
    );
}

#[test]
fn the_connection_header_and_the_tabs_share_one_row() {
    let page = browse(ViewMode::Table);
    let harness = &page.harness;
    let documents = harness
        .get_by_label(&t(CollectionTab::Documents.label_key()))
        .rect();
    let connection = harness
        .get_by_label(&I18n::english().t_with("doc.connection", &[("name", "Production")]))
        .rect();
    assert!(connection.right() < documents.left(), "the tabs follow it");
    assert!(
        (connection.center().y - documents.center().y).abs() < 2.0,
        "{connection:?} and {documents:?} are not on one row"
    );
    let filter = harness.get_by_label(&t("findbar.filter")).rect();
    assert!(filter.top() > documents.bottom(), "the Filter comes next");
}

#[test]
fn the_view_switcher_leads_the_documents_toolbar_row() {
    let page = browse(ViewMode::Table);
    let harness = &page.harness;
    let table = harness.get_by_label(&t("view.table")).rect();
    let insert = harness.get_by_label(&t("doc.insert")).rect();
    assert!(table.right() < insert.left());
    assert!(
        (table.center().y - insert.center().y).abs() < 2.0,
        "{table:?} and {insert:?} are not on one row"
    );
}
