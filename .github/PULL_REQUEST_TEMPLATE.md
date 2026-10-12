## What and why

<!-- What does this change, and why is it needed? Link the issue if there is one. -->

## How it was tested

<!-- Unit tests? Integration tests with MONGOSTUDIO_IT=1? Manual steps? -->

## Checklist

- [ ] Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:` ...). The release notes are generated from them.
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
- [ ] New user-facing text uses i18n keys in `crates/ui/locales/en.json` (no literal strings in UI code).
- [ ] Behaviour changes have tests. Secrets never appear in logs, errors or config files.
