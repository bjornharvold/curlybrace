# Contributing to Curlybrace

Thank you for helping. This project is free for everyone and stays that way because people like you
build it. You do not need to be a Rust expert: small fixes, docs, tests and bug reports all help.

- [Ways to contribute](#ways-to-contribute)
- [Set up your machine](#set-up-your-machine)
- [How the code is organised](#how-the-code-is-organised)
- [Making a change](#making-a-change)
- [Adding a feature](#adding-a-feature)
- [Adding a platform](#adding-a-platform)
- [Testing](#testing)
- [Code standards](#code-standards)
- [Commits, pull requests and releases](#commits-pull-requests-and-releases)

## Ways to contribute

- **Report a bug** or request a feature with an [issue](https://github.com/bjornharvold/curlybrace/issues/new/choose).
  Never paste passwords, credentialed connection strings or private keys.
- **Build a feature.** The [roadmap in the README](README.md#roadmap) lists what is planned next
  and what is in the backlog, each with an issue. Comment on the issue before you start so we can
  agree on the shape before you invest time.
- **Support more platforms.** Only macOS arm64 is built today. Intel Mac, Linux and Windows
  builds are very welcome, see [Adding a platform](#adding-a-platform).
- **Improve trust and polish:** Apple code signing and notarization, real screenshots for the
  website, accessibility, translations (see [Code standards](#code-standards)).
- **Fix documentation** or add tests. Both are always useful.

## Set up your machine

You need:

- macOS (the app links the Keychain and GSS frameworks).
- Rust 1.87 or newer: `rustup toolchain install 1.87.0`. CI uses exactly 1.87.0, so use that version
  if clippy results differ.
- A local MongoDB (tier 1) and/or Docker (tier 2), only for the opt-in integration tests.

```sh
git clone https://github.com/bjornharvold/curlybrace.git
cd curlybrace
cargo run --release -p curlybrace     # launch the app
cargo test --workspace                 # unit tests, offline, a few seconds
```

Before you push, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## How the code is organised

A Cargo workspace with four crates (the fourth, `crates/testkit`, is dev-only test support):

| Crate | Path | What lives there |
|-------|------|------------------|
| `curlybrace-core` | `crates/core` | Everything that is not UI: connection model, auth, TLS, the MongoDB driver wrapper (`DocumentStore` trait), query parsing, extended-JSON conversion, CRUD, secrets (Keychain), the SSH tunnel and host-key policy, and the async runtime that the UI talks to through `Command` and `Event` messages. It must not depend on egui. |
| `curlybrace-ui` | `crates/ui` | The egui views, the **pure view-model functions** that hold all decisions (state transitions, click-to-action mapping, validation), the app state and reducer, persistence of the connection list, and the i18n layer (`locales/en.json`). |
| `curlybrace` | `crates/app` | The binary: creates the Tokio runtime, wires the Keychain and the host-key prompt, and starts the window. |

Two ideas shape most changes:

1. **Decisions live in pure functions, views stay thin.** Put logic in `viewmodel/` or the reducer
   where it can be unit tested without a window. egui code should only draw and forward intents.
2. **The runtime is asynchronous and events can arrive late.** Every `Connect` and `Disconnect` bumps
   an epoch, and answers that belong to a replaced session are dropped (`Superseded`). If you add
   a command or event, read the comments in `crates/core/src/runtime` first and keep the
   "exactly one event per command" rule.

The original design and plan are in [docs/superpowers](docs/superpowers) for background.

## Making a change

1. Fork the repository and create a branch from `main`.
2. Write a failing test first, then the code (this repository is developed test-first).
3. Run the checks above.
4. Open a pull request using the template. Explain what changed and why, and how you tested it.

Small, focused pull requests are reviewed fastest.

## Adding a feature

Larger features (anything on the roadmap) follow a short process so reviewers and contributors do
not waste effort:

1. **Open an issue** describing the user problem and your rough approach.
2. **Write a short design** (a markdown file in `docs/superpowers/specs/`): what it does, what it
   deliberately does not do, how data flows, how errors are shown, how it is tested.
3. **Then build it** in small pull requests, each independently useful and tested.

A new feature should fit the existing patterns: the logic in `curlybrace-core` behind a trait so
it can be tested with an in-memory fake, state transitions as pure functions in `curlybrace-ui`,
and a thin egui view on top.

## Adding a platform

Releases are built by `.github/workflows/build-binaries.yml` using a build matrix. Today it has one
row, macOS arm64. To add a platform:

1. **Make it build and run locally** on that platform first. Expect to touch:
   - `crates/core/Cargo.toml`: the `keyring` crate is configured for the macOS Keychain
     (`apple-native`). Add target-specific dependency tables for the Secret Service (Linux) or
     Windows Credential Manager, and keep the macOS one.
   - The `gssapi` feature (Kerberos) links macOS GSS.framework. Make it opt-in or per-platform.
   - Unix-only code such as file permissions (`0600`) for `known_hosts` and the connection list.
     Windows needs an equivalent.
   - Config directories come from the `dirs` crate, so they should already follow platform norms.
   - eframe may need system libraries on Linux (for example X11 or Wayland development packages).
2. **Add a packaging script** in `scripts/`, modelled on `scripts/package-macos.sh`. It receives
   the version and target triple and must write `dist/curlybrace-v<version>-<os>-<arch>.<ext>`
   plus a `.sha256` file next to it.
3. **Add a matrix row** to `build-binaries.yml`:

   ```yaml
   - label: linux-x86_64
     runner: ubuntu-22.04
     target: x86_64-unknown-linux-gnu
     package: scripts/package-linux.sh
   ```

   Add any system packages the build needs as an extra step before `Build`.
4. **Extend CI** (`.github/workflows/ci.yml`) so the new platform is built and tested on every pull
   request, not only at release time.
5. **Update the README** install section and the known-limitations list.

You can try the build without making a release: run **Actions > Build binaries > Run workflow** and
give it an existing tag. It uploads the archives to that release.

Apple code signing and notarization are also welcome: the packaging script currently applies an
ad-hoc signature only. Signing needs a Developer ID certificate and notarization credentials stored
as repository secrets, which only the maintainer can add, so open an issue to coordinate.

## Testing

GitHub runs only the unit tests (and a non-blocking screenshot job); it never starts a database.

```sh
cargo test --workspace          # unit tests, offline, a few seconds: the only thing CI needs
```

Two separate local tiers need a real MongoDB and are never run on GitHub:

```sh
scripts/integration-tests.sh    # tier 1: needs a MongoDB already running (no docker)
scripts/container-tests.sh      # tier 2: docker; creates its own cb-it-* containers
```

Tier 1 connects to `CURLYBRACE_IT_MONGODB_URI` (default `mongodb://127.0.0.1:27017`) and WRITES to
it, but only to throwaway `cbit_*` databases that it drops again: point it at a throwaway server,
never at production. Tier 2 only ever touches containers it created. Screenshot tests need a GPU:

```sh
CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture
```

Add `UPDATE_SNAPSHOTS=1` to deliberately rewrite the committed PNGs. Always pass `--nocapture` so
the reason a gated test skipped (or failed) is visible. Shared helpers live in `crates/testkit`;
`scripts/check-no-db-in-ci.sh` keeps database tests out of the workflows. Details, cleanup and
coverage commands are in [docs/superpowers/notes/testing.md](docs/superpowers/notes/testing.md). The
target is at least 80% line coverage for `curlybrace-core`.

The target lists in those two scripts are written by hand: when you add a `docker_*` or `local_*`
test file, add it to the right script (or to the documented own-container list in
`docs/superpowers/notes/testing.md`), or nothing will ever run it.

If you change a UI label, a button or a flow, also run `scripts/container-tests.sh` (and the tier 1
flows when you have a local MongoDB). Those test binaries are `#[ignore]`d, so the normal
`cargo test --workspace` gate never exercises them; `cargo clippy --workspace --all-targets` only
proves that they still compile.

Prefer real values (real `bson` documents from `mock_objects`) over mocks. Mock only boundaries:
the driver, SSH and the Keychain.

## Code standards

- **No literal user-facing strings in code.** Every label, tooltip, banner and message is a key in
  `crates/ui/locales/en.json` (nested objects, keys in alphabetical order) and is looked up through
  `I18n`. This keeps the app translatable.
- **Immutability.** Prefer returning new values from state transitions over mutating shared state.
  egui widgets editing a view-local buffer are the accepted exception.
- **Errors are typed.** Failures become a `CoreError` variant with a static, secret-free message and
  an i18n key. Never swallow an error, and never put a secret in an error, a log line or a banner.
- **Validate at the boundary:** user-entered URIs, JSON, tunnel settings and file contents.
- **Keep code small:** files under about 800 lines, functions under about 50, shallow nesting.
- **Security-sensitive areas** (secrets, SSH host keys, session handling) get extra review. Say in
  the pull request when you touch them.

## Commits, pull requests and releases

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/), for example:

```
feat(core): add collection diff
fix(ui): keep the editor text when the modal is dismissed
docs: explain the SSH tunnel limits
```

`feat` produces a minor version bump and `fix` a patch bump (while the app is 0.x, a breaking
change also bumps the minor version). Add `!` after the type or a `BREAKING CHANGE:` footer for
breaking changes. `docs`, `test`, `chore`, `ci` and `refactor` do not trigger a release by themselves.

You never edit the version or `CHANGELOG.md` yourself. After changes land on `main`,
[release-please](https://github.com/googleapis/release-please) opens or updates a **release pull
request** with the new version and changelog. When the maintainer merges it, the tag and GitHub
release are created and the build workflow attaches the binaries.

By contributing you agree that your work is licensed under the same terms as the project, either
Apache-2.0 or MIT at the user's option (see [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT)).
