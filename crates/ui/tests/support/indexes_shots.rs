//! Fixed state for the Indexes tab screenshot: three indexes on `crm.customers`, the second one
//! selected with a day of usage that holds a server restart and a stretch without data.
use super::shots::{SHOT_NOW, browsing};
use curlybrace_core::connection::{ConnectionColor, SavedColor};
use curlybrace_core::history::{Bucket, ChartRange, Sample};
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{IndexSeries, SamplerStatus};
use curlybrace_core::store::CollectionIndex;
use curlybrace_core::test_support::mock_objects::{mock_index_spec, mock_index_spec_named};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::index_detail::anchored_now;
use curlybrace_ui::viewmodel::{
    CollectionTab, DropConfirm, IndexBuilderState, IndexesState, ViewMode,
};

const BUCKETS: i64 = 144;
const WIDTH: i64 = 600;

fn listed(spec: IndexSpec, size_bytes: u64) -> CollectionIndex {
    CollectionIndex { spec, size_bytes: Some(size_bytes) }
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
