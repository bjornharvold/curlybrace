//! Rows of the Queries view and the consent text.
use super::*;
use curlybrace_core::explain::{VerdictKind, js_string};
use curlybrace_core::history::{ShapeSource, ShapeSummary};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{Namespace, Query};

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn shape_json(field: &str) -> String {
    from_query(&Query {
        filter: bson::doc! { field: 1 },
        ..Query::default()
    })
    .json()
}

fn summary(field: &str, worst: VerdictKind, misses: u64, max_millis: u64) -> ShapeSummary {
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: shape_json(field),
        source: ShapeSource::App,
        executions: 10,
        misses,
        total_millis: max_millis * 2,
        max_millis,
        worst,
        buckets: Vec::new(),
    }
}

fn state_of(shapes: Vec<ShapeSummary>) -> QueriesState {
    QueriesState::default().with_shapes(shapes)
}

fn fields(rows: &[QueryRow]) -> Vec<String> {
    rows.iter().map(|r| r.shape_json.clone()).collect()
}

#[test]
fn rows_sort_misses_first() {
    let state = state_of(vec![
        summary("good", VerdictKind::IndexUsed, 0, 900),
        summary("sort", VerdictKind::SortNotCovered, 50, 10),
        summary("few", VerdictKind::CollectionScan, 2, 10),
        summary("many", VerdictKind::CollectionScan, 9, 10),
    ]);
    // Worst severity first (scan, sort, used), then more misses within a severity.
    let order = fields(&rows(&state, &ns()));
    assert_eq!(
        order,
        ["many", "few", "sort", "good"].map(shape_json).to_vec()
    );
}

#[test]
fn ties_break_on_misses_then_slowest_time() {
    let state = state_of(vec![
        summary("a", VerdictKind::CollectionScan, 3, 10),
        summary("b", VerdictKind::CollectionScan, 3, 500),
        summary("c", VerdictKind::CollectionScan, 4, 1),
    ]);
    let order = fields(&rows(&state, &ns()));
    assert_eq!(order, ["c", "b", "a"].map(shape_json).to_vec());
}

#[test]
fn rows_have_a_suggestion_only_for_misses() {
    let state = state_of(vec![
        summary("email", VerdictKind::CollectionScan, 3, 10),
        summary("fine", VerdictKind::IndexUsed, 0, 10),
        summary("nofilter", VerdictKind::NoFilter, 0, 10),
    ]);
    let rows = rows(&state, &ns());
    let find = |f: &str| rows.iter().find(|r| r.shape_json == shape_json(f)).unwrap();
    let suggestion = find("email").suggestion.clone().expect("a miss is advised");
    assert!(
        suggestion.contains(r#"getSiblingDB("shop")"#),
        "{suggestion}"
    );
    assert!(suggestion.contains(r#""email": 1"#), "{suggestion}");
    assert_eq!(find("fine").suggestion, None);
    assert_eq!(find("nofilter").suggestion, None);
}

#[test]
fn rows_survive_an_unparseable_shape() {
    let broken = ShapeSummary {
        shape_json: "not json".into(),
        ..summary("x", VerdictKind::CollectionScan, 5, 5)
    };
    let state = state_of(vec![
        broken,
        summary("ok", VerdictKind::CollectionScan, 1, 1),
    ]);
    let rows = rows(&state, &ns());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].shape_json, "not json");
    assert_eq!(rows[0].suggestion, None);
    assert!(rows[1].suggestion.is_some());
}

#[test]
fn consent_command_text_matches_the_exact_form() {
    let on = ProfilerConsent {
        db: "shop".into(),
        level: 2,
    };
    assert_eq!(
        on.command_text(),
        r#"db.getSiblingDB("shop").setProfilingLevel(2)"#
    );
    let off = ProfilerConsent {
        db: "shop".into(),
        level: 0,
    };
    assert_eq!(
        off.command_text(),
        r#"db.getSiblingDB("shop").setProfilingLevel(0)"#
    );
}

#[test]
fn consent_command_text_escapes_the_database_name() {
    for name in [
        "a\"b",
        "x\")); db.dropDatabase(); (\"",
        "semi;colon",
        "line\u{2028}sep\u{2029}",
        "back\\slash",
    ] {
        let text = ProfilerConsent {
            db: name.into(),
            level: 2,
        }
        .command_text();
        assert_eq!(
            text,
            format!("db.getSiblingDB({}).setProfilingLevel(2)", js_string(name))
        );
        assert!(!text.contains('\u{2028}') && !text.contains('\u{2029}'));
    }
}

#[test]
fn a_new_range_drops_the_shapes_read_for_the_old_one() {
    let state = state_of(vec![summary("a", VerdictKind::CollectionScan, 1, 1)])
        .with_range(curlybrace_core::history::ChartRange::Week);
    assert!(state.shapes.is_empty());
}

#[test]
fn no_status_yet_reads_as_idle() {
    use curlybrace_core::runtime::ProfilerStatus;
    let state = QueriesState::default();
    assert_eq!(state.profiler_status(), ProfilerStatus::Idle);
    let state = state.with_status(ProfilerStatus::Failing);
    assert_eq!(state.profiler_status(), ProfilerStatus::Failing);
}

#[test]
fn rows_survive_a_non_canonical_stored_shape() {
    // Valid JSON, but carrying a literal value: not something the recorder writes.
    let odd = ShapeSummary {
        shape_json: r#"{"filter":{"email":"a@b.c"}}"#.into(),
        ..summary("x", VerdictKind::CollectionScan, 5, 5)
    };
    let rows = rows(&state_of(vec![odd]), &ns());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].suggestion, None);
}

#[test]
fn consent_command_text_for_a_nasty_name_is_exactly_this() {
    let text = ProfilerConsent {
        db: "a\"b)\n".into(),
        level: 2,
    }
    .command_text();
    assert_eq!(text, r#"db.getSiblingDB("a\"b)\n").setProfilingLevel(2)"#);
}
