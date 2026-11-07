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
use eframe::egui::accesskit::Role;
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
    // One suggestion (only the miss has one) and a shape copy button per row.
    assert_eq!(
        harness
            .query_all_by_label(&t("indexes.queries.copy_shape"))
            .count(),
        2
    );
    click(&mut harness, &t("query.verdict.copy"));
    let copied = harness.state().copied.clone();
    assert_eq!(copied.len(), 1);
    assert!(copied[0].contains("createIndex"), "{copied:?}");
    harness
        .query_all_by_label(&t("indexes.queries.copy_shape"))
        .next()
        .unwrap()
        .click();
    harness.run_steps(3);
    assert!(harness.state().copied.contains(&shape_json("status")));
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
            i18n.t_with("indexes.queries.poller_reading", &[("databases", "3")]),
        ),
        (
            ProfilerStatus::MissedEntries { db: "shop".into() },
            i18n.t_with("indexes.queries.poller_missed", &[("db", "shop")]),
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
    click(&mut harness, &t("indexes.range_week"));
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
            "indexes.queries.chart_summary",
            &[("misses", "12"), ("range", &t("indexes.range_day"))],
        )],
    );
    let empty = open(known(2), false);
    assert_shows(&empty, &[t("indexes.queries.chart_empty")]);
}

/// The y where the chart starts, i.e. the height of everything above it.
fn status_height(state: QueriesState, read_only: bool) -> f32 {
    let harness = open(state, read_only);
    let label = t("indexes.range_day");
    harness.get_by_label(&label).rect().top()
}

#[test]
fn the_status_row_never_changes_height() {
    let reference = status_height(known(0), false);
    assert!(reference > 0.0);
    let unavailable =
        QueriesState::default().with_profiling(ProfilingView::Unavailable(CoreError::Unauthorized));
    let states = [
        QueriesState::default(),
        QueriesState::default().with_profiling(ProfilingView::Loading),
        known(1),
        known(2),
        known(2).with_status(ProfilerStatus::Reading { databases: 2 }),
        known(2).with_status(ProfilerStatus::MissedEntries { db: "shop".into() }),
        known(2).with_status(ProfilerStatus::Failing),
        known(2).with_status(ProfilerStatus::Paused(SamplerPause::HistoryUnavailable)),
        known(2).with_status(ProfilerStatus::Paused(SamplerPause::ConnectionLost)),
        known(0).write_sent("shop"),
        unavailable,
    ];
    for (i, state) in states.into_iter().enumerate() {
        assert_eq!(status_height(state, false), reference, "state {i}");
    }
    assert_eq!(status_height(known(0), true), reference, "read-only");
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
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.queries.view_queries"))
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
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.queries.view_indexes"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().1.last(),
        Some(&IndexesAction::ShowView(IndexesView::Indexes))
    );
}
