//! The Aggregate tab with the Plan tab in view, in the accesskit tree of the REAL app rendered
//! with the real English locale at the standard 1000x700 and the small 800x500 window: the
//! Explain button, the three tabs and the measuring toggle have a role and a name, nothing leaves
//! the window or overlaps another control, and the plan's rows and hints are in the tree. Fast,
//! offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::theme;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::explain_shots::aggregate_plan_tab;
use support::shots::build_app;

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn page(size: (f32, f32)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    aggregate_plan_tab(app.state_mut());
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

/// The named controls of the explain section, each with its rect.
fn controls(harness: &Harness<'static, CurlybraceApp>) -> Vec<(String, egui::Rect)> {
    let add = t("aggregate.add_stage");
    let left = harness
        .get_by_role_and_label(Role::ComboBox, &add)
        .rect()
        .left();
    let named = [
        (Role::Button, t("aggregate.explain.button")),
        (Role::RadioButton, t("aggregate.explain.tabs.preview")),
        (Role::RadioButton, t("aggregate.explain.tabs.plan")),
        (Role::RadioButton, t("aggregate.explain.tabs.raw")),
        (Role::CheckBox, t("aggregate.explain.verbosity")),
    ];
    named
        .into_iter()
        .map(|(role, label)| {
            let rect = harness
                .query_all_by_role_and_label(role, &label)
                .map(|n| n.rect())
                .find(|r| r.left() >= left)
                .unwrap_or_else(|| panic!("{label} is not in the tab"));
            (label, rect)
        })
        .collect()
}

fn check_size(size: (f32, f32)) {
    let page = page(size);
    let harness = &page.harness;
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let shown = controls(harness);
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
    let name = t("aggregate.explain.tabs.plan");
    let plan = harness.get_by_role_and_label(Role::RadioButton, &name);
    assert_eq!(plan.accesskit_node().toggled(), Some(Toggled::True));
    harness.get_by_label(&t("aggregate.explain.input.collscan"));
    let note = I18n::english().t_with("aggregate.explain.cost_note", &[("seconds", "10")]);
    harness.get_by_label(&note);
}

#[test]
fn the_plan_tab_has_named_controls_that_fit_the_standard_window() {
    check_size(STANDARD);
}

#[test]
fn the_plan_tab_has_named_controls_that_fit_the_small_window() {
    check_size(SMALL);
}

#[test]
fn the_plan_is_in_the_tree_with_its_stage_figures_and_hints_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let page = page(size);
        let harness = &page.harness;
        assert!(harness.query_all_by_label_contains("8400 returned").count() >= 1);
        assert!(harness.query_all_by_label_contains("4 returned").count() >= 1);
        harness.get_by_label(&t("aggregate.explain.hints.heading"));
        harness.get_by_label(&t("aggregate.explain.hint.collection_scan"));
    }
}

#[test]
fn the_explain_button_is_a_named_enabled_button_for_a_valid_pipeline() {
    let page = page(STANDARD);
    let name = t("aggregate.explain.button");
    let button = page.harness.get_by_role_and_label(Role::Button, &name);
    assert!(!button.accesskit_node().is_disabled());
}
