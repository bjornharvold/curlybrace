use std::time::Duration;

use bson::doc;

use super::*;
use crate::pipeline_options::MAX_TIME_MS_CAP;
use crate::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};

const ALL: [Operation; 4] = [
    Operation::Preview,
    Operation::Count,
    Operation::Explain,
    Operation::Run,
];

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn collation() -> CollationOption {
    CollationOption {
        locale: "en".into(),
        strength: Some(Strength::Secondary),
        case_level: Some(true),
        case_first: Some(CaseFirst::Lower),
        numeric_ordering: Some(true),
        alternate: Some(Alternate::Shifted),
    }
}

fn disk_use(on: bool) -> PipelineOptions {
    PipelineOptions {
        allow_disk_use: Some(on),
        ..PipelineOptions::default()
    }
}

fn max_time(ms: u64) -> PipelineOptions {
    PipelineOptions {
        max_time_ms: Some(ms),
        ..PipelineOptions::default()
    }
}

fn reading(mode: ReadPreferenceMode) -> PipelineOptions {
    PipelineOptions {
        read_preference: Some(mode),
        ..PipelineOptions::default()
    }
}

#[test]
fn the_default_applies_today_s_behaviour_to_every_operation() {
    let none = PipelineOptions::default();
    for op in [Operation::Preview, Operation::Count] {
        let applied = none.applied(op);
        assert_eq!(applied.max_time, secs(10));
        assert_eq!(applied.allow_disk_use, Some(false));
        assert_eq!(applied.collation, None);
        assert_eq!(applied.hint, None);
        assert_eq!(applied.comment, None);
        assert_eq!(applied.read_preference, None);
    }
    let explain = none.applied(Operation::Explain);
    assert_eq!(explain.max_time, secs(10));
    assert_eq!(explain.allow_disk_use, None);
    assert_eq!(explain.read_preference, None);
    let run = none.applied(Operation::Run);
    assert_eq!(run.max_time, secs(60));
    assert_eq!(run.allow_disk_use, None);
    assert_eq!(run.read_preference, None);
    assert_eq!(run.collation, None);
}

#[test]
fn preview_and_count_never_allow_disk_use() {
    for op in [Operation::Preview, Operation::Count] {
        assert_eq!(disk_use(true).applied(op).allow_disk_use, Some(false));
        assert_eq!(disk_use(false).applied(op).allow_disk_use, Some(false));
    }
}

#[test]
fn explain_and_run_carry_disk_use_as_set() {
    for op in [Operation::Explain, Operation::Run] {
        assert_eq!(disk_use(true).applied(op).allow_disk_use, Some(true));
        assert_eq!(disk_use(false).applied(op).allow_disk_use, Some(false));
    }
}

#[test]
fn max_time_only_lowers() {
    let low = max_time(5_000);
    for op in ALL {
        assert_eq!(low.applied(op).max_time, Duration::from_millis(5_000));
    }
    let mid = max_time(30_000);
    assert_eq!(mid.applied(Operation::Preview).max_time, secs(10));
    assert_eq!(mid.applied(Operation::Count).max_time, secs(10));
    assert_eq!(mid.applied(Operation::Explain).max_time, secs(10));
    assert_eq!(mid.applied(Operation::Run).max_time, secs(30));
    let cap = max_time(60_000);
    assert_eq!(cap.applied(Operation::Preview).max_time, secs(10));
    assert_eq!(cap.applied(Operation::Explain).max_time, secs(10));
    assert_eq!(cap.applied(Operation::Run).max_time, secs(60));
}

#[test]
fn the_run_never_carries_a_read_preference() {
    for mode in [
        ReadPreferenceMode::Primary,
        ReadPreferenceMode::Secondary,
        ReadPreferenceMode::Nearest,
    ] {
        assert_eq!(reading(mode).applied(Operation::Run).read_preference, None);
    }
}

#[test]
fn reads_carry_the_read_preference_as_set() {
    for op in [Operation::Preview, Operation::Count, Operation::Explain] {
        assert_eq!(
            reading(ReadPreferenceMode::SecondaryPreferred)
                .applied(op)
                .read_preference,
            Some(ReadPreferenceMode::SecondaryPreferred)
        );
    }
}

#[test]
fn collation_hint_and_comment_reach_every_operation() {
    let options = PipelineOptions {
        collation: Some(collation()),
        hint: Some(HintOption::Keys(doc! { "a": 1 })),
        comment: Some("why".into()),
        ..PipelineOptions::default()
    };
    for op in ALL {
        let applied = options.applied(op);
        assert_eq!(applied.collation, Some(collation()));
        assert_eq!(applied.hint, Some(HintOption::Keys(doc! { "a": 1 })));
        assert_eq!(applied.comment.as_deref(), Some("why"));
    }
}

#[test]
fn max_time_lowered_reports_only_a_real_lowering() {
    let none = PipelineOptions::default();
    let low = max_time(5_000);
    let mid = max_time(30_000);
    let cap = max_time(60_000);
    for op in ALL {
        assert!(!none.applied(op).max_time_lowered(op));
        assert!(low.applied(op).max_time_lowered(op));
    }
    assert!(
        !mid.applied(Operation::Preview)
            .max_time_lowered(Operation::Preview)
    );
    assert!(mid.applied(Operation::Run).max_time_lowered(Operation::Run));
    assert!(!cap.applied(Operation::Run).max_time_lowered(Operation::Run));
}

#[test]
fn bounded_sends_no_user_option() {
    let preview = AppliedOptions::bounded(Operation::Preview, Duration::from_millis(1_500));
    assert_eq!(preview.max_time, Duration::from_millis(1_500));
    assert_eq!(preview.allow_disk_use, Some(false));
    assert_eq!(preview.collation, None);
    let run = AppliedOptions::bounded(Operation::Run, secs(60));
    assert_eq!(run.allow_disk_use, None);
    assert_eq!(run.read_preference, None);
    let explain = AppliedOptions::bounded(Operation::Explain, secs(10));
    assert_eq!(explain.allow_disk_use, None);
}

#[test]
fn the_defaults_are_the_existing_constants() {
    assert_eq!(Operation::Preview.default_max_time_ms(), 10_000);
    assert_eq!(Operation::Count.default_max_time_ms(), 10_000);
    assert_eq!(Operation::Explain.default_max_time_ms(), 10_000);
    assert_eq!(Operation::Run.default_max_time_ms(), 60_000);
}

#[test]
fn a_zero_or_oversized_time_never_means_no_limit() {
    // Validation refuses these, but a value built by hand must not turn into `maxTimeMS: 0`
    // (no limit) or go past the cap.
    for op in ALL {
        assert_eq!(
            max_time(0).applied(op).max_time,
            Duration::from_millis(1),
            "{op:?}"
        );
        assert_eq!(
            AppliedOptions::bounded(op, Duration::ZERO).max_time,
            Duration::from_millis(1)
        );
        assert_eq!(
            max_time(u64::MAX).applied(op).max_time,
            Duration::from_millis(op.default_max_time_ms())
        );
        assert_eq!(
            AppliedOptions::bounded(op, Duration::from_secs(3_600)).max_time,
            Duration::from_millis(MAX_TIME_MS_CAP)
        );
    }
}
