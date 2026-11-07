# Tool paths for mongosh, mongodump and mongorestore

Issue [#49](https://github.com/bjornharvold/curlybrace/issues/49), part of #45. The prerequisite
for the shell tab (#50) and the backup/restore wrapper (#53).

## What it does

The app does not bundle the MongoDB command-line tools. This lets the user say which `mongosh`,
`mongodump` and `mongorestore` to run, at two levels:

- a **global default**, edited in a Tool paths dialog opened from the gear (sidebar, or the
  connection manager when no session is open);
- a **per-connection override**, in a collapsed "Tool paths" section of the connection dialog,
  with a "Use global settings" box per tool.

For each tool the choice is *the one found on `PATH`* or *this file* (path box, file dialog, or
one of the suggested install folders). Next to the choice the app shows the detected version, or
the reason it cannot be used.

## What it deliberately does not do

- It does not run the tools for anything yet. That is #50 and #53.
- It only runs what the user selected: a file they picked, or the first match on their `PATH`,
  which is the default selection. Opening the Tool paths dialog therefore runs `<tool> --version`
  for a tool left on `PATH`. Suggested folders (`/opt/homebrew/bin`, `/usr/local/bin`) are only
  looked at (is it a runnable file?), never executed.
- It does not put executable paths in `connections.json`, so an exported connection file never
  carries a path from this machine.
- `mongoexport` and `mongoimport` are not added; `ToolKind` is the one place to extend.

## Data

- `curlybrace-core::tools` holds everything that is not UI: `ToolKind`, `ToolChoice`
  (`Path` | `Executable(path)`), `ToolSettings` (global map plus per-connection overrides, every
  change returns a new value), `resolve` (absolute, existing, regular, executable file, or a
  `PATH` search that skips relative entries), `probe_version` and `compatibility`.
- `tools.json`, beside `connections.json`, stores `ToolSettings`. It follows the `sampling.json`
  rules: atomic 0600 write, a file that cannot be used is backed up and the defaults apply, and
  saving is refused if the backup failed so the original is never overwritten.
- Overrides are keyed by connection id and live in `tools.json`, not on `ConnectionConfig`.
  Deleting a connection (once the delete reached the disk) drops its overrides; duplicating one
  copies them; the section is only offered for a saved connection so a cancelled new connection
  leaves no stray entry.

## Version checks

`ToolsState` (pure) records a choice and queues a numbered check. The app layer takes the queue,
runs each check on a worker thread and feeds the answer back. Only the newest check for a tool is
wanted, so the answer to a replaced choice is dropped (the same idea as the runtime's epochs).
It is not a runtime `Command`: the runtime promises one answer per command and a check never
touches the server.

A check resolves the choice first (nothing runs for a missing, non-regular or non-executable
file), then runs `<file> --version`: never through a shell, no stdin or stderr, an environment of
only `PATH` (wrappers such as a `node` script need it), a 5 second timeout after which the
process is killed, and at most 8 KiB of output read. Paths and output are never logged.

## "Too old for the server"

`compatibility(kind, tool, server)` compares the tool with a table of the oldest tool for each
server release. A version is only called too old when a server version is known. The connection
dialog knows one after **Test connection** (the test report carries it); the global dialog has no
server and shows the version only. The table is advisory and lives in one const
(`tools/version.rs`).

## Errors

Every failure is a `ToolError` with a fixed, path-free message and an i18n key (`tools.error_*`).
A file that cannot be saved raises the usual persistence banner. All text is in `en.json`.

## Tests

Unit tests with fake executables (scripts in a temp dir, no network) for parsing, resolving,
probing (timeout, output cap, minimal environment, missing interpreter), the settings model and
the store; app tests for saving, blocked saves, real worker-thread checks, the file dialog, and
delete/duplicate; accessibility tests (accesskit tree) for both dialogs.

## Follow-ups

- Show a "too old" warning in the global dialog and in the shell tab by storing the server version
  of a connected session (it needs a field on `Event::Connected`, which touches many tests).
- Verify the compatibility table against the tools' release notes.
- A macOS app started from the Finder has a minimal `PATH` without Homebrew, so "found on `PATH`"
  often finds nothing there; the suggested folders are the answer. Reading the login shell's
  `PATH` would be a separate decision.
