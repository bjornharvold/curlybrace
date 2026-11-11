# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private reporting:

1. Go to the [Security tab](https://github.com/bjornharvold/curlybrace/security) of this repository.
2. Choose **Report a vulnerability** and describe what you found and how to reproduce it.

You can expect an acknowledgement within a few days. Fixes are released as a new version, and the
changelog will mention the fix without exploit details until users have had time to update.

## Supported versions

Only the latest release is supported while the app is at version 0.x.

## What counts

The app handles database credentials, SSH keys and host-key trust decisions, so these are in scope:

- Secrets (passwords, passphrases, keys) appearing in logs, errors, config files or the UI.
- Accepting an SSH host key without the user's explicit decision, or trusting a changed key.
- Acting on data from a previous connection or session after the user switched.
- Anything that makes an edit or delete hit a different document than the one shown.

Reports about the app being unsigned or not notarized are known: see the README.

## How the Shell tab hands a password to `mongosh`

The Shell tab runs your scripts in a `mongosh` process. This is exactly how the connection's
password reaches it, and where it does not:

- The password is written to the child's **stdin** as its first line, a connect line with the
  percent-encoded credentials. It is never in the command line (the arguments are only `--nodb` and
  `--quiet`), never in the environment, and never in a log or in the Shell history (below).
- The output pane masks the exact password and its percent-encoded form. This is a best-effort
  courtesy, **not a security boundary**: a script can read and transform the value it was started with,
  and anything it prints appears in the output, in a form the app cannot recognise. A short
  password such as `admin` is masked wherever that word appears, including in legitimate output
  such as a list of databases; this is by design.
- `mongosh` itself writes the lines you run later to its own log, `~/.mongodb/mongosh/*_log`, as
  "Evaluating input". It does not log the connect line. This was checked on macOS with `mongosh`
  2.14.1; it has not been checked on Linux or Windows. On Linux the process environment
  (`/proc/<pid>/environ`) was not checked either. Do not put secrets in the scripts you type.
- The Shell history stores each script that ran **in plain text** in `history.sqlite` (owner-only,
  `0600`), exactly as written: anything you typed into a script, such as a password, is kept, up to
  the last 200 scripts per connection. Output is never stored. Duplicating a connection copies it.
  Deleting the connection or **Clear history** removes it from the app's database, but SQLite can
  keep the old bytes in the file's free space and write-ahead log until it reuses them, and backups
  (such as Time Machine) keep their own copies. A password typed into a script may stay on disk for
  a while: do not type secrets into scripts.
- The **Table** and **Tree** views get their data from a side channel: one extra line the app sends
  after your script, marked with a random per-run token that the script cannot know (a line a
  script prints with a guessed marker is plain text). That line goes through the same password
  masking as the text, before it is parsed, and is never written to a log. The masking also covers
  the escaped forms of the password (a password with a backslash, quote or control character is now
  masked in the text pane too). The feature depends on `mongosh` internals: a `mongosh` upgrade can
  reduce it to "no table", never to wrong data. `prompt` and `__cbEmit` are locked in the preamble
  so a script cannot redefine them.
- A read-only connection is refused in the core, not only in the UI. A production connection asks
  for confirmation on every run.
- A script runs with your own credentials and can do anything you can do on that server, including
  writes and drops. The app does not inspect what a script does.
- The script runs as one block, sent to `mongosh` a line at a time, each line only when `mongosh`
  asks for it. As soon as `mongosh` has evaluated what it read, nothing more is sent. So a syntax
  error anywhere means none of the script runs, and an `else` on its own line can never run apart
  from its `if`. The shell commands `use`, `show` and `it` are syntax errors in a block: write
  `db.getSiblingDB("name")` and `db.adminCommand({ listDatabases: 1 })` instead. A stray `}` that
  closes the block early, or a bracket, quote or comment left open, stops the run with a notice
  that the rest of the script did not run. `passwordPrompt()` throws an error instead of reading
  the rest of the script as a password.
- Cancel, a lost connection and quitting the app stop a running `mongosh` and everything it
  started. If the app itself is killed hard (a crash, or `SIGKILL`), a long-running `mongosh` is not
  stopped with it.
