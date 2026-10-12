//! Environment gates. Each prints the reason when it says no.

pub const ENV_IT: &str = "CURLYBRACE_IT";
/// Overrides the local server the tier-1 tests connect to.
pub const ENV_LOCAL_URI: &str = "CURLYBRACE_IT_MONGODB_URI";
/// Where tier-1 tests connect when [`ENV_LOCAL_URI`] is unset.
pub const DEFAULT_LOCAL_URI: &str = "mongodb://127.0.0.1:27017";
pub const ENV_SNAPSHOTS: &str = "CURLYBRACE_SNAPSHOTS";

/// Pure gate decision: `value` is the raw env value; only `"1"` enables.
pub fn enabled_by(value: Option<&str>) -> bool {
    value == Some("1")
}

fn flag(var: &str) -> bool {
    enabled_by(std::env::var(var).ok().as_deref())
}

fn gate(test: &str, what: &str) -> bool {
    if flag(ENV_IT) {
        return true;
    }
    eprintln!("SKIPPED {test}: set {ENV_IT}=1 to run {what}");
    false
}

/// Tier 1 gate: true when the local-server tests should run; otherwise prints the reason.
pub fn local_enabled(test: &str) -> bool {
    gate(
        test,
        "local-server tests (they need a running MongoDB; see scripts/integration-tests.sh)",
    )
}

/// Tier 2 gate: true when the docker tests should run; otherwise prints the reason.
pub fn docker_enabled(test: &str) -> bool {
    gate(
        test,
        "container-backed tests (they need docker; see scripts/container-tests.sh)",
    )
}

/// The local server for tier-1 tests: `CURLYBRACE_IT_MONGODB_URI` (empty counts as unset) or the
/// default `mongodb://127.0.0.1:27017`.
pub fn local_uri() -> String {
    std::env::var(ENV_LOCAL_URI)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_LOCAL_URI.to_string())
}

/// The failure text when the gate is on but no server answers. This is a failure, not a skip:
/// someone who asked for these tests wants to know they did not run.
pub fn unreachable_message(uri: &str, error: &str) -> String {
    format!(
        "no MongoDB server reachable at {uri} ({error}). The local-server tests need a server \
         you already started, for example `mongod --dbpath /tmp/msit-db` or \
         `docker run -d -p 27017:27017 mongo:7`. Set {ENV_LOCAL_URI} to use another address. \
         They write only to throwaway cbit_* databases: point them at a throwaway server."
    )
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
