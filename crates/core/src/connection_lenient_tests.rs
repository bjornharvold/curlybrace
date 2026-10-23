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
fn an_unknown_color_loads_as_none_and_warns_without_the_uri() {
    capture();
    let uri = "mongodb://secret-host-lenient-color";
    let mut json = base_json(uri);
    json["color"] = serde_json::json!("chartreuse");
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(c.color, None);
    let warnings = warnings_mentioning("colour");
    assert!(!warnings.is_empty(), "no warning for the bad colour");
    let all = RECORDS.lock().unwrap().join("\n");
    assert!(!all.contains("secret-host-lenient-color"), "{all}");
}

#[test]
fn a_known_color_and_a_missing_color_load_as_they_are() {
    let mut json = base_json("mongodb://h");
    json["color"] = serde_json::json!("Red");
    let c: ConnectionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(c.color, Some(ConnectionColor::Red));
    let c: ConnectionConfig = serde_json::from_value(base_json("mongodb://h")).unwrap();
    assert_eq!(c.color, None);
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
