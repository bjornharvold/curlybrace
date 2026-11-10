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
  `--quiet`), never in the environment, and never in a log or in the app's history.
- The output pane masks the exact password and its percent-encoded form. This is a best-effort
  courtesy, **not a security boundary**: a script can transform or print the connection string (for
  example `db.getMongo()`), and then the secret is in the output in a form the app cannot recognise.
- `mongosh` itself writes the lines you run later to its own log, `~/.mongodb/mongosh/*_log`, as
  "Evaluating input". It does not log the connect line. This was checked on macOS with `mongosh`
  2.14.1; it has not been checked on Linux or Windows. On Linux the process environment
  (`/proc/<pid>/environ`) was not checked either. Do not put secrets in the scripts you type.
- A read-only connection is refused in the core, not only in the UI. A production connection asks
  for confirmation on every run.
- A script runs with your own credentials and can do anything you can do on that server, including
  writes and drops. The app does not inspect what a script does.
