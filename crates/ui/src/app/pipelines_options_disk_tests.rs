//! Saved pipelines keep their options on disk: a restart reads them back, and a file with an
//! option that is not valid is treated like a corrupt file (backed up, the list starts empty,
//! the recovery banner says so). Every file lives in a tempdir.
use super::persistence::{self, Recovery};
use super::pipelines_store::load;
use super::pipelines_wiring_tests::{app_in, pipeline_file, stage};
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use curlybrace_core::saved_pipeline::SavedDraft;
use uuid::Uuid;

fn options() -> PipelineOptions {
    PipelineOptions {
        comment: Some("nightly".into()),
        max_time_ms: Some(2500),
        read_preference: Some(ReadPreferenceMode::Nearest),
        ..PipelineOptions::default()
    }
}

#[test]
fn options_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    let next = app
        .state()
        .pipelines
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: stage(),
                options: options(),
            },
            1,
            false,
        )
        .unwrap();
    app.state_mut().persist_pipelines(next).unwrap();
    let loaded = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(loaded.recovery, None);
    assert_eq!(loaded.settings.list(conn)[0].options, options());
}

#[test]
fn a_file_with_an_invalid_option_is_backed_up_and_the_list_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let text = serde_json::json!({
        "version": 1,
        "connections": {conn.to_string(): [{
            "id": Uuid::new_v4(),
            "name": "totals",
            "saved_unix": 1,
            "stages": [{"text": "{\"$match\": {}}", "enabled": true}],
            "options": {"maxTimeMS": 0},
        }]},
    })
    .to_string();
    std::fs::write(pipeline_file(&dir), &text).unwrap();
    let loaded = load(&pipeline_file(&dir), &persistence::back_up);
    assert!(loaded.settings.list(conn).is_empty());
    assert!(matches!(
        loaded.recovery,
        Some(Recovery::Pipelines { backed_up: true })
    ));
    let kept: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("pipelines.json."))
        })
        .collect();
    assert_eq!(kept.len(), 1, "a backup copy was kept");
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), text);
}
