use super::*;
use crate::i18n::I18n;
use bson::doc;
use curlybrace_core::pipeline_options::CollationOption;
use curlybrace_core::query_options::QueryOptions;
use std::path::PathBuf;
use uuid::Uuid;

fn source() -> DocExportSource {
    DocExportSource {
        session: Uuid::nil(),
        ns: Namespace::new("secretdb", "secretcoll"),
        filter: Some(doc! { "secretfield": "secretvalue" }),
    }
}

fn running() -> DocExportUi {
    DocExportUi::default().open(source()).choosing().started(
        7,
        PathBuf::from("/home/someone/out.ndjson"),
        Some(10),
    )
}

#[test]
fn default_draft_is_ndjson_canonical() {
    let draft = DocExportDraft::default();
    assert_eq!(draft.format, DocFormat::Ndjson);
    assert_eq!(draft.mode, JsonMode::Canonical);
    assert_eq!(DocExportUi::default(), DocExportUi::Idle);
}

#[test]
fn happy_path_runs_to_finished() {
    let state = running();
    assert!(state.is_busy());
    let state = state.progressed(7, 4, 10);
    assert!(matches!(
        state,
        DocExportUi::Running {
            done: 4,
            total: Some(10),
            ..
        }
    ));
    let outcome = DocExportOutcome {
        written: 10,
        ..DocExportOutcome::default()
    };
    let state = state.finished(7, Ok(outcome));
    assert!(!state.is_busy());
    assert!(
        matches!(state, DocExportUi::Finished { outcome: Ok(o), path: Some(_) } if o.written == 10)
    );
}

#[test]
fn edits_apply_only_while_configuring() {
    let state = DocExportUi::Idle.open(source());
    let edited = state.clone().edited(|d| DocExportDraft {
        format: DocFormat::JsonArray,
        ..d
    });
    assert!(
        matches!(&edited, DocExportUi::Configuring { draft, .. } if draft.format == DocFormat::JsonArray)
    );
    let locked = state.choosing();
    assert_eq!(locked.clone().edited(|_| DocExportDraft::default()), locked);
}

#[test]
fn stale_run_changes_nothing() {
    let state = running();
    assert_eq!(state.clone().progressed(8, 1, 2), state);
    assert_eq!(
        state.clone().finished(8, Ok(DocExportOutcome::default())),
        state
    );
    assert_eq!(state.clone().cancelling().cancelling(), state.cancelling());
}

#[test]
fn closed_is_ignored_while_busy() {
    let choosing = DocExportUi::Idle.open(source()).choosing();
    assert_eq!(choosing.clone().closed(), choosing);
    let state = running();
    assert_eq!(state.clone().closed(), state);
    let done = state.finished(7, Ok(DocExportOutcome::default()));
    assert_eq!(done.closed(), DocExportUi::Idle);
    assert_eq!(DocExportUi::Idle.open(source()).closed(), DocExportUi::Idle);
}

#[test]
fn done_never_exceeds_a_known_total() {
    let state = running().progressed(7, 99, 10);
    assert!(matches!(
        state,
        DocExportUi::Running {
            done: 10,
            total: Some(10),
            ..
        }
    ));
    let unknown = DocExportUi::Idle
        .open(source())
        .started(1, PathBuf::from("/o"), None)
        .progressed(1, 5, 0);
    assert!(matches!(
        unknown,
        DocExportUi::Running {
            done: 5,
            total: None,
            ..
        }
    ));
}

#[test]
fn debug_hides_path_filter_and_namespace() {
    let states = [
        DocExportUi::Idle.open(source()),
        DocExportUi::Idle.open(source()).choosing(),
        running(),
        running().finished(7, Ok(DocExportOutcome::default())),
        running().finished(7, Err(CoreError::BulkCancelled)),
    ];
    for state in states {
        let text = format!("{state:?}");
        for secret in [
            "secretdb",
            "secretcoll",
            "secretfield",
            "secretvalue",
            "someone",
        ] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    assert!(!format!("{:?}", source()).contains("secret"));
}

#[test]
fn block_order_and_collation() {
    let plain = Query::default();
    let collated = Query {
        options: QueryOptions {
            collation: Some(CollationOption {
                locale: "en".into(),
                strength: None,
                case_level: None,
                case_first: None,
                numeric_ordering: None,
                alternate: None,
            }),
            ..QueryOptions::default()
        },
        ..Query::default()
    };
    assert_eq!(
        doc_export_block(false, &collated, true),
        Some(DocExportBlock::NotConnected)
    );
    assert_eq!(
        doc_export_block(true, &collated, true),
        Some(DocExportBlock::Collated)
    );
    assert_eq!(
        doc_export_block(true, &plain, true),
        Some(DocExportBlock::Busy)
    );
    assert_eq!(doc_export_block(true, &plain, false), None);
}

#[test]
fn every_block_key_exists() {
    let i18n = I18n::english();
    for block in [
        DocExportBlock::NotConnected,
        DocExportBlock::Collated,
        DocExportBlock::Busy,
    ] {
        assert!(
            !i18n.t(block.i18n_key()).starts_with('\u{27e6}'),
            "{block:?}"
        );
    }
}
