# Shell tab

Issue [#50](https://github.com/bjornharvold/curlybrace/issues/50). Builds on the tool paths from
#49 (`curlybrace-core::tools`).

## Why

Some work is faster as a script than through the query bar and the views: a few statements, a loop,
an admin command. People already have `mongosh` and know it. The shell tab lets them write that
script in an editor inside the app, run it against the connection they are looking at, and keep it
tidy, without switching to a terminal or re-entering credentials.

## What it does

A **Shell** tab, one per connected session. It is a connection-level tab, not a collection tab: it
does not need a collection to be selected.

- A multi-line editor for mongosh JavaScript with syntax highlighting.
- **Run** runs the selection, or the whole script when nothing is selected. **Cancel** stops the
  run in progress.
- **Format** re-indents the script (selection or whole buffer). It ships in the first usable
  version, not later.
- Output appears in a pane below the editor as text, stdout and stderr distinguished, streamed as
  it arrives.
- A per-connection **history** of scripts that were run.
- If `mongosh` is missing or too old, the tab says so in plain words and links to the Tool paths
  dialog.

It is an editor, not a terminal. There is no live prompt: each Run is one `mongosh` process that
receives the script, runs it and exits. State does not carry over between runs (variables from a
previous run are gone); a user who wants a variable defines it in the script.

## Goals

- Run the user's own `mongosh` (resolved with `ToolSettings::effective`) against the session's
  connection, honouring its TLS and auth settings and its SSH tunnel.
- Never expose the password: not in argv, not in the environment, not in logs or history, and
  not on disk unless the last-resort fallback below is in use.
- Cancel really stops the work, including anything `mongosh` spawned.
- Everything user-visible is an i18n key; the tab passes the accessibility tests.

## Non-goals (v1)

- Bundling a shell, a debugger, snippets, or a live REPL / pseudo-terminal.
- Completion of collection names and operators (a later slice, the editor is built so a completion
  source can be added), a table/tree view of output, syntax checking before run.
- Making a read-only connection writable (see below), or restricting a shell the user may run
  elsewhere.

## Anti-goals

- A password visible in `ps`, in the environment of the child, in a log or in saved history. On
  disk only in the last-resort case below: owner-only and deleted when the run ends.
- A shell that quietly bypasses the read-only and production guards.
- A Cancel that leaves `mongosh` or its children running.

## Decisions made with the user

- Editor model, not a live prompt.
- Format is part of the first usable version.
- Completion is wanted but not required; it is a stretch goal.
- A short-lived owner-only password file is an acceptable last resort if stdin and an anonymous
  pipe both fail (the user's call, delegated to the lead's recommendation).
- Best-effort password scrubbing of the output pane is wanted (same).

## Decisions made by the builder's lead (my calls, change if wrong)

- **Placement:** connection-level tab (see above).
- **Secrets:** core builds a short connect line containing the credentials and sends it to
  `mongosh --nodb` on **stdin**, followed by the user's script. Spike first: confirm that `mongosh`
  evaluates piped stdin and prints results as it would for a file. Fallbacks, in order:
  (1) hand the script to `mongosh` through an anonymous pipe (`/dev/fd/N`), which stays off disk;
  (2) as a last resort, a private script file (0600, owner-only directory) that holds the password
  only for the length of the run and is deleted when it ends, including on cancel and crash
  cleanup at next start. The user accepted (2) knowingly; the slice's notes must say which method
  shipped and the README/SECURITY notes must describe it.
- **Output scrubbing:** the exact password is replaced with `••••` in the output pane (stdout and
  stderr, across chunk boundaries). This is a courtesy against accidents such as `db.getMongo()`,
  not a security boundary: a script can transform the password before printing it, and the spec
  and UI make no stronger promise. Output is never stored in history.
- **Read-only:** `ConnectionConfig.read_only` is the flag. The refusal is enforced in core on the
  run command (through `is_write()` and the read-only registry), not only hidden in the UI.
- **Production:** the confirmation applies to every run; without a script parser a read cannot be
  told from a write.
- **One run at a time per session.** Run is disabled while one is in progress. A disconnect or
  closing the app kills the process group.
- **`mongosh` version:** uses the existing `tools::compatibility` table and the version the probe
  found. An unknown server version is not flagged.
- **Format** is a pure function (text, optional selection) with 2-space indents; a selection's
  starting depth is computed from the text before it.
- **History** stores the script text that actually ran (selection or whole), a timestamp and the
  exit status, never output; last 200 per connection; a duplicated connection gets a deep copy.
  A password the user pastes into their own script is stored as written; the UI does not claim
  otherwise.
- **Tab model:** this is the first connection-level tab, so slice B includes the change to the tab
  model in `panels.rs` and `state.rs`.
- **Read-only connections:** the shell refuses to start and the tab explains why. A raw `mongosh`
  cannot be restricted to reads.
- **SSH tunnel:** the session's local forwarded address is passed to `mongosh`. Core needs an
  accessor for it; the runtime does not expose one today. Verified TLS over a tunnel is already
  rejected by `reject_unverifiable_tls`, the shell reports the same error.
- **Format:** a small formatter owned by the project that re-indents by bracket depth and is aware
  of strings, template literals and comments. It does not reflow long lines. A full JavaScript
  formatter dependency is a possible later swap.
- **History storage:** `history.sqlite`, new schema version, keyed by connection id, purged when the
  connection is deleted and copied when it is duplicated (same rules as tool overrides).
  Passwords are never part of a stored script because the connect line is not stored.
- **Runner:** a new long-running process runner in core, modelled on `tools/probe.rs` (no shell,
  cleared environment with an absolute-only `PATH`, `process_group(0)` and group kill) but streaming
  output in bounded chunks. Output per run is capped; when the cap is hit the run is cancelled
  and the pane says so.
- **Commands and events:** new `Command` variants (run, cancel) and `Event` variants (output chunk,
  finished with exit status or error). `Command::is_write()` treats run as a write and
  `context()` gets matching keys; both are exhaustive matches.

## Errors

Each is a message with an i18n key, never a raw error string: `mongosh` not found, not executable,
too old for the server (when a server version is known), could not start, connection refused or
auth failed (as `mongosh` reports it, shown in the output), read-only connection, output cap
reached, cancelled by the user.

## Tests

- Unit tests with a fake `mongosh` script in a temp dir (no network), like `probe_tests.rs`:
  streams output, exits non-zero, prints stderr, hangs until cancelled, spawns a child that must
  also die, floods output past the cap, and checks that the password appears neither in the
  arguments nor in the environment the fake sees.
- Pure view-model tests for the tab state (run, cancel, history, format).
- Formatter tests: indentation, strings and comments untouched, idempotence.
- `accessibility_shell.rs` in the style of `accessibility_tools.rs`.
- A Tier 1/2 test against a real `mongosh` and server, gated like the other integration tests, is
  optional and never runs on GitHub CI.

## Slices

1. **A, core runner:** the stdin spike; resolve + spawn + stream + cancel + output cap; secrets;
   tunnel accessor; read-only refusal; fake-`mongosh` tests.
2. **B, first usable tab:** the Shell tab with editor and highlighting, Run (selection or all),
   Cancel, output pane, **Format**, and the missing / too-old `mongosh` message.
3. **C, history:** per-connection history, persistence and purge.
4. **D, table/tree output.** The spike showed `--json` cannot be combined with stdin, so the mechanism (for example parsing `EJSON.stringify` output the script opts into) is decided in that slice's own plan.
5. **E, completion** of collection names and common operators.

## Open for the builder

Exact crate for any dependency, the layouter-based highlighter's structure, where the connection
level tab lives in `panels.rs` and `state.rs`, the output cap value, and the history retention
limit.
