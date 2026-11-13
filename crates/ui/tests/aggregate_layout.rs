//! The Aggregate tab's layout, measured from the accesskit bounds of the REAL app on an in-memory
//! store: in a small 800x500 window no control of the header, of the first stage card or of the
//! preview heading overlaps another or runs out of the window, and at the standard 1000x700 the
//! preview starts within a fixed budget from the top. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::theme;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::aggregate_shots::aggregate_tab;
use support::shots::{SHOT_SIZE, build_app};

/// The smallest window the tab is checked in.
const SMALL: (f32, f32) = (800.0, 500.0);
/// Where the preview heading may start at most at 1000x700 (points from the window's top).
/// Above it: connection + tabs (one row), the tab's header row, the stage list (at most half of
/// the room left) and the separator. It measures at about y = 410.
const PREVIEW_BUDGET: f32 = 440.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page(size: (f32, f32)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    aggregate_tab(app.state_mut());
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

fn tn(key: &str, n: usize) -> String {
    I18n::english().t_with(key, &[("n", &n.to_string())])
}

/// The rect of the node `label` with `role` right of `left` (the sidebar has a Refresh too).
fn rect_of(
    harness: &Harness<'static, CurlybraceApp>,
    role: Role,
    label: &str,
    left: f32,
) -> egui::Rect {
    harness
        .query_all_by_role_and_label(role, label)
        .map(|node| node.rect())
        .find(|rect| rect.left() >= left)
        .unwrap_or_else(|| panic!("{label} is not in the tab"))
}

/// The header, the first stage card and the preview heading, each with its rect.
fn controls(harness: &Harness<'static, CurlybraceApp>) -> Vec<(String, egui::Rect)> {
    let add = t("aggregate.add_stage");
    let left = harness
        .get_by_role_and_label(Role::ComboBox, &add)
        .rect()
        .left();
    let named = [
        (Role::ComboBox, add),
        (Role::RadioButton, t("aggregate.mode.stages")),
        (Role::RadioButton, t("aggregate.mode.text")),
        (Role::Button, t("aggregate.refresh")),
        (Role::Label, t("aggregate.status.shown")),
        (Role::CheckBox, tn("aggregate.stage.enable_n", 1)),
        (Role::Button, tn("aggregate.stage.select_n", 1)),
        (Role::Button, tn("aggregate.stage.move_up_n", 1)),
        (Role::Button, tn("aggregate.stage.move_down_n", 1)),
        (Role::Button, tn("aggregate.stage.remove_n", 1)),
        (Role::MultilineTextInput, tn("aggregate.stage.editor_n", 1)),
        (Role::RadioButton, t("view.tree")),
        (Role::Label, tn("aggregate.preview.heading", 2)),
    ];
    named
        .into_iter()
        .map(|(role, label)| {
            let rect = rect_of(harness, role, &label, left);
            (label, rect)
        })
        .collect()
}

#[test]
fn in_a_small_window_no_control_overlaps_or_leaves_the_window() {
    let page = page(SMALL);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SMALL.0, SMALL.1));
    let shown = controls(&page.harness);
    for (label, rect) in &shown {
        assert!(
            rect.width() > 1.0 && rect.height() > 1.0,
            "{label} is squashed: {rect:?}"
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
fn in_a_small_window_the_preview_comes_under_the_stage_list() {
    let page = page(SMALL);
    let harness = &page.harness;
    let heading = harness
        .get_by_role_and_label(Role::Label, &tn("aggregate.preview.heading", 2))
        .rect();
    let card = harness
        .get_by_role_and_label(Role::CheckBox, &tn("aggregate.stage.enable_n", 1))
        .rect();
    assert!(heading.top() > card.bottom(), "{heading:?} vs {card:?}");
    assert!(
        heading.bottom() < SMALL.1 - 20.0,
        "room for documents: {heading:?}"
    );
}

#[test]
fn the_preview_starts_within_the_budget() {
    let page = page(SHOT_SIZE);
    let heading = page
        .harness
        .get_by_role_and_label(Role::Label, &tn("aggregate.preview.heading", 2))
        .rect();
    assert!(
        heading.top() <= PREVIEW_BUDGET,
        "the preview starts at y = {} (budget {PREVIEW_BUDGET})",
        heading.top()
    );
}
