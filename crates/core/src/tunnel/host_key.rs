//! Host-key policy for the SSH tunnel. Pure and synchronous so it can be
//! unit-tested with a temporary `known_hosts` file and a fake prompt.

use std::path::Path;

use russh::keys::{HashAlg, PublicKey, known_hosts};

use crate::error::{CoreError, Result};

/// What the user decided about an unknown host key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Accept for this connection only; nothing is written.
    Trust,
    /// Accept and record the key in the app-owned `known_hosts`.
    TrustAndRemember,
    Reject,
}

/// Asks whether an UNKNOWN host key may be trusted.
///
/// The trait is synchronous: the runtime calls it through `spawn_blocking`,
/// so a UI implementation may block on a channel until the user answers.
/// It is never consulted for a key that CHANGED.
pub trait HostKeyPrompt: Send + Sync {
    /// `fingerprint` is the `SHA256:...` string only.
    fn decide(&self, host: &str, fingerprint: &str) -> HostKeyDecision;
}

/// Safe default: unknown hosts are rejected (known hosts still connect).
/// The UI (Task 16) supplies an interactive prompt instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct RejectUnknownHostKeys;

impl HostKeyPrompt for RejectUnknownHostKeys {
    fn decide(&self, _host: &str, _fingerprint: &str) -> HostKeyDecision {
        HostKeyDecision::Reject
    }
}

pub(crate) fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Display form of an endpoint: `host`, or `[host]:port` off the default port.
pub(crate) fn display_host(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_ascii_lowercase()
    } else {
        format!("[{}]:{port}", host.to_ascii_lowercase())
    }
}

/// Verifies `key` for `host:port` against `known_hosts`.
///
/// * recorded and equal: `Ok`, no prompt
/// * recorded but different (any algorithm): `Err`, the prompt is NOT called
/// * not recorded (or the file is missing): ask `prompt`
/// * a malformed or unreadable file: `Err` (fails closed)
///
/// Host names are compared case-insensitively (lowercased before lookup and
/// before being recorded).
pub fn check_host_key(
    known_hosts: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
    prompt: &dyn HostKeyPrompt,
) -> Result<()> {
    let host = host.to_ascii_lowercase();
    let recorded = recorded_keys(known_hosts, &host, port)?;
    if recorded.iter().any(|k| k == key) {
        return Ok(());
    }
    if !recorded.is_empty() {
        // Never offer to trust a key that differs from a recorded one.
        return Err(CoreError::SshHostKeyChanged);
    }
    match prompt.decide(&display_host(&host, port), &fingerprint(key)) {
        HostKeyDecision::Trust => Ok(()),
        HostKeyDecision::TrustAndRemember => remember(known_hosts, &host, port, key),
        HostKeyDecision::Reject => Err(CoreError::Ssh("the ssh host key was rejected".into())),
    }
}

/// Keys recorded for the endpoint. A missing file means "no known hosts";
/// any other read problem or a malformed matching line is an error.
fn recorded_keys(known_hosts: &Path, host: &str, port: u16) -> Result<Vec<PublicKey>> {
    match std::fs::File::open(known_hosts) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unreadable()),
    }
    known_hosts::known_host_keys_path(host, port, known_hosts)
        .map(|found| found.into_iter().map(|(_, key)| key).collect())
        .map_err(|_| unreadable())
}

fn unreadable() -> CoreError {
    CoreError::Ssh("the known hosts file could not be read".into())
}

/// Appends the key, creating the file (mode 0600) and its parent if needed.
fn remember(known_hosts: &Path, host: &str, port: u16, key: &PublicKey) -> Result<()> {
    let fail = || CoreError::Ssh("the ssh host key could not be saved".into());
    if let Some(parent) = known_hosts.parent() {
        std::fs::create_dir_all(parent).map_err(|_| fail())?;
    }
    create_private_if_missing(known_hosts).map_err(|_| fail())?;
    known_hosts::learn_known_hosts_path(host, port, key, known_hosts).map_err(|_| fail())
}

fn create_private_if_missing(path: &Path) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use russh::keys::ssh_key::{PrivateKey, private::Ed25519Keypair};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU8, Ordering};

    struct FakePrompt {
        answer: HostKeyDecision,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl FakePrompt {
        fn new(answer: HostKeyDecision) -> Self {
            Self {
                answer,
                calls: Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl HostKeyPrompt for FakePrompt {
        fn decide(&self, host: &str, fingerprint: &str) -> HostKeyDecision {
            self.calls
                .lock()
                .unwrap()
                .push((host.to_string(), fingerprint.to_string()));
            self.answer
        }
    }

    fn new_key() -> PublicKey {
        // Distinct deterministic key per call; no RNG needed in tests.
        static NEXT: AtomicU8 = AtomicU8::new(1);
        let seed = [NEXT.fetch_add(1, Ordering::SeqCst); 32];
        PrivateKey::from(Ed25519Keypair::from_seed(&seed))
            .public_key()
            .clone()
    }

    fn is_ssh_err<T: std::fmt::Debug>(r: Result<T>) -> bool {
        matches!(r, Err(CoreError::Ssh(_)))
    }

    #[test]
    fn unknown_trust_is_ok_and_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let prompt = FakePrompt::new(HostKeyDecision::Trust);
        check_host_key(&path, "h.example", 22, &new_key(), &prompt).unwrap();
        assert_eq!(prompt.call_count(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn trust_does_not_modify_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let other = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "a", 22, &new_key(), &other).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let trust = FakePrompt::new(HostKeyDecision::Trust);
        check_host_key(&path, "b", 22, &new_key(), &trust).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn prompt_receives_sha256_fingerprint_only() {
        let dir = tempfile::tempdir().unwrap();
        let key = new_key();
        let prompt = FakePrompt::new(HostKeyDecision::Trust);
        check_host_key(&dir.path().join("kh"), "H.Example", 2222, &key, &prompt).unwrap();
        let calls = prompt.calls.lock().unwrap();
        assert_eq!(calls[0].0, "[h.example]:2222");
        assert!(calls[0].1.starts_with("SHA256:"));
        assert_eq!(calls[0].1, fingerprint(&key));
    }

    #[test]
    fn remember_creates_file_then_known_host_needs_no_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/dir/known_hosts");
        let key = new_key();
        let remember = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "h.example", 2222, &key, &remember).unwrap();
        let reject = FakePrompt::new(HostKeyDecision::Reject);
        check_host_key(&path, "h.example", 2222, &key, &reject).unwrap();
        assert_eq!(reject.call_count(), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[h.example]:2222"));
    }

    #[cfg(unix)]
    #[test]
    fn created_known_hosts_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/known_hosts");
        let p = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "h", 22, &new_key(), &p).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn hostnames_match_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key = new_key();
        let p = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "Host.EXAMPLE", 22, &key, &p).unwrap();
        let none = FakePrompt::new(HostKeyDecision::Reject);
        check_host_key(&path, "host.example", 22, &key, &none).unwrap();
        assert_eq!(none.call_count(), 0);
    }

    #[test]
    fn unknown_reject_is_ssh_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = FakePrompt::new(HostKeyDecision::Reject);
        let r = check_host_key(&dir.path().join("kh"), "h", 22, &new_key(), &p);
        assert!(is_ssh_err(r));
    }

    #[test]
    fn changed_key_fails_without_prompting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let remember = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "h.example", 22, &new_key(), &remember).unwrap();
        let trusting = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        let r = check_host_key(&path, "h.example", 22, &new_key(), &trusting);
        assert_eq!(r, Err(CoreError::SshHostKeyChanged));
        assert_eq!(trusting.call_count(), 0);
    }

    #[test]
    fn same_host_other_port_is_a_different_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let p = FakePrompt::new(HostKeyDecision::TrustAndRemember);
        check_host_key(&path, "h", 22, &new_key(), &p).unwrap();
        let other = FakePrompt::new(HostKeyDecision::Trust);
        check_host_key(&path, "h", 2222, &new_key(), &other).unwrap();
        assert_eq!(other.call_count(), 1);
    }

    #[test]
    fn malformed_line_for_the_host_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        std::fs::write(&path, "h.example ssh-ed25519 !!!not-base64!!!\n").unwrap();
        let p = FakePrompt::new(HostKeyDecision::Trust);
        let r = check_host_key(&path, "h.example", 22, &new_key(), &p);
        assert!(is_ssh_err(r));
        assert_eq!(p.call_count(), 0);
    }

    #[test]
    fn unreadable_known_hosts_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        // A directory is not a readable known_hosts file.
        let p = FakePrompt::new(HostKeyDecision::Trust);
        let r = check_host_key(dir.path(), "h", 22, &new_key(), &p);
        assert!(is_ssh_err(r));
        assert_eq!(p.call_count(), 0);
    }

    #[test]
    fn reject_unknown_default_rejects() {
        assert_eq!(
            RejectUnknownHostKeys.decide("h", "SHA256:x"),
            HostKeyDecision::Reject
        );
    }
}
