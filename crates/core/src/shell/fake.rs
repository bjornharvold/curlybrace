//! A stand-in `mongosh` for tests: a bash script in a temp dir that behaves like
//! `mongosh --nodb --quiet` reading piped stdin. No real `mongosh` or network is needed.
//!
//! It prints `> ` before reading the first line (the preamble), answers it with
//! `__CB_READY__` plus the blank line `print` leaves, then prompts with the fixed prompt the
//! preamble sets (`\x01CBP> `) before each later statement and exits 0 on EOF.
//!
//! Statements: like `mongosh`, a line that leaves a bracket (`{([`) open, or ends in `+`, makes
//! the next lines continuation lines until the brackets close; before reading each one it prints
//! `| ` (no newline). So the script, sent as one block (`{` + script + `}`), is one statement. A
//! multi-line statement then runs its lines in order (the block's opening `{` dropped; lines
//! ending in `{` and lone `}` are syntax and skipped), and a throw skips the rest of it. A
//! `syntax` line ends the statement at once with a syntax error and none of it runs, like a parse
//! error part way through a block: the lines after it, if sent, are new statements. A statement
//! whose first line starts with `function <name>` prints `[Function: <name>]`; one whose first
//! line ends in `+` prints `NaN`. Script lines it understands:
//!
//! | line | effect |
//! | --- | --- |
//! | `throw <msg>` | `Uncaught \rError: <msg>` on stdout, as `mongosh` does |
//! | `syntax` | a syntax error block (`Uncaught:\n\rSyntaxError: ...` with a code frame) |
//! | `stderr <text>` | `<text>` on stderr |
//! | `exit <n>` | exits with status `n` |
//! | `dump` | `arg:<a>` per argument, then `env` |
//! | `spawn <pidfile>` | starts a background `sleep 30` and writes its pid to `<pidfile>` |
//! | `hang` | `sleep 30` in the foreground |
//! | `flood` | endless output (`yes flood`) |
//! | `bytes <fmt>` | `printf <fmt>` (raw bytes, octal escapes; no spaces in `fmt`) |
//! | `split <a> <b>` | `printf <a>`, a 0.2 s pause, `printf <b>`: two separate reads |
//! | `long <n>` | `n` times `x` without a newline |
//! | `passwordPrompt()` | with the preamble's override: its `Uncaught \rError: <message>`; without: `Enter password`, then reads all of stdin as the password and exits 0, as `mongosh` does |
//! | `result <payload>` | the epilogue's answer: `\x01CBJ-<token> <payload>`, then the blank line `print` leaves |
//! | `result-twice <payload>` | the same answer, twice |
//! | `result-huge <n>` | the answer is `n` times `x` |
//! | `result-slow` | the epilogue prints `epilogue-received` and hangs (`sleep 30`) |
//! | `spoof <token>` | prints a marker line now: `\x01CBJ-<token> {"k":"spoof"}` |
//! | `prompt <text>` | later prompts are `<text>` instead of the fixed one (a redefined `prompt`) |
//! | anything else | echoed back |
//!
//! The epilogue (`__cbEmit(_, "\u0001CBJ-<token> ", 200, 4194304)`, one complete statement, so
//! no `| `) is answered as the script's last `result*` line said, with the token read from the
//! epilogue itself; without such a line it answers nothing. Like `mongosh`, the prompt follows.
//!
//! What it was started with and every stdin line it read go to [`FakeMongosh::seen_log`]:
//! `arg:<a>` per argument, `env:<NAME=value>` per variable, then `in:<line>` per line read.
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
    /// Reads the preamble, prints `junk 1`, `> junk 2` and `Error: cannot start` and exits 3
    /// without a sentinel.
    ExitsBeforeReady,
    /// Reads the preamble, prints two junk lines, creates [`FakeMongosh::stdin_log`] (so a test
    /// knows the lines are out) and hangs (`sleep 30`) without a sentinel.
    HangsBeforeReady,
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
            Behaviour::ExitsBeforeReady => ("exit", ""),
            Behaviour::HangsBeforeReady => ("hang", ""),
        };
        assert!(!message.contains('\''), "the fake cannot quote '");
        let script = SCRIPT
            .replace("@MODE@", mode)
            .replace("@MESSAGE@", message)
            .replace("@LOG@", &dir.path().join("stdin.log").display().to_string())
            .replace("@SEEN@", &dir.path().join("seen.log").display().to_string());
        std::fs::write(&exe, script).expect("write the fake mongosh");
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
            .expect("make the fake mongosh executable");
        Self { dir, exe }
    }

    /// Where [`Behaviour::ConnectFails`] logs the stdin lines it read after the preamble.
    pub(crate) fn stdin_log(&self) -> PathBuf {
        self.dir.path().join("stdin.log")
    }

    /// Its arguments, environment and every stdin line it read (see the module docs).
    pub(crate) fn seen_log(&self) -> PathBuf {
        self.dir.path().join("seen.log")
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
SEEN='@SEEN@'
{ for a in "$@"; do printf 'arg:%s\n' "$a"; done; env | sed 's/^/env:/'; } > "$SEEN"
# Reads one stdin line into $line and logs it.
read_line() {
  IFS= read -r line || return 1
  printf 'in:%s\n' "$line" >> "$SEEN"
}
if [ "$MODE" = ignore ]; then
  printf '__CB_READY__\n\nignored\n'
  sleep 0.3
  exit 0
fi
printf '> '
read_line || exit 0
preamble="$line"
if [ "$MODE" = fail ]; then
  printf '__CB_FAILED__ %s\n\n> ' '@MESSAGE@'
  while IFS= read -r line; do printf '%s\n' "$line" >> '@LOG@'; done
  exit 0
fi
if [ "$MODE" = exit ]; then printf 'junk 1\n> junk 2\nError: cannot start\n'; exit 3; fi
if [ "$MODE" = hang ]; then printf 'junk 1\njunk 2\n'; : > '@LOG@'; sleep 30; exit 0; fi
early=''
if [ "$MODE" = early ] && IFS= read -r -t 1 line; then early=yes; fi
# The message the preamble's `passwordPrompt` override throws, if it has one.
pw_re='passwordPrompt = \(\) => \{ throw new Error\("([^"]*)"\) \}'
pw_blocked=''
if [[ "$preamble" =~ $pw_re ]]; then pw_blocked="${BASH_REMATCH[1]}"; fi
printf '__CB_READY__\n\n'
[ -n "$early" ] && echo EARLY
# Open minus closed brackets in $1.
depth_of() {
  local opens="${1//[^\{\(\[]/}" closes="${1//[^\}\)\]]/}"
  echo $(( ${#opens} - ${#closes} ))
}
# One command; returns 1 when it throws, which skips the rest of its statement.
run_line() {
  case "$1" in
    'throw '*) printf 'Uncaught \rError: %s\n' "${1#throw }"; return 1 ;;
    syntax) printf 'Uncaught:\n\rSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n\n'; return 1 ;;
    'stderr '*) printf '%s\n' "${1#stderr }" >&2 ;;
    'exit '*) exit "${1#exit }" ;;
    dump) for a in "${ARGS[@]}"; do printf 'arg:%s\n' "$a"; done; env ;;
    'spawn '*) sleep 30 & echo $! > "${1#spawn }" ;;
    hang) sleep 30 ;;
    flood) yes flood ;;
    'bytes '*) printf "${1#bytes }" ;;
    'split '*) read -r _ first second <<< "$1"; printf "$first"; sleep 0.2; printf "$second" ;;
    'long '*) head -c "${1#long }" /dev/zero | tr '\0' x ;;
    'result '*) answer_mode=payload; answer="${1#result }" ;;
    'result-twice '*) answer_mode=twice; answer="${1#result-twice }" ;;
    'result-huge '*) answer_mode=huge; answer="${1#result-huge }" ;;
    result-slow) answer_mode=slow ;;
    'spoof '*) printf '\001CBJ-%s {"k":"spoof"}\n' "${1#spoof }" ;;
    'prompt '*) PROMPT_TEXT="${1#prompt }" ;;
    'passwordPrompt()')
      if [ -n "$pw_blocked" ]; then printf 'Uncaught \rError: %s\n' "$pw_blocked"; return 1; fi
      printf 'Enter password\n'; cat > /dev/null; exit 0 ;;
    *) printf '%s\n' "$1" ;;
  esac
}
# Evaluates one statement (its lines as arguments; a block's opening `{` is dropped).
evaluate() {
  set -- "${1#\{}" "${@:2}"
  local re='^function ([A-Za-z_]+)'
  if [[ "$1" =~ $re ]]; then printf '[Function: %s]\n' "${BASH_REMATCH[1]}"; return; fi
  if [[ "$1" == *+ ]]; then echo NaN; return; fi
  if [ $# -eq 1 ]; then run_line "$1"; return; fi
  local line
  for line in "$@"; do
    case "$line" in *'{'|'}') continue ;; esac
    run_line "$line" || return
  done
}
# The epilogue's answer, as `__cbEmit` prints it: one line, then the blank line `print` leaves.
answer_epilogue() {
  case "$answer_mode" in
    payload) printf '\001CBJ-%s %s\n\n' "$1" "$answer" ;;
    twice) printf '\001CBJ-%s %s\n\n' "$1" "$answer" "$1" "$answer" ;;
    huge) printf '\001CBJ-%s ' "$1"; head -c "$answer" /dev/zero | tr '\0' x; printf '\n\n' ;;
    slow) printf 'epilogue-received\n'; sleep 30 ;;
    *) ;;
  esac
}
emit_re='^__cbEmit\(_, "\\u0001CBJ-([0-9a-f]+) ", 200, 4194304\)$'
answer_mode=''
answer=''
PROMPT_TEXT=$'\001CBP> '
ARGS=("$@")
while true; do
  printf '%s' "$PROMPT_TEXT"
  read_line || exit 0
  if [[ "$line" =~ $emit_re ]]; then answer_epilogue "${BASH_REMATCH[1]}"; continue; fi
  lines=()
  depth=0
  # Like mongosh: an unfinished statement reads continuation lines, each after a `| `; a
  # syntax error ends the statement at once (nothing of it runs).
  while true; do
    if [ "${line#\{}" = syntax ]; then run_line syntax; lines=(); break; fi
    lines+=("$line")
    depth=$((depth + $(depth_of "$line")))
    if [ "$depth" -le 0 ] && [[ "$line" != *+ ]]; then break; fi
    printf '| '
    read_line || exit 0
  done
  if [ ${#lines[@]} -gt 0 ]; then evaluate "${lines[@]}"; fi
done
"#;
