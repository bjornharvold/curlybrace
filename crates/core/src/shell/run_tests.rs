use super::*;
use crate::shell::MASK;
use crate::shell::fake::{Behaviour, FAKE_PATH, FakeMongosh};
use std::ffi::OsString;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LONG: Duration = Duration::from_secs(10);
const SECRET: &str = "p@ss:w/rd";
const SECRET_ENCODED: &str = "p%40ss%3Aw%2Frd";

/// What a run reported, in callback order.
#[derive(Clone, Debug, PartialEq)]
enum Event {
    Out(Stream, String),
    Done(Outcome),
}

/// Cancels the run when a test fails half-way, so no fake outlives it.
struct Guard(ShellHandle);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct Run {
    events: Arc<Mutex<Vec<Event>>>,
    done: mpsc::Receiver<()>,
    guard: Guard,
}

impl Run {
    /// Waits for `on_done` and returns the outcome.
    fn wait(&self) -> Outcome {
        self.done
            .recv_timeout(LONG)
            .expect("the run must finish in time");
        let events = self.events.lock().unwrap().clone();
        let dones: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, Event::Done(_)))
            .collect();
        assert_eq!(dones.len(), 1, "on_done exactly once: {events:?}");
        match events.last() {
            Some(Event::Done(outcome)) => *outcome,
            other => panic!("on_done must come after all output, last was {other:?}"),
        }
    }

    fn text(&self, stream: Stream) -> String {
        self.chunks(stream).concat()
    }

    fn chunks(&self, stream: Stream) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Out(s, text) if *s == stream => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn handle(&self) -> &ShellHandle {
        &self.guard.0
    }

    /// The outcome already reported, checking nothing more arrived since.
    fn wait_again_is_quiet(&self) -> Outcome {
        assert!(self.done.try_recv().is_err(), "on_done ran twice");
        match self.events.lock().unwrap().last() {
            Some(Event::Done(outcome)) => *outcome,
            other => panic!("output after on_done: {other:?}"),
        }
    }
}

fn invocation(script: &str) -> Invocation {
    Invocation {
        args: vec![OsString::from("--nodb"), OsString::from("--quiet")],
        preamble: format!("try {{ db = connect(\"mongodb://u:{SECRET_ENCODED}@h/admin\") }}\n")
            .into_bytes(),
        script: format!("{script}\n").into_bytes(),
    }
}

fn start_with(exe: &Path, inv: Invocation, limits: RunLimits) -> Run {
    let secrets = vec![SECRET.to_owned(), SECRET_ENCODED.to_owned()];
    start_full(exe, inv, secrets, limits)
}

fn start_full(exe: &Path, inv: Invocation, secrets: Vec<String>, limits: RunLimits) -> Run {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (tx, done) = mpsc::channel();
    let out_events = Arc::clone(&events);
    let done_events = Arc::clone(&events);
    let handle = spawn_shell(
        exe,
        inv,
        secrets,
        Some(OsStr::new(FAKE_PATH)),
        limits,
        move |stream, text| out_events.lock().unwrap().push(Event::Out(stream, text)),
        move |outcome| {
            done_events.lock().unwrap().push(Event::Done(outcome));
            let _ = tx.send(());
        },
    );
    Run {
        events,
        done,
        guard: Guard(handle),
    }
}

fn start(fake: &FakeMongosh, script: &str) -> Run {
    start_with(&fake.exe, invocation(script), RunLimits::default())
}

fn repl() -> FakeMongosh {
    FakeMongosh::new(Behaviour::Repl)
}

/// Whether the process is no longer running (a zombie is not running).
fn is_gone(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    let state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    state.is_empty() || state.starts_with('Z')
}

fn read_pid(path: &Path) -> i32 {
    let deadline = Instant::now() + LONG;
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "the fake never wrote the pid");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn streams_stdout_and_stderr_in_order_per_stream() {
    let fake = repl();
    let run = start(&fake, "a\nstderr x\nb\nstderr y\nc");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "a\nb\nc\n");
    assert_eq!(run.text(Stream::Stderr), "x\ny\n");
}

#[test]
fn reports_exit_status() {
    let fake = repl();
    let run = start(&fake, "a\nexit 3");
    assert_eq!(run.wait(), Outcome::Exited(3));
    assert_eq!(run.text(Stream::Stdout), "a\n");
}

#[test]
fn password_not_in_args_or_env_seen_by_the_child() {
    let fake = repl();
    let run = start(&fake, "dump");
    assert_eq!(run.wait(), Outcome::Exited(0));
    let out = run.text(Stream::Stdout);
    // A masked secret would mean the child saw it; the raw forms must not appear at all.
    for leaked in [SECRET, SECRET_ENCODED, MASK] {
        assert!(!out.contains(leaked), "leaked {leaked:?} in {out:?}");
    }
    let args: Vec<&str> = out
        .lines()
        .filter_map(|line| line.strip_prefix("arg:"))
        .collect();
    assert_eq!(args, ["--nodb", "--quiet"]);
    let names: Vec<&str> = out
        .lines()
        .filter(|line| !line.starts_with("arg:"))
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .collect();
    assert!(names.contains(&"PATH"), "{names:?}");
    for name in names {
        // PWD, SHLVL, _ and OLDPWD are set by bash itself.
        assert!(
            ["PATH", "HOME", "PWD", "SHLVL", "_", "OLDPWD"].contains(&name),
            "unexpected variable {name}"
        );
    }
}

#[test]
fn prompts_are_stripped_from_output() {
    let fake = repl();
    let run = start(&fake, "first\n\nsecond");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "first\n\nsecond\n");
    assert_eq!(run.text(Stream::Stderr), "");
}

#[test]
fn uncaught_blocks_go_to_stderr_and_give_script_errors() {
    let fake = repl();
    let run = start(&fake, "before\nthrow boom\nafter");
    assert_eq!(run.wait(), Outcome::ScriptErrors);
    assert_eq!(run.text(Stream::Stdout), "before\nafter\n");
    assert_eq!(run.text(Stream::Stderr), "Uncaught Error: boom\n");
}

#[test]
fn connect_failure_shows_a_scrubbed_message_kills_the_child_and_never_writes_the_script() {
    let fake = FakeMongosh::new(Behaviour::ConnectFails(format!(
        "Authentication failed for {SECRET}"
    )));
    let run = start(&fake, "db.secretScript()");
    assert_eq!(run.wait(), Outcome::ConnectFailed);
    assert_eq!(
        run.text(Stream::Stderr),
        format!("Authentication failed for {MASK}\n")
    );
    assert_eq!(run.text(Stream::Stdout), "");
    let log = std::fs::read_to_string(fake.stdin_log()).unwrap_or_default();
    assert!(
        !log.contains("secretScript"),
        "the script was written: {log:?}"
    );
}

#[test]
fn script_is_written_only_after_the_ready_sentinel() {
    let fake = FakeMongosh::new(Behaviour::ChecksEarlyInput);
    let run = start(&fake, "late");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "late\n");
}

#[test]
fn incomplete_block_at_eof_ends_the_run_and_does_not_hang() {
    let fake = repl();
    let started = Instant::now();
    let run = start(&fake, "1+");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn password_and_its_percent_encoded_form_in_output_are_masked() {
    let fake = repl();
    let run = start(&fake, &format!("{SECRET}\nx {SECRET_ENCODED} y"));
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), format!("{MASK}\nx {MASK} y\n"));
}

#[test]
fn cancel_kills_the_whole_group() {
    let fake = repl();
    let pid_file = fake.path("bg.pid");
    let run = start(&fake, &format!("spawn {}\nhang", pid_file.display()));
    let pid = read_pid(&pid_file);
    run.handle().cancel();
    assert_eq!(run.wait(), Outcome::Cancelled);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_gone(pid) {
        assert!(Instant::now() < deadline, "the background child survived");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn second_cancel_and_cancel_after_exit_are_harmless() {
    let fake = repl();
    let run = start(&fake, "hang");
    std::thread::sleep(Duration::from_millis(100));
    run.handle().cancel();
    run.handle().cancel();
    assert_eq!(run.wait(), Outcome::Cancelled);

    let finished = start(&fake, "a");
    assert_eq!(finished.wait(), Outcome::Exited(0));
    finished.handle().cancel();
    finished.handle().cancel();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(finished.wait_again_is_quiet(), Outcome::Exited(0));
}

#[test]
fn child_that_never_reads_stdin_does_not_hang() {
    let fake = FakeMongosh::new(Behaviour::IgnoresStdin);
    // Far more than a pipe buffer holds, so the writer would block if nothing stopped it.
    let script = "x".repeat(1024 * 1024);
    let started = Instant::now();
    let run = start(&fake, &script);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "ignored\n");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn output_over_the_cap_cancels_with_cap_reached() {
    let fake = repl();
    let limits = RunLimits {
        max_output_bytes: 64 * 1024,
        chunk_bytes: 4 * 1024,
    };
    let run = start_with(&fake.exe, invocation("flood"), limits);
    assert_eq!(run.wait(), Outcome::CapReached);
    let shown = run.text(Stream::Stdout).len() + run.text(Stream::Stderr).len();
    assert!(shown > 0 && shown <= limits.max_output_bytes, "{shown}");
}

#[test]
fn non_utf8_output_is_decoded_lossily() {
    let fake = repl();
    let run = start(&fake, r"bytes \377\376abc\n");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "\u{FFFD}\u{FFFD}abc\n");
}

#[test]
fn multi_byte_character_split_across_reads_is_kept_whole() {
    let fake = repl();
    // `é` is \303\251: the first byte ends one read, the second starts the next.
    let run = start(&fake, r"split caf\303 \251\n");
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "café\n");
}

#[test]
fn non_ascii_password_split_across_reads_is_masked() {
    let fake = repl();
    let secret = "pässwörd";
    let inv = invocation(r"split <p\303 \244sswörd>\n");
    let run = start_full(
        &fake.exe,
        inv,
        vec![secret.to_owned()],
        RunLimits::default(),
    );
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), format!("<{MASK}>\n"));
}

#[test]
fn long_line_without_newline_is_chunked() {
    let fake = repl();
    let limits = RunLimits {
        max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        chunk_bytes: 1024,
    };
    let run = start_with(&fake.exe, invocation("long 20000"), limits);
    assert_eq!(run.wait(), Outcome::Exited(0));
    let chunks = run.chunks(Stream::Stdout);
    assert!(chunks.len() > 1, "one chunk of {}", chunks.concat().len());
    // The prompt after the unterminated line is stripped too.
    assert_eq!(chunks.concat(), "x".repeat(20000));
    for chunk in &chunks {
        assert!(chunk.len() <= 2 * limits.chunk_bytes, "{}", chunk.len());
    }
}

#[test]
fn missing_executable_reports_start_failed() {
    let fake = repl();
    let missing = fake.dir().join("no-such-mongosh");
    let run = start_with(&missing, invocation("a"), RunLimits::default());
    assert_eq!(run.wait(), Outcome::StartFailed);
    assert!(run.chunks(Stream::Stdout).is_empty());
    run.handle().cancel();
}
