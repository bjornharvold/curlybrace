use super::*;
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

const LONG: Duration = Duration::from_secs(10);

#[test]
fn the_version_comes_from_the_output() {
    let (_dir, exe) = script("echo 'mongodump version: 100.10.0'");
    assert_eq!(probe_version(&exe, LONG), Ok(ToolVersion::new(100, 10, 0)));
}

#[test]
fn the_tool_is_asked_for_its_version_only() {
    let (_dir, exe) = script("[ \"$1\" = \"--version\" ] && [ $# -eq 1 ] && echo 2.3.1");
    assert_eq!(probe_version(&exe, LONG), Ok(ToolVersion::new(2, 3, 1)));
}

#[test]
fn output_without_a_version_is_reported() {
    let (_dir, exe) = script("echo hello");
    assert_eq!(probe_version(&exe, LONG), Err(ToolError::NoVersion));
}

#[test]
fn a_tool_that_does_not_answer_in_time_is_stopped() {
    let (_dir, exe) = script("exec sleep 30");
    let started = std::time::Instant::now();
    assert_eq!(
        probe_version(&exe, Duration::from_millis(200)),
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
    assert_eq!(probe_version(&path, LONG), Err(ToolError::CouldNotRun));
}

#[test]
fn the_environment_is_minimal() {
    let (_dir, exe) = script(
        "if [ -n \"$HOME\" ] || [ -n \"$USER\" ] || [ -n \"$CARGO_HOME\" ]; then echo leaked; else echo 2.0.0; fi",
    );
    assert_eq!(probe_version(&exe, LONG), Ok(ToolVersion::new(2, 0, 0)));
}

#[test]
fn endless_output_is_capped() {
    let (_dir, exe) = script("exec yes 1.2.3");
    assert_eq!(
        probe_version(&exe, Duration::from_secs(5)),
        Ok(ToolVersion::new(1, 2, 3))
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
        probe_version_with_path(&exe, LONG, Some(&path)),
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
    assert_eq!(
        probe_version(&exe, Duration::from_millis(500)),
        Err(ToolError::TimedOut)
    );
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        // Signal 0 only asks whether the process still exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the child outlived the timeout"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
