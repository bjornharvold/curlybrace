//! The Indexes tab in the accesskit tree: every index is listed, selecting one shows its detail,
//! a paused sampler states why while the list stays, and every control has a name. Fast, offline,
//! no GPU.
use curlybrace_core::clock::now_secs;
use curlybrace_core::history::{Bucket, ChartRange, Sample};
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{IndexSeries, SamplerPause, SamplerStatus};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::index_detail::anchored_now;
use curlybrace_ui::viewmodel::{IndexesAction, IndexesState};
use curlybrace_ui::views::indexes::show_indexes;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const HOUR: i64 = 3_600;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn spec(name: &str, field: &str) -> IndexSpec {
    mock_index_spec_named(name, &[(field, KeyKind::Asc)])
}

fn listed(spec: IndexSpec) -> CollectionIndex {
    CollectionIndex {
        spec,
        size_bytes: 4_096,
    }
}

fn bucket(start_secs: i64, uses: i64) -> Bucket {
    Bucket {
        start_secs,
        width_secs: 600,
        uses,
        restarts: 0,
        gap_secs: 0,
    }
}

/// A series of `spec` on `host` whose newest use was `hours_ago` hours before the chart's end.
fn used_series(spec: &IndexSpec, host: &str, hours_ago: i64) -> IndexSeries {
    let end = anchored_now(ChartRange::Day, now_secs());
    let last = end - hours_ago * HOUR - 600;
    IndexSeries {
        index_name: spec.name.clone(),
        definition_hash: spec.definition_hash(),
        host: host.into(),
        buckets: vec![bucket(end - 20 * HOUR, 0), bucket(last, 5)],
        latest: Some(Sample {
            at_secs: now_secs(),
            ops: 42,
            since_ms: 1,
        }),
    }
}

fn idle_series(spec: &IndexSpec, host: &str) -> IndexSeries {
    let end = anchored_now(ChartRange::Day, now_secs());
    IndexSeries {
        buckets: vec![bucket(end - 2 * HOUR, 0), bucket(end - HOUR, 0)],
        latest: Some(Sample {
            at_secs: now_secs(),
            ops: 0,
            since_ms: 1,
        }),
        ..used_series(spec, host, 0)
    }
}

fn state(indexes: Vec<CollectionIndex>, series: Vec<IndexSeries>) -> IndexesState {
    IndexesState {
        ns: Some(Namespace::new("shop", "orders")),
        indexes,
        series,
        ..IndexesState::default()
    }
}

type Frame = (IndexesState, Vec<IndexesAction>);

/// Draws the tab; a click on a row selects it the way the reducer would, other actions are
/// only recorded.
fn open(state: IndexesState, read_only: bool) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            move |ui, (state, seen): &mut Frame| {
                let action = show_indexes(ui, state, read_only, &I18n::english());
                if let Some(IndexesAction::Select(name)) = &action {
                    *state = state.clone().selecting(name.clone());
                }
                seen.extend(action);
            },
            (state, Vec::new()),
        );
    harness.run_steps(4);
    harness
}

fn count(harness: &Harness<'static, Frame>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn two_indexes() -> (IndexSpec, IndexSpec, IndexesState) {
    let a = spec("a_1", "a");
    let b = spec("b_1", "b");
    let state = state(
        vec![listed(a.clone()), listed(b.clone())],
        vec![used_series(&a, "node-1", 3), used_series(&b, "node-1", 5)],
    );
    (a, b, state)
}

#[test]
fn the_tab_lists_every_index_and_selecting_one_shows_its_detail() {
    let (_, _, state) = two_indexes();
    let mut harness = open(state, false);
    assert!(count(&harness, "a_1") >= 1 && count(&harness, "b_1") >= 1);
    // With nothing selected the first index is shown.
    harness.get_by_label("a: 1");
    assert_eq!(count(&harness, "b: 1"), 0);

    harness.get_by_label("b_1").click();
    harness.run_steps(4);
    assert_eq!(harness.state().1, vec![IndexesAction::Select("b_1".into())]);
    harness.get_by_label("b: 1");
    assert_eq!(count(&harness, "a: 1"), 0);
    // The detail names its node, says usage is per node, and states the facts of the index.
    harness.get_by_label(&I18n::english().t_with("indexes.node", &[("host", "node-1")]));
    harness.get_by_label(&t("indexes.node_note"));
    harness.get_by_label(&t("indexes.uses_since_restart"));
    harness.get_by_label(&I18n::english().t_with("indexes.ago_hours", &[("n", "5")]));
}

#[test]
fn a_hidden_never_used_index_is_marked_hidden_and_zero() {
    let mut hidden = spec("old_1", "old");
    hidden.options.hidden = true;
    let state = IndexesState {
        selected: Some("old_1".into()),
        ..state(
            vec![listed(hidden.clone())],
            vec![idle_series(&hidden, "n1")],
        )
    };
    let harness = open(state, false);
    // The marker is on the list row and on the detail heading.
    assert_eq!(count(&harness, &t("indexes.hidden")), 2);
    harness.get_by_label(&I18n::english().t_with("indexes.uses_count", &[("n", "0")]));
    harness.get_by_label(&t("indexes.never_since_restart"));
}

#[test]
fn a_paused_sampler_shows_its_reason_and_the_list_still_renders() {
    let reasons = [
        (SamplerPause::NotAuthorized, "indexes.pause_not_authorized"),
        (
            SamplerPause::ConnectionLost,
            "indexes.pause_connection_lost",
        ),
        (
            SamplerPause::HistoryUnavailable,
            "indexes.pause_history_unavailable",
        ),
    ];
    for (pause, key) in reasons {
        let (_, _, state) = two_indexes();
        let state = state.with_sampler(SamplerStatus::Paused(pause));
        let harness = open(state, false);
        harness.get_by_label(&t(key));
        assert!(count(&harness, "a_1") >= 1, "{key}: first index listed");
        assert!(count(&harness, "b_1") >= 1, "{key}: second index listed");
    }
    let (_, _, running) = two_indexes();
    let harness = open(running.with_sampler(SamplerStatus::Running), false);
    harness.get_by_label(&t("indexes.sampler_running"));
}

#[test]
fn every_control_has_an_accessible_label() {
    let (_, _, state) = two_indexes();
    let harness = open(state, false);
    for key in [
        "indexes.refresh",
        "indexes.copy_json",
        "indexes.range_day",
        "indexes.range_week",
        "indexes.range_month",
    ] {
        harness.get_by_role_and_label(Role::Button, &t(key));
    }
    // The chart is drawn, not read: its numbers are also in a text line.
    harness.get_by_label(&I18n::english().t_with(
        "indexes.summary",
        &[
            ("uses", "5"),
            ("range", &t("indexes.range_day")),
            ("restarts", "0"),
            ("hours", "0"),
        ],
    ));
}

#[test]
fn the_buttons_report_their_actions() {
    let (a, _, state) = two_indexes();
    let mut harness = open(state, false);
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.refresh"))
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.range_week"))
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.copy_json"))
        .click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().1,
        vec![
            IndexesAction::Refresh,
            IndexesAction::SetRange(ChartRange::Week),
            IndexesAction::CopyJson(a.to_json()),
        ]
    );
}

#[test]
fn a_series_of_the_same_name_and_another_definition_is_not_drawn() {
    let current = spec("a_1", "a");
    let older = spec("a_1", "other");
    let state = state(vec![listed(current)], vec![used_series(&older, "n1", 1)]);
    let harness = open(state, false);
    harness.get_by_label(&t("indexes.no_series"));
    assert_eq!(count(&harness, &t("indexes.uses_since_restart")), 0);
    // The list does not claim zero uses for an index it has no history of.
    harness.get_by_label(&t("indexes.no_data"));
    let zero = I18n::english().t_with("indexes.uses_count", &[("n", "0")]);
    assert_eq!(count(&harness, &zero), 0);
}

#[test]
fn the_read_only_flag_changes_nothing_in_this_view() {
    let labels = |read_only: bool| {
        let (_, _, state) = two_indexes();
        let harness = open(state, read_only);
        harness.query_all_by_role(Role::Button).count()
    };
    assert_eq!(labels(true), labels(false));
}

#[test]
fn empty_states_say_what_is_missing() {
    let loading = IndexesState {
        loading: true,
        ..IndexesState::default()
    };
    open(loading, false).get_by_label(&t("indexes.loading"));

    let none = state(Vec::new(), Vec::new());
    open(none, false).get_by_label(&t("indexes.no_indexes"));

    let a = spec("a_1", "a");
    let unsampled = state(vec![listed(a)], Vec::new());
    open(unsampled, false).get_by_label(&t("indexes.no_history"));
}
