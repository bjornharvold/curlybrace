//! A stand-in `mongosh` for tests: a bash script in a temp dir that behaves like
//! `mongosh --nodb --quiet` reading piped stdin. No real `mongosh` or network is needed.
//!
//! It prints `> ` before reading the first line (the preamble), answers it with
//! `__CB_READY__` plus the blank line `print` leaves, then prompts `test> ` before each later line
//! and exits 0 on EOF. Script lines it understands:
//!
//! | line | effect |
//! | --- | --- |
//! | `throw <msg>` | `Uncaught \rError: <msg>` on stdout, as `mongosh` does |
//! | `stderr <text>` | `<text>` on stderr |
//! | `exit <n>` | exits with status `n` |
//! | `dump` | `arg:<a>` per argument, then `env` |
//! | `spawn <pidfile>` | starts a background `sleep 30` and writes its pid to `<pidfile>` |
//! | `hang` | `sleep 30` in the foreground |
//! | `flood` | endless output (`yes flood`) |
//! | `bytes <fmt>` | `printf <fmt>` (raw bytes, octal escapes; no spaces in `fmt`) |
//! | `split <a> <b>` | `printf <a>`, a 0.2 s pause, `printf <b>`: two separate reads |
//! | `long <n>` | `n` times `x` without a newline |
//! | ends in `+` | an unfinished block: reads more lines until one does not end in `+` |
//! | anything else | echoed back |
//!
//! The child gets an empty environment, so it needs a `PATH` with `/usr/bin:/bin`
//! ([`FAKE_PATH`]).
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The `PATH` to run the fake with.
pub(crate) const FAKE_PATH: &str = "/usr/bin:/bin";

/// How the fake answers the preamble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Behaviour {
    /// Answers `__CB_READY__` and runs the REPL.
    Repl,
    /// Answers `__CB_FAILED__ <message>`, then logs every further stdin line to
    /// [`FakeMongosh::stdin_log`] until EOF. `message` must not contain `'`.
    ConnectFails(String),
    /// Waits up to a second after the preamble; if more input arrives before it answers
    /// `__CB_READY__` it prints `EARLY` after the sentinel. Then runs the REPL.
    ChecksEarlyInput,
    /// Never reads stdin: prints the sentinel and `ignored`, waits 0.3 s and exits 0.
    IgnoresStdin,
}

/// The fake executable; the temp dir (and the script) is removed on drop.
pub(crate) struct FakeMongosh {
    dir: tempfile::TempDir,
    pub(crate) exe: PathBuf,
}

impl FakeMongosh {
    pub(crate) fn new(behaviour: Behaviour) -> Self {
        let dir = tempfile::tempdir().expect("temp dir for the fake mongosh");
        let exe = dir.path().join("mongosh");
        let (mode, message) = match &behaviour {
            Behaviour::Repl => ("repl", ""),
            Behaviour::ConnectFails(message) => ("fail", message.as_str()),
            Behaviour::ChecksEarlyInput => ("early", ""),
            Behaviour::IgnoresStdin => ("ignore", ""),
        };
        assert!(!message.contains('\''), "the fake cannot quote '");
        let script = SCRIPT
            .replace("@MODE@", mode)
            .replace("@MESSAGE@", message)
            .replace("@LOG@", &dir.path().join("stdin.log").display().to_string());
        std::fs::write(&exe, script).expect("write the fake mongosh");
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
            .expect("make the fake mongosh executable");
        Self { dir, exe }
    }

    /// Where [`Behaviour::ConnectFails`] logs the stdin lines it read after the preamble.
    pub(crate) fn stdin_log(&self) -> PathBuf {
        self.dir.path().join("stdin.log")
    }

    /// A scratch path inside the fake's temp dir (for pid files).
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub(crate) fn dir(&self) -> &Path {
        self.dir.path()
    }
}

const SCRIPT: &str = r#"#!/bin/bash
MODE='@MODE@'
if [ "$MODE" = ignore ]; then
  printf '__CB_READY__\n\nignored\n'
  sleep 0.3
  exit 0
fi
printf '> '
IFS= read -r preamble || exit 0
if [ "$MODE" = fail ]; then
  printf '__CB_FAILED__ %s\n\n> ' '@MESSAGE@'
  while IFS= read -r line; do printf '%s\n' "$line" >> '@LOG@'; done
  exit 0
fi
early=''
if [ "$MODE" = early ] && IFS= read -r -t 1 line; then early=yes; fi
printf '__CB_READY__\n\n'
[ -n "$early" ] && echo EARLY
while true; do
  printf 'test> '
  IFS= read -r line || exit 0
  case "$line" in
    'throw '*) printf 'Uncaught \rError: %s\n' "${line#throw }" ;;
    'stderr '*) printf '%s\n' "${line#stderr }" >&2 ;;
    'exit '*) exit "${line#exit }" ;;
    dump) for a in "$@"; do printf 'arg:%s\n' "$a"; done; env ;;
    'spawn '*) sleep 30 & echo $! > "${line#spawn }" ;;
    hang) sleep 30 ;;
    flood) yes flood ;;
    'bytes '*) printf "${line#bytes }" ;;
    'split '*) read -r _ first second <<< "$line"; printf "$first"; sleep 0.2; printf "$second" ;;
    'long '*) head -c "${line#long }" /dev/zero | tr '\0' x ;;
    *+) while [[ "$line" == *+ ]]; do IFS= read -r line || exit 0; done; echo NaN ;;
    *) printf '%s\n' "$line" ;;
  esac
done
"#;
