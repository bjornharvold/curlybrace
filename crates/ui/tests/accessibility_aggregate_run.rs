//! The confirmed run of the Aggregate tab in the accesskit tree, rendered with the REAL English
//! locale at the standard 1000x700 and the small 800x500 window: the Run button has a role, a name
//! and a reason when off; the confirmation dialog and the run status fit the window with named
//! controls and nothing overlapping; the typed name field starts focused. Offline, no GPU.
use curlybrace_core::pipeline::RunTarget;
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::aggregate_run::{
    RunAvailability, RunDialog, RunDisabled, RunStatus, RunUi,
};
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::viewmodel::{AggregateState, ConnectionBadge};
use curlybrace_ui::views::aggregate::show_aggregate;
use curlybrace_ui::views::aggregate_run::RunContext;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

type Frame = (AggregateState, AggregateView, RunContext);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// A previewed `$match` then `$out` draft, so no automatic refresh is asked for.
fn write_draft(run: RunUi) -> AggregateState {
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$out")
        .edit(1, "{ \"$out\": \"city_totals\" }");
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("the write stage is not previewed, the rest is");
    draft.arrived(request, Ok(Vec::new())).with_run(run)
}

fn dialog(typed: &str) -> RunDialog {
    RunDialog {
        target: RunTarget {
            db: None,
            collection: "city_totals".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("crm", "customers"),
        typed: typed.into(),
        target_count: Some(42),
    }
}

fn open(draft: AggregateState, context: RunContext, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, (draft, view, context): &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                show_aggregate(ui, draft, view, &[], context, &I18n::english());
            },
            (draft, AggregateView::default(), context),
        );
    harness.run_steps(4);
    harness
}

fn ready(badge: ConnectionBadge) -> RunContext {
    RunContext {
        availability: RunAvailability::Ready,
        badge,
    }
}

fn window(size: (f32, f32)) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
}

fn label_of(harness: &Harness<'static, Frame>, role: Role, label: &str) -> egui::Rect {
    harness.get_by_role_and_label(role, label).rect()
}

fn assert_inside(size: (f32, f32), name: &str, rect: egui::Rect) {
    assert!(
        rect.width() > 1.0 && rect.height() > 1.0,
        "{name}: {rect:?}"
    );
    assert!(
        window(size).contains_rect(rect),
        "{name} leaves {size:?}: {rect:?}"
    );
}

#[test]
fn the_run_button_is_named_and_sits_in_the_toolbar_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let harness = open(
            write_draft(RunUi::default()),
            ready(Default::default()),
            size,
        );
        let run = label_of(&harness, Role::Button, &t("aggregate.run.button"));
        assert_inside(size, "Run", run);
        let refresh = label_of(&harness, Role::Button, &t("aggregate.refresh"));
        let overlap = run.intersect(refresh);
        assert!(!overlap.is_positive(), "Run overlaps Refresh at {size:?}");
    }
}

#[test]
fn run_is_missing_for_a_read_pipeline() {
    let draft = AggregateState::default().add_stage("$match");
    let harness = open(draft, RunContext::default(), STANDARD);
    let label = t("aggregate.run.button");
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &label)
            .is_none()
    );
}

#[test]
fn run_off_for_a_read_only_connection_says_why_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let context = RunContext {
            availability: RunAvailability::Disabled(RunDisabled::ReadOnly),
            badge: ConnectionBadge::default(),
        };
        let harness = open(write_draft(RunUi::default()), context, size);
        let label = t("aggregate.run.button");
        let run = harness.get_by_role_and_label(Role::Button, &label);
        assert!(run.accesskit_node().is_disabled());
        assert_eq!(
            run.accesskit_node().description(),
            Some(t("aggregate.run.disabled.read_only"))
        );
        assert_inside(size, "Run", run.rect());
    }
}

#[test]
fn the_dialog_fits_both_sizes_with_named_controls() {
    for size in [STANDARD, SMALL] {
        let run = RunUi::default().opened(dialog(""));
        let harness = open(write_draft(run), ready(Default::default()), size);
        let name = curlybrace_ui::I18n::english()
            .t_with("aggregate.run.type_accessible", &[("name", "city_totals")]);
        let field = label_of(&harness, Role::TextInput, &name);
        let cancel = label_of(&harness, Role::Button, &t("common.cancel"));
        let confirm = label_of(&harness, Role::Button, &t("aggregate.run.confirm"));
        let title = label_of(&harness, Role::Label, &t("aggregate.run.title"));
        let warning = label_of(&harness, Role::Label, &t("aggregate.run.irreversible"));
        for (n, rect) in [
            ("field", field),
            ("cancel", cancel),
            ("confirm", confirm),
            ("title", title),
            ("warning", warning),
        ] {
            assert_inside(size, n, rect);
        }
        assert!(!cancel.intersect(confirm).is_positive());
        let label = t("aggregate.run.confirm");
        let run_button = harness.get_by_role_and_label(Role::Button, &label);
        assert!(run_button.accesskit_node().is_disabled());
        let focused = harness
            .root()
            .children_recursive()
            .find(|n| n.accesskit_node().is_focused())
            .and_then(|n| n.accesskit_node().label());
        assert_eq!(focused, Some(name), "focus starts in the name field");
    }
}

#[test]
fn a_production_dialog_shows_its_badge_inside_the_window() {
    for size in [STANDARD, SMALL] {
        let badge = ConnectionBadge::new(
            Some(curlybrace_core::connection::ConnectionColor::Red),
            true,
        );
        let run = RunUi::default().opened(dialog("city_totals"));
        let harness = open(write_draft(run), ready(badge), size);
        let production = t(curlybrace_ui::viewmodel::connection_tag::PRODUCTION_BADGE_KEY);
        assert_inside(size, "badge", label_of(&harness, Role::Label, &production));
        let label = t("aggregate.run.confirm");
        let run_button = harness.get_by_role_and_label(Role::Button, &label);
        assert!(!run_button.accesskit_node().is_disabled());
    }
}

#[test]
fn every_status_fits_both_sizes_without_placeholders() {
    let target = Namespace::new("crm", "city_totals");
    let statuses = [
        RunStatus::Running,
        RunStatus::Done {
            target: target.clone(),
            count: Some(7),
        },
        RunStatus::Done {
            target,
            count: None,
        },
        RunStatus::Failed(curlybrace_core::error::CoreError::NotFound),
        RunStatus::Unknown,
    ];
    for size in [STANDARD, SMALL] {
        for status in &statuses {
            let run = RunUi {
                status: status.clone(),
                ..RunUi::default()
            };
            let harness = open(write_draft(run), ready(Default::default()), size);
            let labels: Vec<String> = harness
                .root()
                .children_recursive()
                .filter_map(|n| n.accesskit_node().label())
                .collect();
            assert!(labels.iter().all(|l| !l.contains('\u{27e6}')), "{labels:?}");
            let shown = harness
                .root()
                .children_recursive()
                .filter(|n| n.accesskit_node().role() == Role::Label)
                .filter_map(|n| n.accesskit_node().label().map(|l| (l, n.rect())))
                .filter(|(l, _)| {
                    l.starts_with("Run finished")
                        || l.starts_with("The run failed")
                        || l.starts_with("The outcome is unknown")
                        || l.starts_with("Running")
                });
            for (label, rect) in shown {
                assert_inside(size, &label, rect);
            }
        }
    }
}
