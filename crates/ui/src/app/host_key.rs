//! Interactive SSH host-key prompt: a blocking `HostKeyPrompt` for the runtime thread, a pure
//! state machine for the UI side, and `forget_host` for a changed key.
use super::persistence::{PersistError, atomic_write};
use curlybrace_core::runtime::Repaint;
use curlybrace_core::tunnel::{HostKeyDecision, HostKeyPrompt};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the runtime thread waits for the user before failing closed.
pub const HOST_KEY_PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// How often a waiting question checks whether its job is still wanted.
const LIVENESS_SLICE: Duration = Duration::from_millis(50);

/// An unknown host key awaiting the user's decision. Answering consumes it, so a decision is
/// sent at most once; dropping it unanswered makes the waiting side fail closed (`Reject`).
#[derive(Debug)]
pub struct HostKeyRequest {
    pub host: String,
    pub fingerprint: String,
    reply: Sender<HostKeyDecision>,
    /// Set when the waiting side gave up (timeout) or went away.
    abandoned: Arc<AtomicBool>,
}

/// The waiting (runtime) side of a request. Dropping it marks the request abandoned so the UI
/// stops asking a question nobody is waiting for.
#[derive(Debug)]
pub struct Waiter {
    rx: Receiver<HostKeyDecision>,
    abandoned: Arc<AtomicBool>,
}

impl Waiter {
    pub fn try_recv(&self) -> Result<HostKeyDecision, std::sync::mpsc::TryRecvError> {
        self.rx.try_recv()
    }

    /// Waits for the answer; every failure to get one is a `Reject`.
    pub fn wait(&self, timeout: Duration) -> HostKeyDecision {
        match self.rx.recv_timeout(timeout) {
            Ok(decision) => decision,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                HostKeyDecision::Reject
            }
        }
    }
}

impl Waiter {
    /// Like [`wait`](Self::wait), but looks at `live` between short slices and gives up (`Reject`)
    /// as soon as it is false (`None`: nobody answered). A decision already waiting is returned.
    fn wait_while(&self, timeout: Duration, live: &dyn Fn() -> bool) -> Option<HostKeyDecision> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(LIVENESS_SLICE)) {
                Ok(decision) => return Some(decision),
                Err(RecvTimeoutError::Disconnected) => return None,
                Err(RecvTimeoutError::Timeout) if left.is_zero() || !live() => return None,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::SeqCst);
    }
}

impl HostKeyRequest {
    pub fn new(host: &str, fingerprint: &str) -> (Self, Waiter) {
        let (reply, rx) = channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        let request = Self {
            host: host.to_owned(),
            fingerprint: fingerprint.to_owned(),
            reply,
            abandoned: abandoned.clone(),
        };
        (request, Waiter { rx, abandoned })
    }

    fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::SeqCst)
    }

    fn answer(self, decision: HostKeyDecision) {
        // The waiter may already have timed out; there is nobody left to tell.
        let _ = self.reply.send(decision);
    }
}

/// Blocks the calling (blocking-pool) thread until the UI answers or the timeout elapses.
pub struct InteractivePrompt {
    requests: Mutex<Sender<HostKeyRequest>>,
    repaint: Repaint,
    timeout: Duration,
}

impl InteractivePrompt {
    /// The prompt plus the receiver the UI polls for requests.
    pub fn channel(repaint: Repaint, timeout: Duration) -> (Arc<Self>, Receiver<HostKeyRequest>) {
        let (tx, rx) = channel();
        let prompt = Arc::new(Self {
            requests: Mutex::new(tx),
            repaint,
            timeout,
        });
        (prompt, rx)
    }
}

impl HostKeyPrompt for InteractivePrompt {
    fn decide(&self, host: &str, fingerprint: &str) -> HostKeyDecision {
        self.ask(host, fingerprint, &|| true)
    }

    /// Withdraws the question (the waiter drops, so `PromptState::pruned` removes the dialog)
    /// within one slice of `live` turning false.
    fn decide_while(
        &self,
        host: &str,
        fingerprint: &str,
        live: &dyn Fn() -> bool,
    ) -> HostKeyDecision {
        self.ask(host, fingerprint, live)
    }
}

impl InteractivePrompt {
    fn ask(&self, host: &str, fingerprint: &str, live: &dyn Fn() -> bool) -> HostKeyDecision {
        let (request, reply) = HostKeyRequest::new(host, fingerprint);
        let sent = match self.requests.lock() {
            Ok(tx) => tx.send(request).is_ok(),
            Err(_) => false,
        };
        if !sent {
            return HostKeyDecision::Reject;
        }
        (self.repaint)();
        match reply.wait_while(self.timeout, live) {
            Some(decision) => decision,
            None => {
                // Release the waiter first so the next frame prunes the dialog, then wake the UI:
                // egui only repaints on input, so a withdrawn dialog would otherwise linger.
                drop(reply);
                (self.repaint)();
                HostKeyDecision::Reject
            }
        }
    }
}

/// Widget id of the dialog for one question: a different question gets different button ids, so
/// a press aimed at one dialog cannot register as a click on the next.
pub fn dialog_id(host: &str, fingerprint: &str) -> eframe::egui::Id {
    eframe::egui::Id::new(("host_key_prompt", host, fingerprint))
}

/// A decision together with the question it was made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyAnswer {
    pub host: String,
    pub fingerprint: String,
    pub decision: HostKeyDecision,
}

impl HostKeyAnswer {
    pub fn new(host: &str, fingerprint: &str, decision: HostKeyDecision) -> Self {
        Self {
            host: host.to_owned(),
            fingerprint: fingerprint.to_owned(),
            decision,
        }
    }
}

/// Pending prompts on the UI side: one is shown, the rest queue behind it.
#[derive(Debug, Default)]
pub struct PromptState {
    queue: VecDeque<HostKeyRequest>,
}

impl PromptState {
    pub fn offered(mut self, request: HostKeyRequest) -> Self {
        self.queue.push_back(request);
        self
    }

    /// Drops requests whose waiter has given up (timed out), so no dead question is shown.
    pub fn pruned(mut self) -> Self {
        self.queue.retain(|r| !r.is_abandoned());
        self
    }

    /// `(host, fingerprint)` of the prompt to show.
    pub fn current(&self) -> Option<(&str, &str)> {
        self.queue
            .front()
            .map(|r| (r.host.as_str(), r.fingerprint.as_str()))
    }

    /// Sends `decision` for the current prompt exactly once and moves on to the next.
    pub fn answered(mut self, decision: HostKeyDecision) -> Self {
        if let Some(request) = self.queue.pop_front() {
            request.answer(decision);
        }
        self
    }

    /// Like [`answered`](Self::answered), but only when the prompt in front is still the
    /// question `(host, fingerprint)` that was drawn; otherwise the answer is dropped, so a click
    /// aimed at a withdrawn question can never answer the one that moved up behind it.
    pub fn answered_for(self, host: &str, fingerprint: &str, decision: HostKeyDecision) -> Self {
        if self.current() == Some((host, fingerprint)) {
            self.answered(decision)
        } else {
            self
        }
    }

    /// Rejects every pending request for `host` (ASCII case-insensitive) and keeps the others,
    /// in order: the question of a session that closed has nobody left to answer for.
    pub fn rejected_for_host(self, host: &str) -> Self {
        let (closed, kept): (VecDeque<_>, VecDeque<_>) = self
            .queue
            .into_iter()
            .partition(|r| r.host.eq_ignore_ascii_case(host));
        for request in closed {
            request.answer(HostKeyDecision::Reject);
        }
        Self { queue: kept }
    }

    /// Rejects everything pending (new Connect/Disconnect, app exit).
    pub fn rejected_all(mut self) -> Self {
        while let Some(request) = self.queue.pop_front() {
            request.answer(HostKeyDecision::Reject);
        }
        self
    }
}

/// Removes the recorded keys of `host:port` from the app-owned known_hosts file
/// (atomic rewrite, mode 0600). Every other line is kept byte for byte (CRLF, whitespace,
/// comments, lines it cannot parse); a missing file is success. Only plain host patterns are
/// matched (the app never writes hashed ones).
pub fn forget_host(known_hosts: &Path, host: &str, port: u16) -> Result<(), PersistError> {
    if !known_hosts.is_absolute() {
        return Err(PersistError::InvalidPath);
    }
    let text = match std::fs::read_to_string(known_hosts) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(PersistError::Read(e.kind())),
    };
    let targets = patterns_for(host, port);
    let out: String = text
        .split_inclusive('\n')
        .filter_map(|line| strip_targets(line, &targets))
        .collect();
    if out == text {
        return Ok(());
    }
    atomic_write(known_hosts, out.as_bytes())
}

/// Spellings under which an endpoint may be recorded.
fn patterns_for(host: &str, port: u16) -> Vec<String> {
    let host = host.to_ascii_lowercase();
    let mut patterns = vec![format!("[{host}]:{port}")];
    if port == 22 {
        patterns.push(host);
    }
    patterns
}

/// The raw line (terminator included) without the target patterns, or `None` when nothing is
/// left of it. Lines that do not match are returned untouched.
fn strip_targets(raw: &str, targets: &[String]) -> Option<String> {
    let body = raw.trim_end_matches(['\r', '\n']);
    let ending = &raw[body.len()..];
    let content = body.trim_start();
    let lead = &body[..body.len() - content.len()];
    if content.is_empty() || content.starts_with('#') || content.starts_with('@') {
        return Some(raw.to_owned());
    }
    let Some(hosts_end) = content.find(char::is_whitespace) else {
        return Some(raw.to_owned());
    };
    let (hosts, rest) = content.split_at(hosts_end);
    let all: Vec<&str> = hosts.split(',').collect();
    let remaining: Vec<&str> = all
        .iter()
        .copied()
        .filter(|p| !targets.iter().any(|t| t.eq_ignore_ascii_case(p)))
        .collect();
    if remaining.len() == all.len() {
        return Some(raw.to_owned());
    }
    (!remaining.is_empty()).then(|| format!("{lead}{}{rest}{ending}", remaining.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn no_repaint() -> Repaint {
        Arc::new(|| {})
    }

    #[test]
    fn answer_sends_the_decision_exactly_once() {
        let (req, rx) = HostKeyRequest::new("h", "SHA256:x");
        let state = PromptState::default().offered(req);
        assert_eq!(state.current(), Some(("h", "SHA256:x")));
        let state = state.answered(HostKeyDecision::TrustAndRemember);
        assert_eq!(rx.try_recv(), Ok(HostKeyDecision::TrustAndRemember));
        assert!(state.current().is_none());
        // A second answer has nothing to answer and sends nothing.
        let _ = state.answered(HostKeyDecision::Trust);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_question_is_withdrawn_when_its_job_ends_and_the_dialog_is_pruned() {
        let (prompt, requests) = InteractivePrompt::channel(no_repaint(), Duration::from_secs(30));
        let live = Arc::new(AtomicBool::new(true));
        let asker = {
            let (prompt, live) = (prompt.clone(), live.clone());
            std::thread::spawn(move || {
                prompt.decide_while("h", "SHA256:x", &|| live.load(Ordering::SeqCst))
            })
        };
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let state = PromptState::default().offered(request).pruned();
        assert_eq!(
            state.current(),
            Some(("h", "SHA256:x")),
            "live: still shown"
        );
        let ended = Instant::now();
        live.store(false, Ordering::SeqCst); // the job ended; nobody answered
        assert_eq!(asker.join().unwrap(), HostKeyDecision::Reject);
        assert!(
            ended.elapsed() < Duration::from_secs(5),
            "withdrawn promptly, not at the 30 s timeout"
        );
        assert!(state.pruned().current().is_none(), "dead question pruned");
    }

    fn counting_repaint() -> (Repaint, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        (
            Arc::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
            count,
        )
    }

    #[test]
    fn a_withdrawn_question_requests_exactly_one_more_repaint_after_the_dialog_is_dead() {
        let (repaint, count) = counting_repaint();
        let (prompt, requests) = InteractivePrompt::channel(repaint, Duration::from_secs(30));
        let live = Arc::new(AtomicBool::new(true));
        let asker = {
            let (prompt, live) = (prompt.clone(), live.clone());
            std::thread::spawn(move || {
                prompt.decide_while("h", "SHA256:x", &|| live.load(Ordering::SeqCst))
            })
        };
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let state = PromptState::default().offered(request);
        let deadline = Instant::now() + Duration::from_secs(5);
        while count.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(count.load(Ordering::SeqCst), 1, "asking repaints once");
        live.store(false, Ordering::SeqCst);
        assert_eq!(asker.join().unwrap(), HostKeyDecision::Reject);
        assert_eq!(count.load(Ordering::SeqCst), 2, "withdrawing repaints once");
        assert!(
            state.pruned().current().is_none(),
            "already pruned when the repaint fires"
        );
    }

    #[test]
    fn an_answered_question_does_not_request_a_withdrawal_repaint() {
        let (repaint, count) = counting_repaint();
        let (prompt, requests) = InteractivePrompt::channel(repaint, Duration::from_secs(30));
        let asker = {
            let prompt = prompt.clone();
            std::thread::spawn(move || prompt.decide_while("h", "SHA256:x", &|| true))
        };
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = PromptState::default()
            .offered(request)
            .answered(HostKeyDecision::Trust);
        assert_eq!(asker.join().unwrap(), HostKeyDecision::Trust);
        assert_eq!(count.load(Ordering::SeqCst), 1, "only the asking repaint");
    }

    #[test]
    fn an_answer_for_a_withdrawn_question_never_answers_the_one_behind_it() {
        let (a, ra) = HostKeyRequest::new("hx", "fx");
        let (b, rb) = HostKeyRequest::new("hy", "fy");
        let state = PromptState::default().offered(a).offered(b);
        // The dialog for A was drawn; A is withdrawn (its waiter drops) before the click lands.
        drop(ra);
        let state = state.pruned();
        assert_eq!(state.current(), Some(("hy", "fy")));
        let state = state.answered_for("hx", "fx", HostKeyDecision::Trust);
        assert_eq!(state.current(), Some(("hy", "fy")), "B is still pending");
        assert!(rb.try_recv().is_err(), "B was not answered");
        let state = state.answered_for("hy", "fy", HostKeyDecision::Trust);
        assert!(state.current().is_none());
        assert_eq!(rb.try_recv(), Ok(HostKeyDecision::Trust));
    }

    #[test]
    fn the_dialog_id_differs_per_question_so_a_click_cannot_carry_over() {
        assert_ne!(dialog_id("hx", "f"), dialog_id("hy", "f"));
        assert_ne!(dialog_id("h", "f1"), dialog_id("h", "f2"));
        assert_eq!(dialog_id("h", "f"), dialog_id("h", "f"));
    }

    #[test]
    fn a_live_question_is_answered_normally_through_decide_while() {
        let (prompt, requests) = InteractivePrompt::channel(no_repaint(), Duration::from_secs(30));
        let asker = {
            let prompt = prompt.clone();
            std::thread::spawn(move || prompt.decide_while("h", "SHA256:x", &|| true))
        };
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(120)); // several slices pass
        let state = PromptState::default().offered(request).pruned();
        assert!(state.current().is_some());
        let _ = state.answered(HostKeyDecision::TrustAndRemember);
        assert_eq!(asker.join().unwrap(), HostKeyDecision::TrustAndRemember);
    }

    #[test]
    fn a_question_that_times_out_still_rejects_through_decide_while() {
        let (prompt, requests) =
            InteractivePrompt::channel(no_repaint(), Duration::from_millis(50));
        assert_eq!(
            prompt.decide_while("h", "SHA256:x", &|| true),
            HostKeyDecision::Reject
        );
        let request = requests.try_recv().unwrap();
        assert!(
            PromptState::default()
                .offered(request)
                .pruned()
                .current()
                .is_none()
        );
    }

    #[test]
    fn prompts_queue_in_order() {
        let (a, ra) = HostKeyRequest::new("a", "fa");
        let (b, rb) = HostKeyRequest::new("b", "fb");
        let state = PromptState::default().offered(a).offered(b);
        assert_eq!(state.current().map(|c| c.0), Some("a"));
        let state = state.answered(HostKeyDecision::Trust);
        assert_eq!(state.current().map(|c| c.0), Some("b"));
        let _ = state.answered(HostKeyDecision::Reject);
        assert_eq!(ra.try_recv(), Ok(HostKeyDecision::Trust));
        assert_eq!(rb.try_recv(), Ok(HostKeyDecision::Reject));
    }

    #[test]
    fn rejected_all_resolves_every_pending_prompt_with_reject() {
        let (a, ra) = HostKeyRequest::new("a", "fa");
        let (b, rb) = HostKeyRequest::new("b", "fb");
        let state = PromptState::default().offered(a).offered(b).rejected_all();
        assert!(state.current().is_none());
        assert_eq!(ra.try_recv(), Ok(HostKeyDecision::Reject));
        assert_eq!(rb.try_recv(), Ok(HostKeyDecision::Reject));
    }

    #[test]
    fn rejected_for_host_rejects_only_that_host_and_keeps_the_order() {
        let (a1, ra1) = HostKeyRequest::new("a", "f1");
        let (b, rb) = HostKeyRequest::new("b", "fb");
        let (a2, ra2) = HostKeyRequest::new("A", "f2");
        let (c, rc) = HostKeyRequest::new("c", "fc");
        let state = PromptState::default()
            .offered(a1)
            .offered(b)
            .offered(a2)
            .offered(c)
            .rejected_for_host("a");
        assert_eq!(ra1.try_recv(), Ok(HostKeyDecision::Reject));
        assert_eq!(ra2.try_recv(), Ok(HostKeyDecision::Reject));
        assert!(rb.try_recv().is_err() && rc.try_recv().is_err());
        assert_eq!(state.current(), Some(("b", "fb")));
        let state = state.answered(HostKeyDecision::Trust);
        assert_eq!(state.current(), Some(("c", "fc")));
    }

    #[test]
    fn rejected_for_an_unknown_host_changes_nothing() {
        let (a, ra) = HostKeyRequest::new("a", "fa");
        let state = PromptState::default().offered(a).rejected_for_host("zzz");
        assert!(ra.try_recv().is_err());
        assert_eq!(state.current(), Some(("a", "fa")));
    }

    #[test]
    fn dropping_the_state_makes_waiters_fail_closed() {
        let (a, ra) = HostKeyRequest::new("a", "fa");
        drop(PromptState::default().offered(a));
        assert!(ra.try_recv().is_err()); // disconnected, no decision sent
        let (prompt, rx) = InteractivePrompt::channel(no_repaint(), Duration::from_secs(5));
        let waiter = std::thread::spawn(move || prompt.decide("h", "f"));
        drop(rx.recv().unwrap()); // UI drops the request unanswered
        assert_eq!(waiter.join().unwrap(), HostKeyDecision::Reject);
    }

    #[test]
    fn decide_blocks_until_the_ui_answers_and_repaints() {
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = repaints.clone();
        let repaint: Repaint = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let (prompt, rx) = InteractivePrompt::channel(repaint, Duration::from_secs(5));
        let waiter = std::thread::spawn(move || prompt.decide("host", "SHA256:abc"));
        let request = rx.recv().unwrap();
        assert_eq!(
            (request.host.as_str(), request.fingerprint.as_str()),
            ("host", "SHA256:abc")
        );
        PromptState::default()
            .offered(request)
            .answered(HostKeyDecision::Trust);
        assert_eq!(waiter.join().unwrap(), HostKeyDecision::Trust);
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn decide_times_out_to_reject() {
        let (prompt, _rx) = InteractivePrompt::channel(no_repaint(), Duration::from_millis(50));
        let start = Instant::now();
        assert_eq!(prompt.decide("h", "f"), HostKeyDecision::Reject);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn decide_rejects_when_the_ui_is_gone() {
        let (prompt, rx) = InteractivePrompt::channel(no_repaint(), Duration::from_secs(5));
        drop(rx);
        assert_eq!(prompt.decide("h", "f"), HostKeyDecision::Reject);
    }

    fn write(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
        let p = dir.path().join("known_hosts");
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn forget_removes_only_the_matching_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "# mine\nbastion {KEY_A}\n[bastion]:2222 {KEY_B}\nother {KEY_A}\n\nBASTION {KEY_B}\n"
        );
        let p = write(&dir, &text);
        forget_host(&p, "Bastion", 22).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            after,
            format!("# mine\n[bastion]:2222 {KEY_B}\nother {KEY_A}\n\n")
        );
        forget_host(&p, "bastion", 2222).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(!after.contains("[bastion]:2222") && after.contains("other"));
    }

    #[test]
    fn forget_keeps_other_patterns_on_a_shared_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, &format!("bastion,10.0.0.1 {KEY_A}\n"));
        forget_host(&p, "bastion", 22).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            format!("10.0.0.1 {KEY_A}\n")
        );
    }

    #[test]
    fn forget_is_private_atomic_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, &format!("bastion {KEY_A}\n"));
        forget_host(&p, "bastion", 22).unwrap();
        forget_host(&p, "bastion", 22).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn forget_on_a_missing_file_is_ok_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        forget_host(&p, "bastion", 22).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn a_timed_out_request_is_pruned_and_a_live_one_stays() {
        let (dead, waiter) = HostKeyRequest::new("dead", "f1");
        let (live, _keep) = HostKeyRequest::new("live", "f2");
        let state = PromptState::default().offered(dead).offered(live);
        assert_eq!(state.current().map(|c| c.0), Some("dead"));
        drop(waiter); // what a timed-out `decide` does
        let state = state.pruned();
        assert_eq!(state.current().map(|c| c.0), Some("live"));
    }

    #[test]
    fn decide_timeout_abandons_its_request() {
        let (prompt, rx) = InteractivePrompt::channel(no_repaint(), Duration::from_millis(30));
        assert_eq!(prompt.decide("h", "f"), HostKeyDecision::Reject);
        let state = PromptState::default()
            .offered(rx.try_recv().unwrap())
            .pruned();
        assert!(state.current().is_none());
    }

    #[test]
    fn forget_keeps_unparseable_and_comment_lines_and_crlf_endings_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "#c\r\nnowhitespace\r\n  other {KEY_A}\r\n\r\nbastion {KEY_B}\r\n[bastion]:2222   {KEY_A}  \r\nlast {KEY_A}"
        );
        let p = write(&dir, &text);
        forget_host(&p, "bastion", 22).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            format!(
                "#c\r\nnowhitespace\r\n  other {KEY_A}\r\n\r\n[bastion]:2222   {KEY_A}  \r\nlast {KEY_A}"
            )
        );
    }

    #[test]
    fn forget_with_a_port_removes_only_the_bracketed_entry() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!("bastion {KEY_A}\n[bastion]:2222 {KEY_B}\n[bastion]:22222 {KEY_B}\n");
        let p = write(&dir, &text);
        forget_host(&p, "bastion", 2222).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            format!("bastion {KEY_A}\n[bastion]:22222 {KEY_B}\n")
        );
    }

    #[test]
    fn forget_without_a_match_does_not_rewrite_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "weird line without a key\n");
        forget_host(&p, "bastion", 22).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "weird line without a key\n"
        );
    }

    #[test]
    fn forget_with_a_relative_path_is_refused_and_touches_nothing() {
        let relative = Path::new("ms-relative-forget-test");
        assert_eq!(
            forget_host(relative, "bastion", 22),
            Err(PersistError::InvalidPath)
        );
        assert!(!relative.exists());
    }
}
