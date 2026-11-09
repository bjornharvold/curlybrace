//! Fixed state for the Indexes tab screenshot: three indexes on `crm.customers`, the second one
//! selected with a day of usage that holds a server restart and a stretch without data.
use super::shots::{SHOT_NOW, browsing};
use curlybrace_core::connection::{ConnectionColor, SavedColor};
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{
    Bucket, ChartRange, Sample, ShapeBucket, ShapeSource, ShapeSummary,
};
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{IndexSeries, ProfilerStatus, SamplerStatus};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{CollectionIndex, ProfilingState, Query};
use curlybrace_core::test_support::mock_objects::{mock_index_spec, mock_index_spec_named};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::index_detail::anchored_now;
use curlybrace_ui::viewmodel::{
    CollectionTab, DropConfirm, IndexBuilderState, IndexesState, IndexesView, ProfilingView,
    QueriesState, ViewMode,
};

const BUCKETS: i64 = 144;
const WIDTH: i64 = 600;

fn listed(spec: IndexSpec, size_bytes: u64) -> CollectionIndex {
    CollectionIndex {
        spec,
        size_bytes: Some(size_bytes),
    }
}

/// A day of 10 minute buckets ending at the chart's right edge. Buckets 60..80 are a gap and a
/// restart falls in bucket 100; the uses follow a fixed pattern.
fn day_of_usage(spec: &IndexSpec, ops: i64) -> IndexSeries {
    let end = anchored_now(ChartRange::Day, SHOT_NOW);
    let buckets = (0..BUCKETS)
        .map(|i| Bucket {
            start_secs: end - (BUCKETS - i) * WIDTH,
            width_secs: WIDTH,
            uses: if (60..80).contains(&i) {
                0
            } else {
                (i * 7) % 11 + 1
            },
            restarts: u32::from(i == 100),
            gap_secs: if (60..80).contains(&i) { WIDTH } else { 0 },
        })
        .collect();
    IndexSeries {
        index_name: spec.name.clone(),
        definition_hash: spec.definition_hash(),
        host: "shard-0.example.com:27017".into(),
        buckets,
        latest: Some(Sample {
            at_secs: end,
            ops,
            since_ms: 1,
        }),
        // Real times: the last use ten minutes before the wall clock the shots freeze.
        last_used_at_secs: Some(SHOT_NOW - 600),
        tracked_since_secs: Some(end - BUCKETS * WIDTH),
    }
}

/// The Indexes tab of `crm.customers` with its usage read and the sampler running.
pub fn indexes_tab(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let id = mock_index_spec_named("_id_", &[("_id", KeyKind::Asc)]);
    let compound = mock_index_spec();
    let mut legacy = mock_index_spec_named("legacy_1", &[("legacy", KeyKind::Asc)]);
    legacy.options.hidden = true;
    let series = vec![day_of_usage(&id, 5_210), day_of_usage(&compound, 1_234)];
    state.tab = CollectionTab::Indexes;
    state.indexes = IndexesState {
        ns: state.selected_ns().cloned(),
        indexes: vec![
            listed(id, 36_864),
            listed(compound.clone(), 1_572_864),
            listed(legacy, 20_480),
        ],
        series,
        selected: Some(compound.name),
        sampler: Some(SamplerStatus::Running),
        ..IndexesState::default()
    };
}

/// The Indexes tab with the builder open: a compound unique index on `crm.customers` with a
/// partial filter, on a collection large enough to carry the build warning.
pub fn index_builder(state: &mut AppState) {
    indexes_tab(state);
    let ns = state.selected_ns().cloned().expect("a collection is shown");
    let builder = IndexBuilderState::new(ns)
        .with_field(0, "customerId")
        .adding_key()
        .with_field(1, "createdAt")
        .with_kind(1, KeyKind::Desc)
        .with_unique(true)
        .with_partial_filter_text("{ \"status\": \"active\" }")
        .with_estimate(2_400_000);
    state.indexes = state.indexes.clone().with_builder(Some(builder));
}

/// The typed drop confirmation of the compound index on a production connection, half typed.
pub fn index_drop_confirm(state: &mut AppState) {
    indexes_tab(state);
    let active = state.active.expect("browsing opens a session");
    for cfg in state.conn.connections.iter_mut().filter(|c| c.id == active) {
        cfg.production = true;
        cfg.color = Some(SavedColor::Known(ConnectionColor::Red));
    }
    let name = mock_index_spec().name;
    let typed = name.chars().take(14).collect::<String>();
    let confirm = DropConfirm::new(name).with_typed(typed);
    state.indexes = state.indexes.clone().with_drop_confirm(Some(confirm));
}

/// One recorded query shape on `field` with `verdict`: a day of 10 minute buckets in which the
/// misses follow `miss_every` (none for a shape that never misses).
fn shape_summary(field: &str, verdict: VerdictKind, miss_every: Option<i64>) -> ShapeSummary {
    let end = anchored_now(ChartRange::Day, SHOT_NOW);
    let buckets: Vec<ShapeBucket> = (0..BUCKETS)
        .map(|i| {
            let executions = (i * 5) % 13 + 2;
            let misses = miss_every
                .filter(|every| i % every == 0)
                .map_or(0, |_| executions / 2 + 1);
            ShapeBucket {
                start_secs: end - (BUCKETS - i) * WIDTH,
                width_secs: WIDTH,
                executions: executions as u64,
                misses: misses as u64,
                gap_secs: 0,
            }
        })
        .collect();
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: from_query(&Query {
            filter: bson::doc! { field: 1 },
            ..Query::default()
        })
        .json(),
        source: ShapeSource::App,
        executions: buckets.iter().map(|b| b.executions).sum(),
        misses: buckets.iter().map(|b| b.misses).sum(),
        total_millis: 52_000,
        max_millis: 840,
        worst: verdict,
        latest: verdict,
        buckets,
    }
}

/// The Queries view of `crm.customers`: the profiler on and read, and shapes with a bad, a
/// warning and a good verdict, so the chips of every tone and the miss trend show.
pub fn queries_view(state: &mut AppState) {
    indexes_tab(state);
    let queries = QueriesState::default()
        .with_profiling(ProfilingView::Known(ProfilingState {
            level: 2,
            slow_ms: 100,
        }))
        .with_status(ProfilerStatus::Reading { databases: 1 })
        .with_shapes(vec![
            shape_summary("email", VerdictKind::CollectionScan, Some(3)),
            shape_summary("createdAt", VerdictKind::SortNotCovered, Some(9)),
            shape_summary("customerId", VerdictKind::IndexUsed, None),
        ]);
    state.indexes = state
        .indexes
        .clone()
        .with_view(IndexesView::Queries)
        .with_queries(queries);
}
