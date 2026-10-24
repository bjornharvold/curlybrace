//! The Indexes tab in the accesskit tree: every index is listed, selecting one shows its detail,
//! a paused sampler states why while the list stays, and every control has a name. Fast, offline,
//! no GPU.
use curlybrace_core::history::{Bucket, ChartRange, Sample};
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{IndexSeries, SamplerPause, SamplerStatus};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use curlybrace_ui::I18n;
use curlybrace_ui::app::sampling_settings::SamplingSettings;
use curlybrace_ui::viewmodel::index_detail::anchored_now;
use curlybrace_ui::viewmodel::{IndexWrite, IndexesAction, IndexesState};
use curlybrace_ui::views::indexes::show_indexes;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const HOUR: i64 = 3_600;
/// The wall clock of every test: injected, never read.
const NOW: i64 = 1_800_000_000;

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

/// A series of `spec` on `host` whose newest use was `hours_ago` hours before the wall clock.
fn used_series(spec: &IndexSpec, host: &str, hours_ago: i64) -> IndexSeries {
    let end = anchored_now(ChartRange::Day, NOW);
    let last = NOW - hours_ago * HOUR - 600;
    IndexSeries {
        index_name: spec.name.clone(),
        definition_hash: spec.definition_hash(),
        host: host.into(),
        buckets: vec![bucket(end - 20 * HOUR, 0), bucket(last, 5)],
        latest: Some(Sample {
            at_secs: NOW,
            ops: 42,
            since_ms: 1,
        }),
    }
}

fn idle_series(spec: &IndexSpec, host: &str) -> IndexSeries {
    let end = anchored_now(ChartRange::Day, NOW);
    IndexSeries {
        buckets: vec![bucket(end - 2 * HOUR, 0), bucket(end - HOUR, 0)],
        latest: Some(Sample {
            at_secs: NOW,
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
    open_at(state, read_only, NOW)
}

fn open_at(state: IndexesState, read_only: bool, now: i64) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            move |ui, (state, seen): &mut Frame| {
                let action = show_indexes(
                    ui,
                    state,
                    read_only,
                    &I18n::english(),
                    now,
                    &SamplingSettings::default(),
                );
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
    let (_, _, failing) = two_indexes();
    let harness = open(failing.with_sampler(SamplerStatus::Failing), false);
    harness.get_by_label(&t("indexes.sampler_failing"));
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

/// The list and the detail of one index whose series is `series`, drawn.
fn open_with_series(series: IndexSeries) -> Harness<'static, Frame> {
    let a = spec("a_1", "a");
    open(state(vec![listed(a)], vec![series]), false)
}

fn assert_no_data_and_no_zero(harness: &Harness<'static, Frame>) {
    // A list row says "No data" for an index nobody observed, never "0 uses". (The chart
    // legend may carry the same words, so count rather than get.)
    assert!(count(harness, &t("indexes.no_data")) >= 1);
    let zero = I18n::english().t_with("indexes.uses_count", &[("n", "0")]);
    assert_eq!(count(harness, &zero), 0);
    // The detail line does not claim zero uses either.
    let summary = I18n::english().t_with(
        "indexes.summary_unobserved",
        &[("range", &t("indexes.range_day")), ("hours", "0")],
    );
    harness.get_by_label(&summary);
}

#[test]
fn a_series_with_only_gap_buckets_in_the_window_shows_no_data_not_zero() {
    let a = spec("a_1", "a");
    let mut series = idle_series(&a, "n1");
    for b in &mut series.buckets {
        b.gap_secs = b.width_secs;
    }
    assert_no_data_and_no_zero(&open_with_series(series));
}

#[test]
fn a_series_with_no_buckets_in_the_window_shows_no_data_not_zero() {
    let a = spec("a_1", "a");
    let mut series = idle_series(&a, "n1");
    // One bucket from a week ago: outside the day that is shown.
    let old_start = anchored_now(ChartRange::Day, NOW) - 7 * 24 * HOUR;
    series.buckets = vec![bucket(old_start, 3)];
    assert_no_data_and_no_zero(&open_with_series(series));
}

#[test]
fn the_create_button_is_disabled_on_a_read_only_connection_and_says_why() {
    let (_, _, state) = two_indexes();
    let harness = open(state, true);
    let create = t("indexes.create");
    assert!(
        harness
            .get_by_role_and_label(Role::Button, &create)
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label(&t("connection.read_only_reason"));
}

#[test]
fn the_create_button_is_enabled_on_a_writable_connection() {
    let (_, _, state) = two_indexes();
    let mut harness = open(state, false);
    let create = t("indexes.create");
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, &create)
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(count(&harness, &t("connection.read_only_reason")), 0);
    harness.get_by_role_and_label(Role::Button, &create).click();
    harness.run_steps(2);
    assert_eq!(harness.state().1, vec![IndexesAction::OpenBuilder]);
}

#[test]
fn the_create_button_waits_while_a_create_is_in_flight() {
    let (_, _, state) = two_indexes();
    let state = IndexesState {
        writing: Some(IndexWrite::Create {
            name: "a_1".into(),
            replaces: None,
        }),
        ..state
    };
    let harness = open(state, false);
    assert!(
        harness
            .get_by_role_and_label(Role::Button, &t("indexes.create"))
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label(&t("indexes.create_busy"));
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

/// Every label and button text on screen, in tree order.
fn texts(harness: &Harness<'static, Frame>) -> Vec<String> {
    [Role::Label, Role::Button]
        .into_iter()
        .flat_map(|role| harness.query_all_by_role(role))
        .filter_map(|n| {
            n.accesskit_node()
                .label()
                .or_else(|| n.accesskit_node().value())
        })
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn the_view_output_depends_only_on_the_injected_now() {
    let (_, _, state) = two_indexes();
    let first = texts(&open_at(state.clone(), false, NOW));
    let again = texts(&open_at(state.clone(), false, NOW));
    assert_eq!(first, again, "same now, same output");

    // Three hours later the same data is three hours older (the grid step is ten minutes).
    let later = texts(&open_at(state, false, NOW + 3 * HOUR));
    let ago = |h: &str| I18n::english().t_with("indexes.ago_hours", &[("n", h)]);
    assert!(first.contains(&ago("3")) && !first.contains(&ago("6")));
    assert!(later.contains(&ago("6")) && !later.contains(&ago("3")));
}

/// "Last used" and "Tracked since" are ages against the real clock, not against the chart's
/// bucket-anchored right edge, so the range selector cannot change them.
#[test]
fn last_used_and_tracked_since_do_not_depend_on_the_range() {
    let a = spec("a_1", "a");
    let mut series = used_series(&a, "node-1", 0);
    series.buckets = vec![bucket(NOW - 20 * HOUR, 0), bucket(NOW - 3 * HOUR - 600, 5)];
    let ago = |h: &str| I18n::english().t_with("indexes.ago_hours", &[("n", h)]);
    for range in [ChartRange::Day, ChartRange::Week, ChartRange::Month] {
        let state = IndexesState {
            range,
            ..state(vec![listed(a.clone())], vec![series.clone()])
        };
        let shown = texts(&open(state, false));
        assert!(shown.contains(&ago("3")), "{range:?}: last used in {shown:?}");
        assert!(
            shown.contains(&ago("20")),
            "{range:?}: tracked since in {shown:?}"
        );
        assert!(!shown.contains(&t("indexes.ago_now")), "{range:?}");
    }
}
