//! The open Options section's layout, measured from the accesskit bounds of the REAL app on an
//! in-memory store: one column in a small window and two columns in the standard one, no control
//! overlapping another or leaving the window, and the preview keeping its place because the
//! section scrolls with the stage list. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::shots::{SHOT_SIZE, build_app};

const SMALL: (f32, f32) = (800.0, 500.0);
/// Where the preview heading may start at most at 1000x700 (the same budget as without the
/// section: it scrolls inside the stage list's share).
const PREVIEW_BUDGET: f32 = 440.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page(size: (f32, f32)) -> Page {
    page_of(size, support::options_shots::options_draft())
}

/// A page with `draft` in the Aggregate tab.
fn page_of(size: (f32, f32), draft: AggregateState) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    support::counts_shots::tab_of(draft)(app.state_mut());
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

/// The rect of the node `label` with `role` right of `left` (the sidebar has controls too).
fn rect_of(harness: &Harness<'static, CurlybraceApp>, role: Role, label: &str) -> egui::Rect {
    harness
        .query_all_by_role_and_label(role, label)
        .map(|node| node.rect())
        .find(|rect| rect.left() >= 280.0)
        .unwrap_or_else(|| panic!("{label} is not in the tab"))
}

/// The controls at the top of the section: the first of each column.
fn top_controls(harness: &Harness<'static, CurlybraceApp>) -> Vec<(String, egui::Rect)> {
    let named = [
        (
            Role::Button,
            t("aggregate.options.toggle_count").replace("{n}", "4"),
        ),
        (Role::CheckBox, t("aggregate.options.collation.enabled")),
        (Role::TextInput, t("aggregate.options.collation.locale")),
        (Role::ComboBox, t("aggregate.options.collation.locale_pick")),
        (Role::ComboBox, t("aggregate.options.collation.strength")),
        (Role::RadioButton, t("view.tree")),
    ];
    named
        .into_iter()
        .map(|(role, label)| (label.clone(), rect_of(harness, role, &label)))
        .collect()
}

fn assert_apart_and_inside(shown: &[(String, egui::Rect)], window: egui::Rect) {
    for (label, rect) in shown {
        assert!(
            rect.width() > 1.0 && rect.height() > 1.0,
            "{label}: {rect:?}"
        );
        assert!(
            window.contains_rect(*rect),
            "{label} leaves the window: {rect:?}"
        );
    }
    for (i, (a, ra)) in shown.iter().enumerate() {
        for (b, rb) in &shown[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(
                !overlap.is_positive() || overlap.area() < 1.0,
                "{a} {ra:?} overlaps {b} {rb:?}"
            );
        }
    }
}

#[test]
fn at_800x500_the_open_section_is_one_column_and_nothing_overlaps_or_leaves_the_window() {
    let page = page(SMALL);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SMALL.0, SMALL.1));
    let shown = top_controls(&page.harness);
    assert_apart_and_inside(&shown, window);
    // One column: the hint's field is not beside the locale's, it is below the scroll or absent.
    let locale = shown[2].1;
    let hint_beside = page
        .harness
        .query_all_by_role_and_label(Role::ComboBox, &t("aggregate.options.hint.existing"))
        .map(|n| n.rect())
        .any(|r| r.left() > locale.right() && r.top() < locale.bottom());
    assert!(!hint_beside, "the section is one column in a small window");
}

#[test]
fn at_1000x700_the_open_section_is_two_columns_and_nothing_overlaps() {
    let page = page(SHOT_SIZE);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SHOT_SIZE.0, SHOT_SIZE.1));
    assert_apart_and_inside(&top_controls(&page.harness), window);
    let locale = rect_of(
        &page.harness,
        Role::TextInput,
        &t("aggregate.options.collation.locale"),
    );
    let hint = rect_of(
        &page.harness,
        Role::ComboBox,
        &t("aggregate.options.hint.existing"),
    );
    assert!(hint.left() > locale.right(), "beside: {locale:?} {hint:?}");
}

#[test]
fn an_empty_draft_with_the_section_open_keeps_the_result_tabs_in_the_window() {
    let page = page_of(SMALL, AggregateState::default().options_toggled());
    let preview = rect_of(
        &page.harness,
        Role::RadioButton,
        &t("aggregate.explain.tabs.preview"),
    );
    assert!(
        preview.bottom() <= SMALL.1,
        "the result tabs are pushed out of the window: {preview:?}"
    );
}

#[test]
fn the_open_section_scrolls_with_the_stage_list_and_the_preview_keeps_its_place() {
    let page = page(SHOT_SIZE);
    let heading = page
        .harness
        .get_by_role_and_label(
            Role::Label,
            &I18n::english().t_with("aggregate.preview.heading", &[("n", "2")]),
        )
        .rect();
    assert!(
        heading.top() <= PREVIEW_BUDGET,
        "the preview starts at y = {} (budget {PREVIEW_BUDGET})",
        heading.top()
    );
}
