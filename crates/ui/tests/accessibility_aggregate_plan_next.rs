//! The Plan tab's follow-ups in the accesskit tree, rendered with the REAL English locale: Try an
//! index, Clear hint, Copy command and Create index... have a role and a name, Tab walks them in
//! reading order, a read-only connection's Create index... reads its reason, and at the small
//! window's width nothing overlaps or leaves the window. Fast, offline, no GPU.
mod support;

use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::views::aggregate::show_aggregate;
use curlybrace_ui::views::aggregate_run::RunContext;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::explain_shots::plan_next_draft;

struct Frame {
    draft: AggregateState,
    view: AggregateView,
    run: RunContext,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn run(read_only: bool) -> RunContext {
    RunContext {
        read_only,
        ns: Some(Namespace::new("crm", "customers")),
        ..RunContext::default()
    }
}

/// The Aggregate tab showing the plan of `plan_next_draft` in a window of `size`, tall enough for
/// the whole tab (the follow-ups sit under the plan, inside its scroll area).
fn open(size: (f32, f32), run: RunContext) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                show_aggregate(
                    ui,
                    &frame.draft,
                    &mut frame.view,
                    &[],
                    &frame.run,
                    &I18n::english(),
                );
            },
            Frame {
                draft: plan_next_draft(),
                view: AggregateView::default(),
                run,
            },
        );
    harness.run_steps(4);
    harness
}

fn controls() -> [(Role, String); 4] {
    [
        (Role::ComboBox, t("aggregate.options.explain.try_index")),
        (Role::Button, t("aggregate.options.explain.clear_hint")),
        (
            Role::Button,
            t("aggregate.options.explain.copy_create_index"),
        ),
        (Role::Button, t("indexes.create_suggested")),
    ]
}

#[test]
fn every_follow_up_control_has_a_role_and_a_name() {
    let harness = open((1000.0, 1400.0), run(false));
    for (role, label) in controls() {
        harness.get_by_role_and_label(role, &label);
    }
    harness.get_by_label(&t("aggregate.options.explain.suggested"));
}

#[test]
fn tab_walks_try_clear_copy_create_in_order() {
    let mut harness = open((1000.0, 1400.0), run(false));
    let wanted: Vec<String> = controls().into_iter().map(|(_, label)| label).collect();
    harness
        .get_by_role_and_label(Role::ComboBox, &wanted[0])
        .focus();
    harness.run_steps(2);
    let mut seen = vec![wanted[0].clone()];
    for _ in 0..8 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        let label = harness
            .root()
            .children_recursive()
            .find(|n| n.accesskit_node().is_focused())
            .and_then(|n| n.accesskit_node().label());
        if let Some(label) = label.filter(|l| wanted.contains(l)) {
            if seen.last() != Some(&label) {
                seen.push(label);
            }
        }
    }
    assert_eq!(seen[..wanted.len()], wanted[..], "reading order");
}

#[test]
fn the_disabled_create_index_reads_its_reason() {
    let harness = open((1000.0, 1400.0), run(true));
    let label = t("indexes.create_suggested");
    let create = harness.get_by_role_and_label(Role::Button, &label);
    assert!(create.accesskit_node().is_disabled());
    assert_eq!(
        create.accesskit_node().description(),
        Some(t("indexes.create_suggested_read_only"))
    );
    // Copy command stays on: the command can be created elsewhere.
    let copy_label = t("aggregate.options.explain.copy_create_index");
    let copy = harness.get_by_role_and_label(Role::Button, &copy_label);
    assert!(!copy.accesskit_node().is_disabled());
}

#[test]
fn at_the_small_windows_width_the_follow_ups_fit_without_overlap() {
    // 800 wide as the small window; the height shows the whole tab (the plan scrolls in 500).
    let size = (800.0, 1400.0);
    let harness = open(size, run(false));
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let shown: Vec<(String, egui::Rect)> = controls()
        .into_iter()
        .map(|(role, label)| {
            let rect = harness.get_by_role_and_label(role, &label).rect();
            (label, rect)
        })
        .collect();
    for (label, rect) in &shown {
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
