//! The History button in the REAL app's find bar: it shares the Filter row with Find and Clear at
//! 1000x700 (the first document stays inside the same budget as without it), wraps without
//! overlapping at 800x500, and is not offered where there is no settings folder to keep queries
//! in (which is also what keeps every other harness's layout and screenshots as they were).
//! Measured from accesskit bounds. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::browse_page::{CLOSED_800, CLOSED_1000};
use support::history_shots::history_page;
use support::shots::{SHOT_SIZE, browsing, build_app};

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn browse(size: (f32, f32), setup: impl FnOnce(&mut AppState)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    setup(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
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

fn rect_of(page: &Page, label: &str) -> egui::Rect {
    page.harness
        .get_by_role_and_label(Role::Button, label)
        .rect()
}

fn window(size: (f32, f32)) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
}

#[test]
fn history_shares_the_filter_row_with_find_and_clear_at_1000() {
    let page = browse(SHOT_SIZE, history_page);
    let (find, clear, history) = (
        rect_of(&page, &t("findbar.run")),
        rect_of(&page, &t("findbar.clear")),
        rect_of(&page, &t("history.button")),
    );
    assert!(
        (history.center().y - find.center().y).abs() < 1.0,
        "one row"
    );
    assert!(
        history.min.x >= clear.max.x - 1.0,
        "after Clear: {history:?} {clear:?}"
    );
    assert!(window(SHOT_SIZE).contains_rect(history));
    let first = I18n::english().t_with("doc.select_row", &[("n", "1")]);
    let row = page.harness.get_by_label(&first).rect();
    assert!(
        row.top() <= CLOSED_1000,
        "the first document starts at y = {} (budget {CLOSED_1000})",
        row.top()
    );
}

#[test]
fn history_wraps_without_overlap_at_800() {
    let size = (800.0, 500.0);
    let page = browse(size, history_page);
    let labels = [
        t("findbar.run"),
        t("findbar.clear"),
        t("history.button"),
        t("findbar.prev"),
        t("findbar.next"),
    ];
    let rects: Vec<(String, egui::Rect)> = labels
        .iter()
        .map(|label| (label.clone(), rect_of(&page, label)))
        .collect();
    for (label, rect) in &rects {
        assert!(window(size).contains_rect(*rect), "{label}: {rect:?}");
    }
    for (i, (a, ra)) in rects.iter().enumerate() {
        for (b, rb) in &rects[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(
                !overlap.is_positive() || overlap.area() < 1.0,
                "{a} overlaps {b}"
            );
        }
    }
    let first = I18n::english().t_with("doc.select_row", &[("n", "1")]);
    let row = page.harness.get_by_label(&first).rect();
    assert!(
        row.top() <= CLOSED_800,
        "the first document starts at y = {} (budget {CLOSED_800})",
        row.top()
    );
}

#[test]
fn without_a_settings_folder_there_is_no_history_button() {
    let page = browse(
        SHOT_SIZE,
        browsing(curlybrace_ui::viewmodel::ViewMode::Table),
    );
    assert!(
        page.harness
            .query_by_role_and_label(Role::Button, &t("history.button"))
            .is_none()
    );
}

#[test]
fn a_paused_history_says_nothing_is_recorded_in_the_popup() {
    let mut page = browse(SHOT_SIZE, |state| {
        let path = support::history_shots::history_file();
        std::fs::write(&path, "{ nope").unwrap();
        browsing(curlybrace_ui::viewmodel::ViewMode::Table)(state);
        *state = std::mem::take(state).with_query_history_path(Ok(path));
    });
    page.harness
        .get_by_role_and_label(Role::Button, &t("history.button"))
        .click();
    page.harness.run_steps(4);
    let off = t("history.empty.recent_off");
    page.harness.get_by_label(&off);
}

#[test]
fn the_popup_opens_inside_the_window_at_both_sizes() {
    for size in [SHOT_SIZE, (800.0, 500.0)] {
        let mut page = browse(size, history_page);
        page.harness
            .get_by_role_and_label(Role::Button, &t("history.button"))
            .click();
        page.harness.run_steps(4);
        let (search_label, clear_label) = (t("history.search"), t("history.clear"));
        let search = page
            .harness
            .get_by_role_and_label(Role::TextInput, &search_label)
            .rect();
        assert!(window(size).contains_rect(search), "{size:?}");
        let clear = rect_of(&page, &clear_label);
        assert!(window(size).contains_rect(clear), "{size:?}");
    }
}
