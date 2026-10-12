//! Environment gates. Each prints the reason when it says no.

pub const ENV_IT: &str = "MONGOSTUDIO_IT";
pub const ENV_EXTERNAL_URI: &str = "MONGOSTUDIO_IT_MONGO_URI";
pub const ENV_SNAPSHOTS: &str = "MONGOSTUDIO_SNAPSHOTS";

/// Pure gate decision: `value` is the raw env value; only `"1"` enables.
pub fn enabled_by(value: Option<&str>) -> bool {
    value == Some("1")
}

fn flag(var: &str) -> bool {
    enabled_by(std::env::var(var).ok().as_deref())
}

/// True when container/server backed tests should run; otherwise prints the reason.
pub fn it_enabled(test: &str) -> bool {
    if flag(ENV_IT) {
        return true;
    }
    eprintln!("SKIPPED {test}: set {ENV_IT}=1 to run container-backed tests");
    false
}

/// The external server URI, if one is configured (empty counts as unset).
pub fn external_uri() -> Option<String> {
    std::env::var(ENV_EXTERNAL_URI)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Like [`it_enabled`] for tests that must control the server (stop/start, `--auth`): they need
/// docker, so they also skip, with a printed reason, when an external server is configured.
pub fn it_enabled_with_docker(test: &str) -> bool {
    if !it_enabled(test) {
        return false;
    }
    if external_uri().is_some() {
        eprintln!("SKIPPED {test}: needs docker container control, but {ENV_EXTERNAL_URI} is set");
        return false;
    }
    true
}

/// True when the GPU screenshot tests should run; otherwise prints the reason.
pub fn snapshots_enabled(test: &str) -> bool {
    if flag(ENV_SNAPSHOTS) {
        return true;
    }
    eprintln!("SKIPPED {test}: set {ENV_SNAPSHOTS}=1 to run screenshot tests");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_literal_one_enables() {
        assert!(enabled_by(Some("1")));
        for no in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!enabled_by(no), "{no:?}");
        }
    }
}
