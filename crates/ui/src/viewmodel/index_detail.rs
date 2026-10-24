//! Facts the Indexes tab derives from an index and its series. Pure and egui-free; text comes
//! out as `en.json` keys plus numbers, never as English.
use curlybrace_core::history::ChartRange;
use curlybrace_core::indexes::{IndexOptions, IndexSpec, KeyKind};
use curlybrace_core::runtime::{IndexSeries, SamplerPause, SamplerStatus};

/// The series drawn for `spec`: the one whose name AND definition hash match (a series of the
/// same name with another hash belongs to an older index that was dropped and recreated). When
/// several hosts report it (a failover moved the reads, or the connection now points at another
/// server), the one with the newest reading wins, so the chart follows the node being read; a
/// series without a reading loses to one with it, and equal readings fall back to the first host
/// in sort order.
pub fn series_of<'a>(series: &'a [IndexSeries], spec: &IndexSpec) -> Option<&'a IndexSeries> {
    let hash = spec.definition_hash();
    series
        .iter()
        .filter(|s| s.index_name == spec.name && s.definition_hash == hash)
        .min_by(|a, b| {
            let newest = |s: &IndexSeries| s.latest.map(|sample| sample.at_secs);
            newest(b).cmp(&newest(a)).then_with(|| a.host.cmp(&b.host))
        })
}

/// The "now" a chart is drawn against: the next bucket boundary after `now_secs`, so the current
/// bucket is whole and the bucket grid stays put for as long as the bucket is wide.
pub fn anchored_now(range: ChartRange, now_secs: i64) -> i64 {
    let width = range.bucket_width_secs();
    now_secs.div_euclid(width) * width + width
}

/// The ranges the switch offers, shortest first.
pub const RANGES: [ChartRange; 3] = [ChartRange::Day, ChartRange::Week, ChartRange::Month];

/// The `en.json` key of a range's label.
pub fn range_key(range: ChartRange) -> &'static str {
    match range {
        ChartRange::Day => "indexes.range_day",
        ChartRange::Week => "indexes.range_week",
        ChartRange::Month => "indexes.range_month",
    }
}

/// How long ago something was, in the coarsest unit that is at least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Age {
    Now,
    Minutes(i64),
    Hours(i64),
    Days(i64),
}

impl Age {
    pub fn of(secs: i64) -> Self {
        match secs {
            s if s < 60 => Self::Now,
            s if s < 3_600 => Self::Minutes(s / 60),
            s if s < 86_400 => Self::Hours(s / 3_600),
            s => Self::Days(s / 86_400),
        }
    }

    /// The `en.json` key of the phrase; `{n}` is [`Age::count`].
    pub fn key(self) -> &'static str {
        match self {
            Self::Now => "indexes.ago_now",
            Self::Minutes(_) => "indexes.ago_minutes",
            Self::Hours(_) => "indexes.ago_hours",
            Self::Days(_) => "indexes.ago_days",
        }
    }

    pub fn count(self) -> i64 {
        match self {
            Self::Now => 0,
            Self::Minutes(n) | Self::Hours(n) | Self::Days(n) => n,
        }
    }
}

/// The `en.json` key of the line shown in place of a chart: the history cannot be used (the
/// sampler said so), no series exists for the collection yet, or none for this index.
pub fn no_series_key(any_series: bool, sampler: Option<SamplerStatus>) -> &'static str {
    match (any_series, sampler) {
        (_, Some(SamplerStatus::Paused(SamplerPause::HistoryUnavailable))) => {
            "indexes.pause_history_unavailable"
        }
        (true, _) => "indexes.no_series",
        (false, _) => "indexes.no_history",
    }
}

/// How long ago the index was last used, from the real time of its newest use (never from a
/// bucket edge, so it is the same on every range and a recent use is never rounded into the
/// future). `None` when no use is on record.
pub fn last_used_age(series: &IndexSeries, now_secs: i64) -> Option<Age> {
    series
        .last_used_at_secs
        .map(|at| Age::of((now_secs - at).max(0)))
}

/// How long ago tracking began, from the oldest data on hand (the same on every range).
pub fn tracked_since_age(series: &IndexSeries, now_secs: i64) -> Option<Age> {
    series
        .tracked_since_secs
        .map(|at| Age::of((now_secs - at).max(0)))
}

/// A size as an `en.json` key and the number to put in `{n}`.
pub fn size_parts(bytes: u64) -> (&'static str, String) {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    match value {
        v if v < KIB => ("indexes.size_bytes", bytes.to_string()),
        v if v < KIB * KIB => ("indexes.size_kib", format!("{:.1}", v / KIB)),
        v if v < KIB * KIB * KIB => ("indexes.size_mib", format!("{:.1}", v / (KIB * KIB))),
        v => ("indexes.size_gib", format!("{:.1}", v / (KIB * KIB * KIB))),
    }
}

/// The key part of an index definition the way MongoDB writes it: `a: 1, b: -1, c: "text"`.
/// The values are MongoDB syntax, not prose, so they are not translated.
pub fn definition_text(spec: &IndexSpec) -> String {
    spec.keys
        .iter()
        .map(|k| format!("{}: {}", k.field, key_token(&k.kind)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn key_token(kind: &KeyKind) -> String {
    match kind {
        KeyKind::Asc | KeyKind::Wildcard => "1".into(),
        KeyKind::Desc => "-1".into(),
        KeyKind::Text => "\"text\"".into(),
        KeyKind::Geo2dSphere => "\"2dsphere\"".into(),
        KeyKind::Hashed => "\"hashed\"".into(),
        KeyKind::Geo2d => "\"2d\"".into(),
        KeyKind::Other(kind) => format!("{kind:?}"),
    }
}

/// The options that are set, as `en.json` keys with the number for `{n}` where there is one.
pub fn option_phrases(options: &IndexOptions) -> Vec<(&'static str, Option<i64>)> {
    [
        (options.unique, "indexes.option_unique", None),
        (options.sparse, "indexes.option_sparse", None),
        (
            options.expire_after_seconds.is_some(),
            "indexes.option_expire",
            options.expire_after_seconds,
        ),
        (
            options.partial_filter.is_some(),
            "indexes.option_partial",
            None,
        ),
        (
            options.collation.is_some(),
            "indexes.option_collation",
            None,
        ),
        (
            options.wildcard_projection.is_some(),
            "indexes.option_wildcard_projection",
            None,
        ),
    ]
    .into_iter()
    .filter(|(set, _, _)| *set)
    .map(|(_, key, n)| (key, n))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::history::Bucket;
    use curlybrace_core::test_support::mock_objects::mock_index_spec_named;

    fn series(spec: &IndexSpec, host: &str) -> IndexSeries {
        IndexSeries {
            index_name: spec.name.clone(),
            definition_hash: spec.definition_hash(),
            host: host.into(),
            buckets: Vec::new(),
            latest: None,
            last_used_at_secs: None,
            tracked_since_secs: None,
        }
    }

    #[test]
    fn the_series_of_an_index_matches_name_and_definition() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let recreated = mock_index_spec_named("a_1", &[("a", KeyKind::Desc)]);
        let all = vec![series(&recreated, "n1"), series(&spec, "n2")];
        assert_eq!(series_of(&all, &spec).map(|s| s.host.as_str()), Some("n2"));
        assert!(series_of(&all[..1], &spec).is_none(), "older index only");
    }

    #[test]
    fn without_readings_the_first_host_in_sort_order_is_drawn() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let all = vec![series(&spec, "node-b"), series(&spec, "node-a")];
        assert_eq!(
            series_of(&all, &spec).map(|s| s.host.as_str()),
            Some("node-a")
        );
    }

    fn read_at(mut series: IndexSeries, at_secs: i64) -> IndexSeries {
        series.latest = Some(curlybrace_core::history::Sample {
            at_secs,
            ops: 1,
            since_ms: 1,
        });
        series
    }

    #[test]
    fn the_host_with_the_newest_reading_is_drawn_even_when_it_sorts_later() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let all = vec![
            read_at(series(&spec, "node-a"), 100),
            read_at(series(&spec, "node-b"), 200),
        ];
        assert_eq!(
            series_of(&all, &spec).map(|s| s.host.as_str()),
            Some("node-b")
        );
    }

    #[test]
    fn equal_readings_fall_back_to_the_first_host_and_no_reading_loses() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let tied = vec![
            read_at(series(&spec, "node-b"), 100),
            read_at(series(&spec, "node-a"), 100),
        ];
        assert_eq!(
            series_of(&tied, &spec).map(|s| s.host.as_str()),
            Some("node-a")
        );
        let unread = vec![series(&spec, "node-a"), read_at(series(&spec, "node-b"), 1)];
        assert_eq!(
            series_of(&unread, &spec).map(|s| s.host.as_str()),
            Some("node-b")
        );
    }

    #[test]
    fn the_chart_ends_at_the_next_bucket_boundary() {
        assert_eq!(anchored_now(ChartRange::Week, 7_200), 10_800);
        assert_eq!(anchored_now(ChartRange::Week, 7_201), 10_800);
        assert_eq!(anchored_now(ChartRange::Day, 1_200), 1_800);
    }

    #[test]
    fn an_age_uses_the_coarsest_whole_unit() {
        assert_eq!(Age::of(59), Age::Now);
        assert_eq!(Age::of(120), Age::Minutes(2));
        assert_eq!(Age::of(7_300), Age::Hours(2));
        assert_eq!(Age::of(3 * 86_400 + 5), Age::Days(3));
        assert_eq!(Age::Days(3).count(), 3);
        assert_eq!(Age::Now.key(), "indexes.ago_now");
    }

    /// The series the server returns for `range`: the facts are real times, the buckets are
    /// as wide as the range makes them.
    fn for_range(range: ChartRange, now: i64, used: Option<i64>, since: i64) -> IndexSeries {
        let width = range.bucket_width_secs();
        let end = anchored_now(range, now);
        IndexSeries {
            buckets: vec![Bucket {
                start_secs: end - width,
                width_secs: width,
                uses: 1,
                restarts: 0,
                gap_secs: 0,
            }],
            last_used_at_secs: used,
            tracked_since_secs: Some(since),
            ..series(&mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]), "n1")
        }
    }

    #[test]
    fn last_used_and_tracked_since_are_the_same_on_every_range() {
        let now = 1_800_000_000 / 86_400 * 86_400 + 23 * 3_600;
        for (used, expected) in [
            (now - 59 * 60, Age::Minutes(59)),
            (now - (23 * 3_600 - 300), Age::Hours(22)),
        ] {
            for range in RANGES {
                let s = for_range(range, now, Some(used), now - 3 * 86_400);
                assert_eq!(last_used_age(&s, now), Some(expected), "{range:?}");
                assert_eq!(tracked_since_age(&s, now), Some(Age::Days(3)), "{range:?}");
            }
        }
    }

    #[test]
    fn an_unavailable_history_is_never_called_empty() {
        let unavailable = Some(SamplerStatus::Paused(SamplerPause::HistoryUnavailable));
        for any in [false, true] {
            assert_eq!(
                no_series_key(any, unavailable),
                "indexes.pause_history_unavailable"
            );
        }
        assert_eq!(no_series_key(false, None), "indexes.no_history");
        assert_eq!(
            no_series_key(false, Some(SamplerStatus::Running)),
            "indexes.no_history"
        );
        assert_eq!(no_series_key(true, None), "indexes.no_series");
    }

    #[test]
    fn no_recorded_use_is_none_and_a_clock_behind_the_sample_is_just_now() {
        let s = for_range(ChartRange::Week, 10_000, None, 0);
        assert_eq!(last_used_age(&s, 10_000), None);
        let ahead = for_range(ChartRange::Week, 10_000, Some(10_500), 0);
        assert_eq!(last_used_age(&ahead, 10_000), Some(Age::Now));
    }

    #[test]
    fn sizes_pick_a_unit() {
        assert_eq!(size_parts(512), ("indexes.size_bytes", "512".into()));
        assert_eq!(size_parts(1_536), ("indexes.size_kib", "1.5".into()));
        assert_eq!(size_parts(3 << 20), ("indexes.size_mib", "3.0".into()));
        assert_eq!(size_parts(2 << 30), ("indexes.size_gib", "2.0".into()));
    }

    #[test]
    fn the_definition_reads_like_mongodb_writes_it() {
        let spec = mock_index_spec_named("n", &[("a", KeyKind::Asc), ("b", KeyKind::Text)]);
        assert_eq!(definition_text(&spec), "a: 1, b: \"text\"");
    }

    #[test]
    fn only_the_options_that_are_set_are_listed() {
        let options = IndexOptions {
            unique: true,
            expire_after_seconds: Some(60),
            ..IndexOptions::default()
        };
        assert_eq!(
            option_phrases(&options),
            vec![
                ("indexes.option_unique", None),
                ("indexes.option_expire", Some(60))
            ]
        );
        assert!(option_phrases(&IndexOptions::default()).is_empty());
    }
}
