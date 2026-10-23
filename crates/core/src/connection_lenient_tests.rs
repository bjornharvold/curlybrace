//! Lenient loading of the colour tag and the production flag: a bad value never drops the
//! entry, never hides silently, and never fails open for the production flag.
use super::*;
use std::sync::{Mutex, Once};

static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if let Ok(mut records) = RECORDS.lock() {
            records.push(format!("{} {}", record.level(), record.args()));
        }
    }
    fn flush(&self) {}
}

fn capture() {
    INSTALL.call_once(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Warn);
    });
}

fn warnings_mentioning(word: &str) -> Vec<String> {
    RECORDS
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.starts_with("WARN") && r.contains(word))
        .cloned()
        .collect()
}

fn base_json(uri: &str) -> serde_json::Value {
    let mut v = serde_json::to_value(ConnectionConfig::new("a", uri)).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("color");
    o.remove("production");
    v
}

#[test]
fn an_unknown_color_is_kept_and_warns_without_the_uri_or_the_value() {
    capture();
    let uri = "mongodb://secret-host-lenient-color";
    let mut json = base_json(uri);
    json["color"] = serde_json::json!("chartreuse-secret");
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(
        c.color,
        Some(SavedColor::Unrecognised("chartreuse-secret".into()))
    );
    assert_eq!(c.known_color(), None);
    assert!(
        !warnings_mentioning("colour").is_empty(),
        "no warning for the bad colour"
    );
    let all = RECORDS.lock().unwrap().join("\n");
    assert!(!all.contains("secret-host-lenient-color"), "{all}");
    assert!(!all.contains("chartreuse-secret"), "{all}");
}

#[test]
fn an_unknown_color_round_trips_load_to_save_unchanged() {
    for raw in ["chartreuse", "red", "", "Red "] {
        let mut json = base_json("mongodb://h");
        json["color"] = serde_json::json!(raw);
        let c: ConnectionConfig = serde_json::from_value(json).unwrap();
        let saved = serde_json::to_value(&c).unwrap();
        assert_eq!(saved["color"], serde_json::json!(raw));
    }
}

#[test]
fn a_known_color_serialises_as_its_variant_name() {
    for color in ConnectionColor::ALL {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.color = Some(SavedColor::Known(color));
        let saved = serde_json::to_value(&c).unwrap();
        assert_eq!(saved["color"], serde_json::to_value(color).unwrap());
        assert_eq!(c.known_color(), Some(color));
    }
}

#[test]
fn a_known_a_null_and_a_missing_color_load_as_they_are() {
    let mut json = base_json("mongodb://h");
    json["color"] = serde_json::json!("Red");
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(c.color, Some(SavedColor::Known(ConnectionColor::Red)));
    let mut json = base_json("mongodb://h");
    json["color"] = serde_json::json!(null);
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(c.color, None);
    let c: ConnectionConfig = serde_json::from_value(base_json("mongodb://h")).unwrap();
    assert_eq!(c.color, None);
}

#[test]
fn one_bad_color_does_not_stop_the_other_entries_loading() {
    let mut bad = base_json("mongodb://bad");
    bad["color"] = serde_json::json!(42);
    let mut odd = base_json("mongodb://odd");
    odd["color"] = serde_json::json!("future-shade");
    let mut good = base_json("mongodb://good");
    good["color"] = serde_json::json!("Blue");
    let list: Vec<ConnectionConfig> =
        serde_json::from_value(serde_json::json!([bad, odd, good])).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].color, Some(SavedColor::Unrecognised(42.into())));
    assert_eq!(
        list[1].color,
        Some(SavedColor::Unrecognised("future-shade".into()))
    );
    assert_eq!(list[2].known_color(), Some(ConnectionColor::Blue));
}

#[test]
fn a_malformed_production_value_loads_as_production_and_warns() {
    capture();
    for bad in [
        serde_json::json!("yes"),
        serde_json::json!(1),
        serde_json::json!(null),
        serde_json::json!({"x": 1}),
    ] {
        let mut json = base_json("mongodb://secret-host-lenient-production");
        json["production"] = bad.clone();
        let c: ConnectionConfig = serde_json::from_value(json).expect("the entry still loads");
        assert!(c.production, "{bad} must fail safe to production");
        assert_eq!(c.name, "a");
    }
    assert!(!warnings_mentioning("production").is_empty());
    let all = RECORDS.lock().unwrap().join("\n");
    assert!(!all.contains("secret-host-lenient-production"), "{all}");
}

#[test]
fn a_boolean_production_value_is_kept_as_it_is() {
    for flag in [true, false] {
        let mut json = base_json("mongodb://h");
        json["production"] = serde_json::json!(flag);
        let c: ConnectionConfig = serde_json::from_value(json).unwrap();
        assert_eq!(c.production, flag);
    }
    let c: ConnectionConfig = serde_json::from_value(base_json("mongodb://h")).unwrap();
    assert!(!c.production, "an absent flag stays non-production");
}

#[test]
fn a_duplicate_keeps_an_unrecognised_color() {
    let mut src = ConnectionConfig::new("a", "mongodb://h");
    src.color = Some(SavedColor::Unrecognised("future-shade".into()));
    let copy = src.duplicate("a copy".into(), 5);
    assert_eq!(copy.color, src.color);
    assert_eq!(copy.known_color(), None);
}

#[test]
fn any_non_null_colour_value_is_written_back_exactly() {
    let values = [
        serde_json::json!({"hue": 12, "name": "teal"}),
        serde_json::json!(42),
        serde_json::json!(["Red", "Blue"]),
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(""),
        serde_json::json!("red"),
        serde_json::json!("future-shade"),
    ];
    for value in values {
        let mut json = base_json("mongodb://round-trip");
        json["color"] = value.clone();
        let c: ConnectionConfig = serde_json::from_value(json).unwrap();
        assert!(c.color.is_some(), "{value} must be kept");
        assert_eq!(c.known_color(), None, "{value} is not a preset");
        let saved = serde_json::to_value(&c).unwrap();
        assert_eq!(saved["color"], value, "{value} must round-trip");
    }
}

#[test]
fn a_null_or_missing_colour_stays_none() {
    let mut json = base_json("mongodb://none");
    assert_eq!(
        serde_json::from_value::<ConnectionConfig>(json.clone())
            .unwrap()
            .color,
        None
    );
    json["color"] = serde_json::Value::Null;
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(c.color, None);
}

#[test]
fn a_structured_colour_warns_without_naming_the_value() {
    capture();
    let mut json = base_json("mongodb://structured");
    json["color"] = serde_json::json!({"secret-key": "secret-structured-value"});
    let _: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert!(warnings_mentioning("secret-structured-value").is_empty());
    assert!(warnings_mentioning("secret-key").is_empty());
}
