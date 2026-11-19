//! Create index... and Last run on the rows of the Queries view in the accesskit tree, rendered
//! with the REAL English locale: the button is named for its row, off with its own reason for
//! each cause, absent on a good row, and each row says when it last ran. Fast, offline.
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeBucket, ShapeSource, ShapeSummary};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{Namespace, ProfilingState, Query};
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::{ProfilingView, QueriesAction, QueriesState};
use curlybrace_ui::views::create_index::CreateIndex;
use curlybrace_ui::views::queries::show_queries_led;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const NOW: i64 = 1_800_000_000;

struct Frame {
    state: QueriesState,
    create: CreateIndex,
    seen: Vec<QueriesAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn summary(field: &str, worst: VerdictKind, source: ShapeSource) -> ShapeSummary {
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: from_query(&Query {
            filter: bson::doc! { field: 1 },
            ..Query::default()
        })
        .json(),
        source,
        executions: 42,
        misses: 3,
        total_millis: 900,
        max_millis: 350,
        worst,
        latest: worst,
        // The newest bucket ended 10 minutes before now.
        buckets: vec![ShapeBucket {
            start_secs: NOW - 1_200,
            width_secs: 600,
            executions: 42,
            misses: 3,
            gap_secs: 0,
        }],
    }
}

fn known(shapes: Vec<ShapeSummary>) -> QueriesState {
    QueriesState::default()
        .with_profiling(ProfilingView::Known(ProfilingState {
            level: 2,
            slow_ms: 100,
        }))
        .with_shapes(shapes)
}

fn open(state: QueriesState, create: CreateIndex, size: egui::Vec2) -> Harness<'static, Frame> {
    let mut harness = Harness::builder().with_size(size).build_ui_state(
        |ui, frame: &mut Frame| {
            let ns = Namespace::new("shop", "orders");
            let read_only = frame.create == CreateIndex::ReadOnly;
            let (action, ()) = show_queries_led(
                ui,
                &frame.state,
                &ns,
                (read_only, frame.create),
                &I18n::english(),
                NOW,
                |_| (),
            );
            frame.seen.extend(action);
        },
        Frame {
            state,
            create,
            seen: Vec::new(),
        },
    );
    harness.run_steps(4);
    harness
}

fn big() -> egui::Vec2 {
    egui::vec2(1000.0, 700.0)
}

fn create_label(position: &str, total: &str) -> String {
    I18n::english().t_with(
        "indexes.queries.create_index_n",
        &[("position", position), ("total", total)],
    )
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

#[test]
fn a_miss_row_has_a_named_create_index_button() {
    let state = known(vec![
        summary("status", VerdictKind::CollectionScan, ShapeSource::App),
        summary("sku", VerdictKind::IndexUsed, ShapeSource::App),
    ]);
    let mut harness = open(state, CreateIndex::On, big());
    let label = create_label("1", "2");
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, &label)
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().seen,
        vec![QueriesAction::CreateIndex {
            hash: "hash-status".into(),
            source: ShapeSource::App
        }]
    );
}

#[test]
fn the_accessible_name_starts_with_the_drawn_text() {
    // WCAG 2.5.3, label in name: the drawn text is "Create index" (then an ellipsis).
    let name = create_label("1", "1");
    assert!(
        name.starts_with("Create index"),
        "{name} must start with the drawn text"
    );
    assert!(t("indexes.create_suggested").starts_with("Create index"));
}

#[test]
fn create_index_is_off_with_its_own_reason_for_each_cause() {
    for (create, key) in [
        (CreateIndex::ReadOnly, "indexes.create_suggested_read_only"),
        (CreateIndex::IndexWrite, "indexes.write_busy"),
        (CreateIndex::Prompt, "indexes.create_prompt_open"),
        (CreateIndex::WriteWarning, "indexes.create_warning_open"),
    ] {
        let state = known(vec![summary(
            "status",
            VerdictKind::CollectionScan,
            ShapeSource::App,
        )]);
        let mut harness = open(state, create, big());
        let label = create_label("1", "1");
        let reason = t(key);
        let node = harness.get_by_role_and_label(Role::Button, &label);
        assert!(node.accesskit_node().is_disabled(), "{key}");
        assert_eq!(
            node.accesskit_node().description().as_deref(),
            Some(reason.as_str()),
            "{key}"
        );
        node.click();
        harness.run_steps(3);
        assert!(harness.state().seen.is_empty(), "{key}");
    }
}

#[test]
fn a_good_row_has_no_create_index_button() {
    let state = known(vec![summary(
        "sku",
        VerdictKind::IndexUsed,
        ShapeSource::App,
    )]);
    let harness = open(state, CreateIndex::On, big());
    assert!(harness.query_by_label(&create_label("1", "1")).is_none());
    assert!(!texts(&harness).contains("Create index"));
}

#[test]
fn each_row_says_when_it_last_ran() {
    let state = known(vec![
        summary("status", VerdictKind::CollectionScan, ShapeSource::App),
        summary("sku", VerdictKind::IndexUsed, ShapeSource::App),
    ]);
    let harness = open(state, CreateIndex::On, big());
    // From the end of the newest bucket: 10 minutes ago.
    let when = I18n::english().t_with("indexes.ago_minutes", &[("n", "10")]);
    let line = I18n::english().t_with("indexes.queries.last_run", &[("when", &when)]);
    assert_eq!(line, "Last run 10 min ago");
    assert_eq!(harness.query_all_by_label(&line).count(), 2);
    // A row with no bucket that ran says nothing rather than guessing.
    let mut quiet = summary("none", VerdictKind::IndexUsed, ShapeSource::App);
    quiet.buckets.clear();
    let harness = open(known(vec![quiet]), CreateIndex::On, big());
    assert!(!texts(&harness).contains("Last run"));
}

#[test]
fn the_counts_row_labels_never_split_across_lines() {
    // At 800x500 and narrower, each label of the counts row moves whole: its text is one line.
    for size in [egui::vec2(800.0, 500.0), egui::vec2(420.0, 600.0)] {
        let state = known(vec![summary(
            "status",
            VerdictKind::CollectionScan,
            ShapeSource::App,
        )]);
        let harness = open(state, CreateIndex::On, size);
        for text in [
            "42 runs",
            "3 misses",
            "slowest 350 ms",
            "Last run 10 min ago",
        ] {
            let node = harness.get_by_label(text);
            assert!(
                node.rect().height() < 24.0,
                "`{text}` is wrapped at {size:?}: {:?}",
                node.rect()
            );
            assert!(
                node.rect().right() <= size.x,
                "`{text}` runs off at {size:?}"
            );
        }
    }
}
