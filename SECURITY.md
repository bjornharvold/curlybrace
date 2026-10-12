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
