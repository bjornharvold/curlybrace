//! One log capture for every unit test of this crate: a process has a single global logger, so
//! tests that look at what was logged share it and filter the records by their own words.
//! (A small copy of the core crate's private one.)
use std::sync::{Mutex, Once};

/// Every record logged since [`capture`] first ran, as `LEVEL text`.
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

/// Installs the capture (once per process).
pub(crate) fn capture() {
    INSTALL.call_once(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// The captured records that contain `word`.
pub(crate) fn records_mentioning(word: &str) -> Vec<String> {
    RECORDS
        .lock()
        .map(|records| {
            records
                .iter()
                .filter(|r| r.contains(word))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}
