//! The Queries view in the accesskit tree, rendered with the REAL English locale: every state
//! shows resolved text (never a missing-key marker, never server text), verdicts are icon plus
//! words, every control is named and reports the right action, and the status row keeps its
//! height across states. Fast, offline.
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ChartRange, ShapeBucket, ShapeSource, ShapeSummary};
use curlybrace_core::runtime::{ProfilerStatus, SamplerPause};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{Namespace, PROFILER_MONGOS_MESSAGE, ProfilingState, Query};
use curlybrace_ui::I18n;
use curlybrace_ui::app::sampling_settings::SamplingSettings;
use curlybrace_ui::viewmodel::{
    IndexesAction, IndexesState, IndexesView, ProfilingView, QueriesAction, QueriesState,
};
use curlybrace_ui::views::indexes::show_indexes;
use curlybrace_ui::views::queries::show_queries;
use eframe::egui;
use eframe::egui::OutputCommand;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const NOW: i64 = 1_800_000_000;
const SECRET: &str = "secret-server-detail";

struct Frame {
    state: QueriesState,
    read_only: bool,
    seen: Vec<QueriesAction>,
    copied: Vec<String>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn shape_json(field: &str) -> String {
    from_query(&Query {
        filter: bson::doc! { field: 1 },
        ..Query::default()
    })
    .json()
}

fn summary(field: &str, worst: VerdictKind, misses: u64) -> ShapeSummary {
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: shape_json(field),
        source: ShapeSource::App,
        executions: 42,
        misses,
        total_millis: 900,
        max_millis: 350,
        worst,
        latest: worst,
        buckets: vec![ShapeBucket {
            start_secs: NOW - 1_200,
            width_secs: 600,
            executions: 42,
            misses,
            gap_secs: 0,
        }],
    }
}

fn known(level: i32) -> QueriesState {
    QueriesState::default().with_profiling(ProfilingView::Known(ProfilingState {
        level,
        slow_ms: 100,
    }))
}

fn open(state: QueriesState, read_only: bool) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let ns = Namespace::new("shop", "orders");
                let action = show_queries(
                    ui,
                    &frame.state,
                    &ns,
                    frame.read_only,
                    &I18n::english(),
                    NOW,
                );
                frame.seen.extend(action);
                let copies: Vec<String> = ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| match c {
                            OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect()
                });
                frame.copied.extend(copies);
            },
            Frame {
                state,
                read_only,
                seen: Vec::new(),
                copied: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn texts(harness: &Harness<'static, Frame>) -> String {
    harness
        .root()
        .children_recursive()
        .flat_map(|node| {
            let n = node.accesskit_node();
            [n.label(), n.value()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_shows(harness: &Harness<'static, Frame>, expected: &[String]) {
    let all = texts(harness);
    assert!(!all.contains('\u{27e6}'), "missing key marker in:\n{all}");
    assert!(!all.contains(SECRET), "server text leaked in:\n{all}");
    for want in expected {
        assert!(all.contains(want.as_str()), "`{want}` not in:\n{all}");
    }
}

fn button<'a>(harness: &'a Harness<'static, Frame>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

fn click(harness: &mut Harness<'static, Frame>, label: &str) {
    button(harness, label).click();
    harness.run_steps(3);
}

#[test]
fn profiler_off_explains_and_offers_to_enable() {
    let mut harness = open(known(0), false);
    assert_shows(
        &harness,
        &[
            t("indexes.queries.profiler_off"),
            t("indexes.queries.empty_off"),
            t("indexes.queries.enable"),
        ],
    );
    click(&mut harness, &t("indexes.queries.enable"));
    assert_eq!(harness.state().seen, vec![QueriesAction::AskEnable]);
}

#[test]
fn profiler_on_offers_to_turn_off_and_says_the_level() {
    let mut harness = open(known(2), false);
    let level = I18n::english().t_with("indexes.queries.profiler_level", &[("level", "2")]);
    assert_shows(&harness, &[level, t("indexes.queries.empty_on")]);
    click(&mut harness, &t("indexes.queries.turn_off"));
    assert_eq!(harness.state().seen, vec![QueriesAction::AskDisable]);
}

#[test]
fn rows_show_icon_and_words_and_each_text_has_a_copy_button() {
    let state = known(2).with_shapes(vec![
        summary("status", VerdictKind::CollectionScan, 12),
        summary("sku", VerdictKind::IndexUsed, 0),
    ]);
    let mut harness = open(state, false);
    assert_shows(
        &harness,
        &[
            "\u{26a0} Collection scan: no index used".to_string(),
            "\u{2714} Uses an index".to_string(),
            shape_json("status"),
            t("indexes.queries.source_app"),
            "42 runs".to_string(),
            "12 misses".to_string(),
            "slowest 350 ms".to_string(),
            t("query.verdict.suggestion"),
            "createIndex".to_string(),
        ],
    );
    // Each row's buttons are told apart by position.
    let i18n = I18n::english();
    for position in ["1", "2"] {
        let label = i18n.t_with(
            "indexes.queries.copy_shape",
            &[("position", position), ("total", "2")],
        );
        assert_eq!(harness.query_all_by_label(&label).count(), 1, "{label}");
    }
    // Only the miss (listed first) has a suggestion.
    let suggestion = i18n.t_with(
        "indexes.queries.copy_suggestion",
        &[("position", "1"), ("total", "2")],
    );
    click(&mut harness, &suggestion);
    let copied = harness.state().copied.clone();
    assert_eq!(copied.len(), 1);
    assert!(copied[0].contains("createIndex"), "{copied:?}");
    let first = i18n.t_with(
        "indexes.queries.copy_shape",
        &[("position", "1"), ("total", "2")],
    );
    click(&mut harness, &first);
    assert!(harness.state().copied.contains(&shape_json("status")));
}

#[test]
fn n_rows_have_n_distinct_copy_labels() {
    let shapes = ["a", "b", "c", "d"]
        .map(|f| summary(f, VerdictKind::CollectionScan, 3))
        .to_vec();
    let harness = open(known(2).with_shapes(shapes), false);
    let labels: Vec<String> = harness
        .root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Button)
        .filter_map(|n| n.accesskit_node().label().map(|l| l.to_string()))
        .filter(|l| l.starts_with("Copy "))
        .collect();
    let mut unique = labels.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(labels.len(), 8, "{labels:?}");
    assert_eq!(unique.len(), 8, "{labels:?}");
}

#[test]
fn counts_use_the_singular_for_exactly_one() {
    let mut once = summary("status", VerdictKind::CollectionScan, 1);
    once.executions = 1;
    let harness = open(known(2).with_shapes(vec![once]), false);
    assert_shows(&harness, &["1 run".to_string(), "1 miss".to_string()]);
    assert!(!texts(&harness).contains("1 runs") && !texts(&harness).contains("1 misses"));
    let mut none = summary("sku", VerdictKind::IndexUsed, 0);
    none.executions = 0;
    let harness = open(known(2).with_shapes(vec![none]), false);
    assert_shows(&harness, &["0 runs".to_string(), "0 misses".to_string()]);
}

#[test]
fn neutral_kinds_read_as_app_queries_without_the_warning_tone() {
    let state = known(2).with_shapes(vec![
        summary("a", VerdictKind::NoFilter, 0),
        summary("b", VerdictKind::EmptyCollection, 0),
    ]);
    let harness = open(state, false);
    assert_shows(
        &harness,
        &[
            format!("\u{2139} {}", t("indexes.queries.title_no_filter")),
            format!("\u{2139} {}", t("indexes.queries.title_empty")),
        ],
    );
    assert!(!texts(&harness).contains('\u{26a0}'));
}

#[test]
fn level_one_offers_both_buttons_and_explains_the_threshold() {
    let mut harness = open(known(1), false);
    let note = I18n::english().t_with("indexes.queries.level_one_note", &[("slow_ms", "100")]);
    assert_shows(&harness, std::slice::from_ref(&note));
    assert!(note.contains("100 ms"));
    assert!(
        !button(&harness, &t("indexes.queries.turn_off"))
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .query_by_label(&t("indexes.queries.enable"))
            .is_none()
    );
    click(&mut harness, &t("indexes.queries.record_all"));
    click(&mut harness, &t("indexes.queries.turn_off"));
    assert_eq!(
        harness.state().seen,
        vec![QueriesAction::AskEnable, QueriesAction::AskDisable]
    );
}

#[test]
fn the_other_levels_show_only_their_one_button_and_no_note() {
    let record_all = t("indexes.queries.record_all");
    for (level, shown, hidden) in [
        (0, "indexes.queries.enable", "indexes.queries.turn_off"),
        (2, "indexes.queries.turn_off", "indexes.queries.enable"),
    ] {
        let harness = open(known(level), false);
        assert_eq!(harness.query_all_by_label(&t(shown)).count(), 1);
        assert!(harness.query_by_label(&t(hidden)).is_none());
        assert!(harness.query_by_label(&record_all).is_none());
        assert!(!texts(&harness).contains("Level 1 records"));
    }
}

#[test]
fn read_only_disables_both_level_one_buttons() {
    let harness = open(known(1), true);
    assert_shows(&harness, &[t("indexes.queries.read_only_note")]);
    for key in ["indexes.queries.turn_off", "indexes.queries.record_all"] {
        assert!(
            button(&harness, &t(key)).accesskit_node().is_disabled(),
            "{key}"
        );
    }
}

#[test]
fn typed_rows_are_labelled_as_typed() {
    let mut typed = summary("status", VerdictKind::CollectionScan, 1);
    typed.source = ShapeSource::Typed;
    let harness = open(known(2).with_shapes(vec![typed]), false);
    assert_shows(&harness, &[t("indexes.queries.source_typed")]);
}

#[test]
fn unavailable_reasons_are_fixed_texts_never_server_text() {
    for (error, key) in [
        (
            CoreError::Unauthorized,
            "indexes.queries.profiler_unauthorized",
        ),
        (
            CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into()),
            "indexes.queries.profiler_mongos",
        ),
        (
            CoreError::Database(curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE.into()),
            "indexes.queries.profiler_timed_out",
        ),
        (
            CoreError::Unreachable,
            "indexes.queries.profiler_connection_lost",
        ),
        (
            CoreError::ConnectionInterrupted(SECRET.into()),
            "indexes.queries.profiler_connection_lost",
        ),
        (
            CoreError::Database(SECRET.into()),
            "indexes.queries.profiler_unavailable",
        ),
        (
            CoreError::Unsupported(SECRET.into()),
            "indexes.queries.profiler_unavailable",
        ),
    ] {
        let state = QueriesState::default().with_profiling(ProfilingView::Unavailable(error));
        let harness = open(state, false);
        assert_shows(&harness, &[t(key), t("indexes.queries.empty_unavailable")]);
        assert!(
            harness
                .query_by_label(&t("indexes.queries.enable"))
                .is_none()
        );
    }
}

#[test]
fn a_failed_read_is_shown_inline_without_the_server_text() {
    let state = known(2).failed(CoreError::Database(SECRET.into()));
    let harness = open(state, false);
    assert_shows(&harness, &[t("indexes.queries.error")]);
}

#[test]
fn a_failed_read_is_not_hidden_by_the_profiler_being_on() {
    // The profiler answered after the failed history read: the body must not claim nothing was
    // recorded.
    let state = known(2).failed(CoreError::Database(SECRET.into()));
    let harness = open(state, false);
    assert_shows(&harness, &[t("indexes.queries.error")]);
    assert!(
        !texts(&harness).contains(&t("indexes.queries.empty_on")),
        "the empty-profiler text would be untrue"
    );
}

#[test]
fn an_unconfirmed_profiler_change_is_shown_next_to_the_rows() {
    let mut state = known(2).with_shapes(vec![summary("a", VerdictKind::CollectionScan, 1)]);
    state.profiler_error = Some(CoreError::Database(SECRET.into()));
    let harness = open(state, false);
    assert_shows(&harness, &[t("indexes.queries.profiler_error")]);
}

#[test]
fn a_row_that_is_fine_now_says_what_the_worst_was() {
    let mut fixed = summary("a", VerdictKind::CollectionScan, 4);
    fixed.latest = VerdictKind::IndexUsed;
    let harness = open(known(2).with_shapes(vec![fixed]), false);
    let worst_title = t("query.verdict.title_collscan");
    let note = I18n::english().t_with(
        "indexes.queries.worst_in_range",
        &[("verdict", &worst_title)],
    );
    assert_shows(&harness, &[note]);
}

#[test]
fn checking_says_so() {
    let harness = open(QueriesState::default(), false);
    assert_shows(&harness, &[t("indexes.queries.profiler_checking")]);
}

#[test]
fn poller_statuses_have_their_own_texts() {
    let i18n = I18n::english();
    let cases = [
        (
            ProfilerStatus::Reading { databases: 3 },
            i18n.t_with(
                "indexes.queries.poller_reading_other",
                &[("databases", "3")],
            ),
        ),
        (
            ProfilerStatus::MissedEntries { db: "shop".into() },
            i18n.t_with("indexes.queries.poller_missed", &[("db", "shop")]),
        ),
        (
            ProfilerStatus::Reading { databases: 1 },
            i18n.t_with("indexes.queries.poller_reading_one", &[("databases", "1")]),
        ),
        (ProfilerStatus::Failing, t("indexes.queries.poller_failing")),
        (
            ProfilerStatus::Paused(SamplerPause::NotAuthorized),
            t("indexes.queries.poller_paused_not_authorized"),
        ),
        (
            ProfilerStatus::Paused(SamplerPause::ConnectionLost),
            t("indexes.queries.poller_paused_connection_lost"),
        ),
        (
            ProfilerStatus::Paused(SamplerPause::HistoryBusy),
            t("indexes.queries.poller_paused_history_busy"),
        ),
        (
            ProfilerStatus::Paused(SamplerPause::HistoryUnavailable),
            t("indexes.queries.poller_paused_history_unavailable"),
        ),
        (ProfilerStatus::Idle, t("indexes.queries.poller_idle")),
    ];
    for (status, expected) in cases {
        let harness = open(known(2).with_status(status.clone()), false);
        assert_shows(&harness, &[expected]);
    }
    assert!(t("indexes.queries.poller_missed").contains("missed"));
}

#[test]
fn read_only_disables_the_button_and_says_why() {
    for state in [known(0), known(2)] {
        let label = if state == known(0) {
            t("indexes.queries.enable")
        } else {
            t("indexes.queries.turn_off")
        };
        let mut harness = open(state, true);
        assert_shows(&harness, &[t("indexes.queries.read_only_note")]);
        assert!(button(&harness, &label).accesskit_node().is_disabled());
        button(&harness, &label).click();
        harness.run_steps(3);
        assert!(harness.state().seen.is_empty());
    }
}

/// Every status button (Enable profiler, Turn off, Record all operations) is reported disabled
/// and inert on a read-only connection, whichever level the profiler is at.
#[test]
fn read_only_status_buttons_are_disabled_and_emit_nothing() {
    let cases = [
        (0, vec!["indexes.queries.enable"]),
        (
            1,
            vec!["indexes.queries.turn_off", "indexes.queries.record_all"],
        ),
        (2, vec!["indexes.queries.turn_off"]),
    ];
    for (level, keys) in cases {
        let mut harness = open(known(level), true);
        for key in keys {
            let label = t(key);
            assert!(
                button(&harness, &label).accesskit_node().is_disabled(),
                "level {level}: {key} is not disabled"
            );
            click(&mut harness, &label);
            assert!(
                harness.state().seen.is_empty(),
                "level {level}: clicking {key} emitted {:?}",
                harness.state().seen
            );
        }
    }
}

#[test]
fn busy_disables_the_button() {
    let harness = open(known(0).write_sent("shop"), false);
    assert!(
        button(&harness, &t("indexes.queries.enable"))
            .accesskit_node()
            .is_disabled()
    );
    assert_shows(&harness, &[t("indexes.queries.busy")]);
}

#[test]
fn range_and_refresh_report_their_actions() {
    let mut harness = open(known(2), false);
    // The range switch is a segmented control (radio buttons), like the Indexes toolbar's.
    harness
        .get_by_role_and_label(Role::RadioButton, &t("indexes.range_week"))
        .click();
    harness.run_steps(3);
    click(&mut harness, &t("indexes.refresh"));
    assert_eq!(
        harness.state().seen,
        vec![
            QueriesAction::SetRange(ChartRange::Week),
            QueriesAction::Refresh
        ]
    );
}

#[test]
fn the_chart_has_a_text_alternative() {
    let state = known(2).with_shapes(vec![summary("status", VerdictKind::CollectionScan, 12)]);
    let harness = open(state, false);
    let i18n = I18n::english();
    assert_shows(
        &harness,
        &[i18n.t_with(
            "indexes.queries.chart_summary_other",
            &[("misses", "12"), ("range", &t("indexes.range_day"))],
        )],
    );
    let one = open(
        known(2).with_shapes(vec![summary("status", VerdictKind::CollectionScan, 1)]),
        false,
    );
    assert_shows(
        &one,
        &[i18n.t_with(
            "indexes.queries.chart_summary_one",
            &[("misses", "1"), ("range", &t("indexes.range_day"))],
        )],
    );
    let empty = open(known(2), false);
    assert_shows(&empty, &[t("indexes.queries.chart_empty")]);
}

/// The height of the status row and the chart under it: from the bottom of the range selector
/// (the toolbar row above the status) to the top of the chart's text summary (the chart has a
/// fixed height). A row drawn only in some states (or without its reserved height) changes this
/// number, so comparing it across states catches the layout jump: without the reserved height
/// the button-less states (Unknown, Loading, unavailable) are shorter than the ones with a
/// button. The states here record no shapes, so the summary is the empty-chart line.
fn status_height(state: QueriesState, read_only: bool) -> f32 {
    let harness = open(state, read_only);
    let (above, below) = status_band(&harness);
    below - above
}

/// The band the status row (and the chart) occupy: from the range selector's bottom to the top
/// of the empty-chart line.
fn status_band(harness: &Harness<'static, Frame>) -> (f32, f32) {
    let above = harness
        .get_by_label(&t("indexes.range_day"))
        .rect()
        .bottom();
    let below = harness
        .get_by_label(&t("indexes.queries.chart_empty"))
        .rect()
        .top();
    (above, below)
}

/// Whether the node carries text (a label's text is its value in the tree).
fn named(node: &egui_kittest::Node<'_>) -> bool {
    let n = node.accesskit_node();
    n.label().is_some() || n.value().is_some()
}

fn all_states() -> Vec<QueriesState> {
    let unavailable =
        QueriesState::default().with_profiling(ProfilingView::Unavailable(CoreError::Unauthorized));
    let statuses = [
        ProfilerStatus::Idle,
        ProfilerStatus::Reading { databases: 2 },
        ProfilerStatus::MissedEntries { db: "shop".into() },
        ProfilerStatus::Failing,
        ProfilerStatus::Paused(SamplerPause::NotAuthorized),
        ProfilerStatus::Paused(SamplerPause::ConnectionLost),
        ProfilerStatus::Paused(SamplerPause::HistoryBusy),
        ProfilerStatus::Paused(SamplerPause::HistoryUnavailable),
    ];
    let mut states = vec![
        QueriesState::default(),
        QueriesState::default().with_profiling(ProfilingView::Loading),
        known(0).write_sent("shop"),
        known(1).write_sent("shop"),
        unavailable.clone(),
    ];
    for status in statuses {
        for base in [known(0), known(1), known(2), unavailable.clone()] {
            states.push(base.with_status(status.clone()));
        }
    }
    states
}

#[test]
fn the_status_row_never_changes_height() {
    let reference = status_height(known(0), false);
    assert!(reference > 0.0);
    for (i, state) in all_states().into_iter().enumerate() {
        assert_eq!(status_height(state, false), reference, "state {i}");
    }
    assert_eq!(status_height(known(0), true), reference, "read-only");
    assert_eq!(
        status_height(known(1), true),
        reference,
        "read-only level 1"
    );
}

#[test]
fn at_a_narrow_width_nothing_of_the_status_row_is_beyond_the_edge() {
    const WIDTH: f32 = 360.0;
    let worst = [
        QueriesState::default()
            .with_profiling(ProfilingView::Unavailable(CoreError::Unsupported(
                PROFILER_MONGOS_MESSAGE.into(),
            )))
            .with_status(ProfilerStatus::Failing),
        known(1).with_status(ProfilerStatus::MissedEntries {
            db: "a_long_database_name".into(),
        }),
    ];
    for state in worst {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(WIDTH, 700.0))
            .build_ui_state(
                |ui, frame: &mut Frame| {
                    let ns = Namespace::new("shop", "orders");
                    let read_only = frame.read_only;
                    let _ = show_queries(ui, &frame.state, &ns, read_only, &I18n::english(), NOW);
                },
                Frame {
                    state: state.clone(),
                    read_only: true,
                    seen: Vec::new(),
                    copied: Vec::new(),
                },
            );
        harness.run_steps(4);
        let (above, below) = status_band(&harness);
        for node in harness.root().children_recursive() {
            let rect = node.rect();
            let in_band = rect.top() >= above && rect.top() < below;
            if named(&node) && in_band && rect.width() > 0.0 {
                assert!(
                    rect.right() <= WIDTH,
                    "{:?} {rect:?}",
                    node.accesskit_node().label()
                );
            }
        }
    }
}

#[test]
fn the_toggle_switches_views_and_opens_the_queries() {
    type Seen = (IndexesState, Vec<IndexesAction>);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            |ui, (state, seen): &mut Seen| {
                seen.extend(show_indexes(
                    ui,
                    state,
                    false,
                    &I18n::english(),
                    NOW,
                    &SamplingSettings::default(),
                ));
            },
            (
                IndexesState {
                    ns: Some(Namespace::new("shop", "orders")),
                    ..IndexesState::default()
                },
                Vec::new(),
            ),
        );
    harness.run_steps(3);
    let selected = |h: &Harness<'_, _>, key: &str| {
        h.get_by_role_and_label(Role::RadioButton, &t(key))
            .accesskit_node()
            .toggled()
    };
    assert_eq!(
        selected(&harness, "indexes.queries.view_indexes"),
        Some(Toggled::True)
    );
    assert_eq!(
        selected(&harness, "indexes.queries.view_queries"),
        Some(Toggled::False)
    );
    harness
        .get_by_role_and_label(Role::RadioButton, &t("indexes.queries.view_queries"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().1,
        vec![IndexesAction::ShowView(IndexesView::Queries)]
    );
    // In the Queries view the same toggle shows the Queries view and its own status row.
    harness.state_mut().0.view = IndexesView::Queries;
    harness.run_steps(3);
    harness.get_by_label(&t("indexes.queries.profiler_checking"));
    assert_eq!(
        selected(&harness, "indexes.queries.view_queries"),
        Some(Toggled::True)
    );
    harness
        .get_by_role_and_label(Role::RadioButton, &t("indexes.queries.view_indexes"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().1.last(),
        Some(&IndexesAction::ShowView(IndexesView::Indexes))
    );
}
