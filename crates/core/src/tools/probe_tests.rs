use super::*;
use crate::tools::ToolKind;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

fn script(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, path)
}

/// Whether the process is no longer running. A killed orphan stays a zombie until whatever adopted
/// it reaps it (a container's init may never), and a zombie is not running.
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

/// The pid a script wrote, waiting for it: the file can exist before its content does.
fn read_pid(path: &std::path::Path) -> i32 {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the script never wrote its child's pid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

const LONG: Duration = Duration::from_secs(10);

#[test]
fn the_version_comes_from_the_output() {
    let (_dir, exe) = script("echo 'mongodump version: 100.10.0'");
    assert_eq!(
        probe_version(ToolKind::Mongodump, &exe, LONG),
        Ok(ToolVersion::new(100, 10, 0))
    );
}

#[test]
fn the_tool_is_asked_for_its_version_only() {
    let (_dir, exe) = script("[ \"$1\" = \"--version\" ] && [ $# -eq 1 ] && echo 2.3.1");
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Ok(ToolVersion::new(2, 3, 1))
    );
}

#[test]
fn output_without_a_version_is_reported() {
    let (_dir, exe) = script("echo hello");
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Err(ToolError::NoVersion)
    );
}

#[test]
fn a_tool_that_does_not_answer_in_time_is_stopped() {
    let (_dir, exe) = script("exec sleep 30");
    let started = std::time::Instant::now();
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, Duration::from_millis(200)),
        Err(ToolError::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_script_with_a_missing_interpreter_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool");
    std::fs::write(&path, "#!/nonexistent/interpreter\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        probe_version(ToolKind::Mongosh, &path, LONG),
        Err(ToolError::CouldNotRun)
    );
}

#[test]
fn the_environment_is_minimal() {
    let (_dir, exe) = script(
        "if [ -n \"$HOME\" ] || [ -n \"$USER\" ] || [ -n \"$CARGO_HOME\" ]; then echo leaked; else echo 2.0.0; fi",
    );
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Ok(ToolVersion::new(2, 0, 0))
    );
}

#[test]
fn endless_output_is_capped_and_is_not_a_version() {
    // Reading stops at the cap, so the program dies of SIGPIPE: not a successful exit.
    let (_dir, exe) = script("exec yes 1.2.3");
    let started = std::time::Instant::now();
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, Duration::from_secs(5)),
        Err(ToolError::NoVersion)
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the check must not hang"
    );
}

#[test]
fn only_absolute_path_entries_reach_the_tool() {
    // A wrapper such as a `node` script looks its interpreter up on PATH: a relative entry
    // would run whatever sits in the working directory.
    let (_dir, exe) =
        script("case \":$PATH:\" in *:.:*|*::*|*:bin:*) echo leaked;; *) echo 2.0.0;; esac");
    let path = std::ffi::OsString::from(".:bin::/usr/bin:/bin");
    assert_eq!(
        probe_version_with_path(ToolKind::Mongosh, &exe, LONG, Some(&path)),
        Ok(ToolVersion::new(2, 0, 0))
    );
}

#[test]
fn a_timeout_also_stops_what_the_tool_started() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let (_tool_dir, exe) = script(&format!(
        "sleep 30 &\necho $! > '{}'\nwait",
        pid_file.display()
    ));
    // Long enough that a busy machine has started the script before the timeout fires.
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, Duration::from_secs(2)),
        Err(ToolError::TimedOut)
    );
    let pid = read_pid(&pid_file);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !is_gone(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "the child outlived the timeout"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_program_that_is_not_the_tool_is_rejected() {
    let (_dir, exe) = script("echo 'Python 3.12.1'");
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Err(ToolError::NoVersion)
    );
    assert_eq!(
        probe_version(ToolKind::Mongodump, &exe, LONG),
        Err(ToolError::NoVersion)
    );
}

#[test]
fn a_failing_exit_is_not_a_version_even_if_it_prints_one() {
    let (_dir, exe) = script("echo 2.3.1; exit 1");
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Err(ToolError::NoVersion)
    );
}

#[test]
fn a_child_left_holding_the_output_does_not_hide_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let (_tool_dir, exe) = script(&format!(
        "sleep 30 &\necho $! > '{}'\necho 2.3.1",
        pid_file.display()
    ));
    let started = std::time::Instant::now();
    assert_eq!(
        probe_version(ToolKind::Mongosh, &exe, LONG),
        Ok(ToolVersion::new(2, 3, 1))
    );
    assert!(started.elapsed() < Duration::from_secs(8));
    let pid = read_pid(&pid_file);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !is_gone(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "the child was left running"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
