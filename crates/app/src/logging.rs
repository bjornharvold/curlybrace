//! A tiny stderr logger, off unless `CURLYBRACE_LOG` names a level (`error`..`trace`).
//! Logged failure texts are secret-free by construction (see `curlybrace_ui::error_text`).
use log::{LevelFilter, Log, Metadata, Record};
use std::str::FromStr;

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

/// The level asked for by the environment value; unset or unparsable means logging stays off.
pub fn level_from_env(value: Option<&str>) -> LevelFilter {
    value
        .and_then(|v| LevelFilter::from_str(v.trim()).ok())
        .unwrap_or(LevelFilter::Off)
}

pub fn init() {
    let level = level_from_env(std::env::var("CURLYBRACE_LOG").ok().as_deref());
    if level != LevelFilter::Off && log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_is_off_unless_a_valid_level_is_given() {
        assert_eq!(level_from_env(None), LevelFilter::Off);
        assert_eq!(level_from_env(Some("nonsense")), LevelFilter::Off);
        assert_eq!(level_from_env(Some("warn")), LevelFilter::Warn);
        assert_eq!(level_from_env(Some(" debug ")), LevelFilter::Debug);
    }
}
