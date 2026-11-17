//! The Query options section on the Browse page: with it closed (and More closed) the first
//! document row stays within the page's budget, the open section adds at most its share of the
//! window, its controls wrap without overlapping and read top to bottom, and the status of a
//! shaped page costs at most two rows. Measured from the accesskit bounds of the REAL app on an
//! in-memory store. Fast, offline, no GPU.
//!
//! What these do not claim: that the first document row is on screen while the section is open in
//! a small window. With More open alone it already is not at 800x500 (the measurements are in
//! `the_first_row_is_off_a_500_point_window_with_more_open_before_any_option`).
mod support;

use curlybrace_core::store::Query;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::FindBarState;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;
use curlybrace_ui::views::findbar_options::{OPTIONS_MAX_HEIGHT_SHARE, OPTIONS_MIN_HEIGHT};
use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::find_options_shots::find_options_page;

/// Where the first document row may start at most with the section closed (points from the top),
/// the budget of `browse_layout.rs` with More closed.
const FIRST_ROW_BUDGET: f32 = 300.0;
/// What the frame of the section, its margins and the lines around it add to its scrolling part.
const AROUND: f32 = 64.0;
/// One row of the page's header, rounded up.
const ROW: f32 = 36.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn browse(size: (f32, f32), adjust: impl FnOnce(&mut AppState)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = support::shots::build_app(vec![], None);
    find_options_page(app.state_mut());
    adjust(app.state_mut());
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

fn first_row_top(page: &Page) -> f32 {
    let label = I18n::english().t_with("doc.select_row", &[("n", "1")]);
    page.harness.get_by_label(&label).rect().top()
}

/// The collation ticked with its locale, so every flag shows (the tallest the section gets).
fn everything_shown(state: &mut AppState) {
    state.find = state
        .find
        .clone()
        .options_edited(OptionsEdit::Locale("en".into()));
}

fn section_closed(state: &mut AppState) {
    state.find.options_open = Some(false);
}

/// More closed, the form holding options that parse (no problem line), and a page no option
/// shaped.
fn more_closed_nothing_shaped(state: &mut AppState) {
    state.find = state
        .find
        .clone()
        .options_edited(OptionsEdit::Comment(String::new()));
    state.find.options_open = Some(false);
    state.find.more = Some(false);
    state.issued_query = Query::default();
    state.page_query = Query::default();
}

/// The bounds of the controls the person can see in the bar and its section: the ones above the
/// switches row (what a scrolling section holds out of view is not visible).
fn visible_controls(page: &Page) -> Vec<(String, Rect)> {
    let limit = page
        .harness
        .get_by_label(&t("query.verdict.automatic"))
        .rect()
        .top();
    [
        Role::Button,
        Role::TextInput,
        Role::ComboBox,
        Role::CheckBox,
    ]
    .into_iter()
    .flat_map(|role| page.harness.query_all_by_role(role))
    .filter_map(|node| {
        let label = node.accesskit_node().label()?;
        let rect = node.rect();
        (rect.top() > 20.0 && rect.bottom() <= limit && rect.width() > 1.0).then_some((label, rect))
    })
    .collect()
}

fn assert_no_overlap(page: &Page, what: &str) {
    let all = visible_controls(page);
    assert!(all.len() > 12, "{what}: only {} controls seen", all.len());
    for (i, (a_label, a)) in all.iter().enumerate() {
        for (b_label, b) in all.iter().skip(i + 1) {
            let shared = a.intersect(*b);
            assert!(
                shared.width() <= 1.0 || shared.height() <= 1.0,
                "{what}: {a_label:?} {a:?} overlaps {b_label:?} {b:?}"
            );
        }
    }
}

#[test]
fn with_the_section_closed_the_first_row_stays_within_the_budget() {
    // Nothing set: the page of `browse_layout.rs`, within its budget.
    let untouched = browse((1000.0, 700.0), |state| {
        state.find = FindBarState::default();
        state.issued_query = Query::default();
        state.page_query = Query::default();
    });
    let top = first_row_top(&untouched);
    assert!(
        top <= FIRST_ROW_BUDGET,
        "the first document starts at y = {top} (budget {FIRST_ROW_BUDGET})"
    );
    // Options set, More and the section closed: the toggle's "(N set)" label is what the Sort
    // box already has, so it costs what a set Sort costs and no more.
    let options = browse((1000.0, 700.0), more_closed_nothing_shaped);
    let sorted = browse((1000.0, 700.0), |state| {
        state.find = FindBarState {
            text: curlybrace_core::query::QueryText {
                sort: "{ \"a\": 1 }".into(),
                ..Default::default()
            },
            more: Some(false),
            ..FindBarState::default()
        };
        state.issued_query = Query::default();
        state.page_query = Query::default();
    });
    let (with_options, with_sort) = (first_row_top(&options), first_row_top(&sorted));
    assert!(
        (with_options - with_sort).abs() <= 1.0,
        "options {with_options}, a set Sort {with_sort}"
    );
}

#[test]
fn the_status_of_a_shaped_page_costs_at_most_two_rows() {
    let plain = browse((1000.0, 700.0), more_closed_nothing_shaped);
    let shaped = browse((1000.0, 700.0), |state| {
        section_closed(state);
        state.find.more = Some(false);
    });
    let extra = first_row_top(&shaped) - first_row_top(&plain);
    assert!(
        (0.0..=4.0 * ROW).contains(&extra),
        "the five status lines of a fully shaped page add {extra} points"
    );
}

#[test]
fn at_800x500_the_open_section_adds_at_most_its_share_and_nothing_overlaps() {
    let size = (800.0, 500.0);
    let closed = browse(size, section_closed);
    let open = browse(size, everything_shown);
    let added = first_row_top(&open) - first_row_top(&closed);
    let share = (size.1 * OPTIONS_MAX_HEIGHT_SHARE).max(OPTIONS_MIN_HEIGHT);
    assert!(
        (0.0..=share + AROUND).contains(&added),
        "the section adds {added} points (share {share})"
    );
    assert_no_overlap(&open, "800x500");
    // The bar reads top to bottom: Filter, the More row, then the section.
    let filter = open.harness.get_by_label(&t("findbar.filter")).rect();
    let limit = open.harness.get_by_label(&t("findbar.limit")).rect();
    let collation = open
        .harness
        .get_by_role_and_label(Role::CheckBox, &t("aggregate.options.collation.enabled"))
        .rect();
    assert!(filter.bottom() <= limit.top() + 2.0, "{filter:?} {limit:?}");
    assert!(
        limit.bottom() <= collation.top() + 2.0,
        "{limit:?} {collation:?}"
    );
}

#[test]
fn at_1000x700_the_open_section_wraps_without_overlap() {
    let page = browse((1000.0, 700.0), everything_shown);
    assert_no_overlap(&page, "1000x700");
}

#[test]
fn the_section_scrolls_and_never_exceeds_its_share() {
    for size in [(800.0, 500.0), (1000.0, 700.0), (1000.0, 1000.0)] {
        let closed = browse(size, section_closed);
        let open = browse(size, everything_shown);
        let added = first_row_top(&open) - first_row_top(&closed);
        let share = (size.1 * OPTIONS_MAX_HEIGHT_SHARE).max(OPTIONS_MIN_HEIGHT);
        assert!(
            added > 0.0 && added <= share + AROUND,
            "{size:?}: the open section adds {added} points (share {share})"
        );
    }
}

#[test]
fn the_first_row_is_off_a_500_point_window_with_more_open_before_any_option() {
    // A finding, kept as a measurement: with More open and no option anywhere the first document
    // row is already below an 800x500 window, so no height share can keep it on screen there.
    let page = browse((800.0, 500.0), |state| {
        state.find = FindBarState {
            more: Some(true),
            ..FindBarState::default()
        };
        state.issued_query = Query::default();
        state.page_query = Query::default();
    });
    assert!(first_row_top(&page) > 500.0);
}
