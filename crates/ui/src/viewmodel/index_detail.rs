//! Facts the Indexes tab derives from an index and its series. Pure and egui-free; text comes
//! out as `en.json` keys plus numbers, never as English.
use curlybrace_core::history::ChartRange;
use curlybrace_core::indexes::{IndexOptions, IndexSpec, KeyKind};
use curlybrace_core::runtime::IndexSeries;

/// The series drawn for `spec`: the one whose name AND definition hash match (a series of the
/// same name with another hash belongs to an older index that was dropped and recreated). When
/// several nodes report it, the first host in sort order wins.
pub fn series_of<'a>(series: &'a [IndexSeries], spec: &IndexSpec) -> Option<&'a IndexSeries> {
    let hash = spec.definition_hash();
    series
        .iter()
        .filter(|s| s.index_name == spec.name && s.definition_hash == hash)
        .min_by(|a, b| a.host.cmp(&b.host))
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

/// When the index was last used: the end of the newest bucket that holds a use.
pub fn last_used_secs(series: &IndexSeries) -> Option<i64> {
    series
        .buckets
        .iter()
        .filter(|b| b.uses > 0)
        .map(|b| b.start_secs + b.width_secs)
        .max()
}

/// The start of the oldest bucket on hand.
pub fn tracked_since_secs(series: &IndexSeries) -> Option<i64> {
    series.buckets.iter().map(|b| b.start_secs).min()
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

fn key_token(kind: &KeyKind) -> &'static str {
    match kind {
        KeyKind::Asc | KeyKind::Wildcard => "1",
        KeyKind::Desc => "-1",
        KeyKind::Text => "\"text\"",
        KeyKind::Geo2dSphere => "\"2dsphere\"",
        KeyKind::Hashed => "\"hashed\"",
        KeyKind::Geo2d => "\"2d\"",
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
        }
    }

    fn bucket(start_secs: i64, uses: i64) -> Bucket {
        Bucket {
            start_secs,
            width_secs: 3_600,
            uses,
            restarts: 0,
            gap_secs: 0,
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
    fn the_first_host_in_sort_order_is_drawn() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let all = vec![series(&spec, "node-b"), series(&spec, "node-a")];
        assert_eq!(
            series_of(&all, &spec).map(|s| s.host.as_str()),
            Some("node-a")
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

    #[test]
    fn last_used_and_tracked_since_come_from_the_buckets() {
        let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
        let mut s = series(&spec, "n1");
        assert_eq!((last_used_secs(&s), tracked_since_secs(&s)), (None, None));
        s.buckets = vec![bucket(0, 0), bucket(3_600, 4), bucket(7_200, 0)];
        assert_eq!(last_used_secs(&s), Some(7_200));
        assert_eq!(tracked_since_secs(&s), Some(0));
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
