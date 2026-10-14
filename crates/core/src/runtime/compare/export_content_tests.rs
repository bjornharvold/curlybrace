//! More export content checks: destination sides, unreadable ops and the JSON header.

use super::apply_tests::op;
use super::export_tests::{export, outcome_of, request, rig, standard_ops, text};
use crate::compare::apply::{ApplyKind, ApplyOp, Direction};
use crate::compare::protocol::{CompareEvent, ExportFormat};
use crate::error::CoreError;

fn mixed_ops() -> Vec<ApplyOp> {
    let mut to_source = op(1, 2, ApplyKind::Replace);
    to_source.direction = Direction::TargetToSource;
    vec![op(1, 1, ApplyKind::Insert), to_source]
}

#[tokio::test]
async fn a_script_with_two_destination_sides_is_refused_but_a_plan_may_mix_them() {
    let mut r = rig().await;
    let script = r.path("mixed.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, mixed_ops()),
        &script,
    )
    .await;
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::Failed {
            context: "compare_export",
            error: CoreError::Unsupported(m),
            ..
        }] if m == "export one direction at a time"
    ));
    assert!(!script.exists());
    let plan = r.path("mixed.json");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::JsonPlan, mixed_ops()),
        &plan,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 2);
    let json: serde_json::Value = serde_json::from_str(&text(&plan)).unwrap();
    let ops = json["pairs"][0]["operations"].as_array().unwrap();
    let seen: Vec<(&str, &str)> = ops
        .iter()
        .map(|o| {
            (
                o["direction"].as_str().unwrap(),
                o["destination"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [("sourceToTarget", "target"), ("targetToSource", "source")]
    );
    assert_eq!(json["header"]["complete"], true);
}

/// The sum of the counts in the `// operations:` line of a script.
pub(super) fn header_total(script: &str) -> u64 {
    let line = script
        .lines()
        .find(|l| l.starts_with("// operations:"))
        .unwrap();
    line.split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<u64>().unwrap())
        .sum()
}

#[tokio::test]
async fn unreadable_operations_are_listed_as_skipped_and_the_counts_add_up() {
    let mut r = rig().await;
    r.source
        .fail_reads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let path = r.path("unreadable.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    let outcome = outcome_of(&events);
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (0, 0, 3)
    );
    let content = text(&path);
    assert_eq!(content.matches("skipped (unreadable)").count(), 3);
    assert_eq!(header_total(&content), 3, "{content}");
    let plan = r.path("unreadable.json");
    export(
        &mut r,
        6,
        request(ExportFormat::JsonPlan, standard_ops()),
        &plan,
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&text(&plan)).unwrap();
    assert_eq!(json["header"]["counts"]["skipped"], 3);
    assert_eq!(json["pairs"][0]["operations"][0]["reason"], "unreadable");
}

#[tokio::test]
async fn a_json_label_round_trips_even_when_it_imitates_the_markers() {
    let mut r = rig().await;
    let label = "q\"\n\u{2028}\"complete\":false \"counts\":@@counts@@ {\"x\":1}";
    let mut req = request(ExportFormat::JsonPlan, standard_ops());
    req.label = label.into();
    let path = r.path("label.json");
    export(&mut r, 5, req, &path).await;
    let json: serde_json::Value = serde_json::from_str(&text(&path)).unwrap();
    assert_eq!(json["header"]["label"], label);
    assert_eq!(json["header"]["complete"], true);
    assert_eq!(json["header"]["counts"]["insert"], 2);
}
