//! What a saved pipeline keeps of its options: the file spelling (MongoDB's names), the rules of
//! loading (unknown keys inside `options` are ignored, a known key with an invalid value refuses
//! the whole file and never repeats the value), and that the file format stays at version 1.
use super::*;
use crate::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};
use bson::doc;
use serde_json::{Value, json};

fn stage(text: &str) -> SavedStage {
    SavedStage {
        text: text.to_string(),
        enabled: true,
    }
}

fn stages() -> Vec<SavedStage> {
    vec![stage("{\"$match\": {}}")]
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Secondary),
            case_level: Some(true),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: Some(Alternate::Shifted),
        }),
        hint: Some(HintOption::Name("status_1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

fn entry(options: Option<Value>) -> Value {
    let mut entry = json!({
        "id": Uuid::new_v4(),
        "name": "totals",
        "saved_unix": 1,
        "stages": [{"text": "{\"$match\": {}}", "enabled": true}],
    });
    if let Some(options) = options {
        entry["options"] = options;
    }
    entry
}

fn file(conn: Uuid, entry: Value) -> String {
    json!({"version": 1, "connections": {conn.to_string(): [entry]}}).to_string()
}

fn load(text: &str) -> Result<SavedPipelines, serde_json::Error> {
    serde_json::from_str(text)
}

fn written(store: &SavedPipelines) -> Value {
    serde_json::to_value(store).unwrap()
}

#[test]
fn an_entry_without_options_loads_with_none() {
    let conn = Uuid::new_v4();
    let store = load(&file(conn, entry(None))).unwrap();
    assert!(store.list(conn)[0].options.is_empty());
}

#[test]
fn options_are_written_in_mongodb_spelling() {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: stages(),
                options: every_option(),
            },
            1,
            false,
        )
        .unwrap();
    let value = written(&store);
    let options = &value["connections"][conn.to_string()][0]["options"];
    assert_eq!(
        *options,
        json!({
            "collation": {
                "locale": "en", "strength": 2, "caseLevel": true, "caseFirst": "upper",
                "numericOrdering": true, "alternate": "shifted"
            },
            "hint": {"name": "status_1"},
            "allowDiskUse": true,
            "maxTimeMS": 5000,
            "comment": "nightly",
            "readPreference": "secondaryPreferred",
        })
    );
    let keyed = PipelineOptions {
        hint: Some(HintOption::Keys(doc! { "status": 1, "created": -1 })),
        ..PipelineOptions::default()
    };
    let store = SavedPipelines::default()
        .saved(
            conn,
            "keys",
            None,
            SavedDraft {
                stages: stages(),
                options: keyed.clone(),
            },
            1,
            false,
        )
        .unwrap();
    let value = written(&store);
    assert_eq!(
        value["connections"][conn.to_string()][0]["options"]["hint"],
        json!({"keys": {"status": 1, "created": -1}})
    );
    let back: SavedPipelines = serde_json::from_value(value).unwrap();
    assert_eq!(back.list(conn)[0].options, keyed);
}

#[test]
fn an_entry_without_options_writes_no_options_key() {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(
            conn,
            "plain",
            None,
            SavedDraft::stages_only(stages()),
            1,
            false,
        )
        .unwrap();
    let value = written(&store);
    let entry = value["connections"][conn.to_string()][0]
        .as_object()
        .unwrap();
    assert!(!entry.contains_key("options"));
}

#[test]
fn unknown_keys_inside_options_are_ignored() {
    let conn = Uuid::new_v4();
    let options = json!({"comment": "kept", "somethingNewer": {"x": 1}});
    let store = load(&file(conn, entry(Some(options)))).unwrap();
    let loaded = &store.list(conn)[0].options;
    assert_eq!(loaded.comment.as_deref(), Some("kept"));
    assert_eq!(loaded.set_count(), 1);
}

#[test]
fn a_key_pattern_written_with_doubles_loads_as_the_integers_the_form_reads_back() {
    let conn = Uuid::new_v4();
    let options = json!({"hint": {"keys": {"a": 1.0, "b": -1.0}}});
    let store = load(&file(conn, entry(Some(options)))).unwrap();
    let loaded = store.list(conn)[0].options.clone();
    assert_eq!(
        loaded.hint,
        Some(HintOption::Keys(doc! { "a": 1, "b": -1 })),
        "a whole-number direction is the same hint however the file spells it"
    );
}

#[test]
fn an_invalid_option_refuses_the_whole_file() {
    let conn = Uuid::new_v4();
    let bad = [
        json!({"collation": {"locale": "en", "strength": 9}}),
        json!({"maxTimeMS": 0}),
        json!({"readPreference": "tagged"}),
        json!({"hint": {"keys": {}}}),
        json!({"collation": {"locale": "en US"}}),
        json!({"comment": "   "}),
    ];
    for options in bad {
        assert!(
            load(&file(conn, entry(Some(options.clone())))).is_err(),
            "{options}"
        );
    }
}

#[test]
fn the_refusal_holds_no_value() {
    let conn = Uuid::new_v4();
    let secrets = [
        (json!({"readPreference": "tagged-secret"}), "tagged-secret"),
        (json!({"comment": "   "}), "   "),
        (json!({"collation": {"locale": "xx yy zz"}}), "xx yy zz"),
        (json!({"hint": {"name": "  "}}), "  \""),
    ];
    for (options, value) in secrets {
        let error = load(&file(conn, entry(Some(options))))
            .unwrap_err()
            .to_string();
        assert!(!error.contains(value), "{error}");
    }
}

#[test]
fn a_file_with_options_still_says_version_one_and_reads_without_them() {
    #[derive(Deserialize)]
    struct OldEntry {
        name: String,
        stages: Vec<SavedStage>,
    }
    #[derive(Deserialize)]
    struct OldFile {
        version: u32,
        connections: BTreeMap<Uuid, Vec<OldEntry>>,
    }
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: stages(),
                options: every_option(),
            },
            1,
            false,
        )
        .unwrap();
    let text = serde_json::to_string(&store).unwrap();
    let old: OldFile = serde_json::from_str(&text).expect("a build without options reads it");
    assert_eq!(old.version, 1);
    assert_eq!(old.connections[&conn][0].name, "totals");
    assert_eq!(old.connections[&conn][0].stages, stages());
}

#[test]
fn saved_keeps_the_options_and_replace_overwrites_them() {
    let conn = Uuid::new_v4();
    let first = SavedPipelines::default()
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: stages(),
                options: every_option(),
            },
            1,
            false,
        )
        .unwrap();
    assert_eq!(first.list(conn)[0].options, every_option());
    let replaced = first
        .saved(
            conn,
            "Totals",
            None,
            SavedDraft::stages_only(stages()),
            2,
            true,
        )
        .unwrap();
    assert!(
        replaced.list(conn)[0].options.is_empty(),
        "the new content wins"
    );
    let again = replaced
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: stages(),
                options: PipelineOptions {
                    comment: Some("later".into()),
                    ..PipelineOptions::default()
                },
            },
            3,
            true,
        )
        .unwrap();
    assert_eq!(
        again.list(conn)[0].options.comment.as_deref(),
        Some("later")
    );
    assert_eq!(again.list(conn).len(), 1);
}

#[test]
fn stages_only_saves_with_no_options() {
    let draft = SavedDraft::stages_only(stages());
    assert_eq!(draft.stages, stages());
    assert!(draft.options.is_empty());
}

#[test]
fn a_copied_connection_keeps_the_options() {
    let (from, to) = (Uuid::new_v4(), Uuid::new_v4());
    let store = SavedPipelines::default()
        .saved(
            from,
            "totals",
            None,
            SavedDraft {
                stages: stages(),
                options: every_option(),
            },
            1,
            false,
        )
        .unwrap()
        .with_connection_copied(from, to);
    assert_eq!(store.list(to)[0].options, every_option());
}
