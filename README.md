<p align="center"><img src="assets/icon/app-icon-256.png" width="112" height="112" alt="The app icon: a document between curly braces"></p>

# Curlybrace

**Curlybrace is a free, native MongoDB GUI for macOS.** Browse your MongoDB collections, run queries,
edit documents and compare data between databases, without an account, a trial, or a license fee.

[![CI](https://github.com/bjornharvold/curlybrace/actions/workflows/ci.yml/badge.svg)](https://github.com/bjornharvold/curlybrace/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/bjornharvold/curlybrace?include_prereleases)](https://github.com/bjornharvold/curlybrace/releases/latest)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

**Website:** <https://bjornharvold.github.io/curlybrace/> ·
**Download:** [latest release](https://github.com/bjornharvold/curlybrace/releases/latest) ·
**Changes:** [CHANGELOG.md](CHANGELOG.md)

> **Status: early.** The app is at version 0.x. The core browsing and editing flow is built and
> tested, but it has had little real-world use. Expect rough edges, and keep backups of data you
> edit. See [Known limitations](#known-limitations).

## What it does today

**Connect**
- A connection manager with folders, an edit dialog and Test Connection is being built
  ([#45](https://github.com/bjornharvold/curlybrace/issues/45)).
- Saved connections with SCRAM, X.509 and LDAP (PLAIN) authentication, TLS, replica sets and SRV
  (`mongodb+srv://`) URIs.
- Kerberos (GSSAPI) support is included but **experimental**: it compiles and links against macOS
  GSS.framework but has not been tested against a real KDC.
- SSH tunnel through a bastion host (password or private key). Host keys are checked against an
  app-owned `known_hosts` file. An unknown host asks you to trust it, and a **changed** host key is
  refused outright with an option to forget the old key.
- Passwords and SSH secrets live in the macOS Keychain. They are never written to the
  connections file, logs, or error messages.

**Browse and query**
- Sidebar with databases and collections, loaded on demand.
- View the same page of documents as a **tree**, a **table** or **JSON**, 50 documents per page.
- Query bar with filter, sort, projection, skip and limit. Bad input shows an error and keeps your
  previous results on screen.

**Edit safely**
- Insert, edit, clone and delete documents. Edits use canonical extended JSON, so `Int32` and
  `Int64`, `Decimal128`, binary, dates, `MinKey`/`MaxKey` and the other BSON types keep their type.
- Works with any `_id` type (ObjectId, string, number, UUID, compound).
- Deleting a document or a saved connection asks you to confirm first.
- Pages shown with a projection are read-only for edit and clone, so a partial copy can never
  overwrite a full document.
- Unsaved editor text is kept if you click outside the editor.

**Compare**
- Compare two collections, or two whole databases, on the same server or on different ones. Match
  documents by `_id` or by your own key, and see what is only on one side and what differs, field by
  field.
- Comparing never writes. To change data you tick the differences, review exactly what will be
  written where, and confirm. Every document is checked again just before it is written.
- Copy or delete differences, copy single fields of a changed document, or export the changes as a
  mongosh script or JSON plan and the differences as a CSV report.
- The full guide is [docs/compare.md](docs/compare.md).

## Install

1. Download `curlybrace-vX.Y.Z-macos-arm64.zip` from the
   [latest release](https://github.com/bjornharvold/curlybrace/releases/latest) (Apple Silicon).
2. Optionally check it: `shasum -a 256 -c curlybrace-vX.Y.Z-macos-arm64.zip.sha256`
3. Unzip it and move **Curlybrace.app** to `/Applications`.
4. The app is **not notarized by Apple yet**, so the first launch is blocked by Gatekeeper.
   Right-click the app, choose **Open**, then **Open** again. Or remove the quarantine flag once:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Curlybrace.app"
   ```

Only macOS on Apple Silicon (arm64) is built today. Other platforms are welcome contributions, see
[CONTRIBUTING.md](CONTRIBUTING.md#adding-a-platform).

## Quick start

1. Open the app and add a connection. Enter a URI **without** the password, for example
   `mongodb://localhost:27017`, then pick an authentication method and type the password into its
   own field. It is stored in the Keychain.
2. Select the connection and connect. Expand a database and click a collection.
3. Use the query bar. Filter, sort and projection are JSON:

   | Field      | Example                     |
   |------------|-----------------------------|
   | Filter     | `{"status": "active"}`      |
   | Sort       | `{"createdAt": -1}`         |
   | Projection | `{"name": 1, "email": 1}`   |

4. Right-click a document to edit, clone or delete it. Use the Tree, Table and JSON tabs to change
   how the page is shown.

To see failure details, launch the app from a terminal with logging on (levels: `error`, `warn`,
`info`, `debug`, `trace`; logging is off by default):

```sh
CURLYBRACE_LOG=warn "/Applications/Curlybrace.app/Contents/MacOS/curlybrace"
```

## Known limitations

- macOS on Apple Silicon only, unsigned and not notarized.
- No inline editing in the table or tree. Documents are edited as JSON in the editor.
- Documents shown with a projection cannot be edited or cloned (delete and insert still work).
- Kerberos is untested against a real server.
- A TLS connection through an SSH tunnel is refused unless "allow invalid certificates" is on,
  because the driver cannot verify the certificate against the original host name. Turning that on
  removes certificate protection on the bastion-to-MongoDB leg.
- Only one SSH host per tunnel and single-host URIs are supported with a tunnel (no replica set
  discovery through a tunnel, no `mongodb+srv://`).
- The find bar takes strict JSON, not mongo shell syntax.
- Everyday tools you may expect from other clients (aggregation builder, document import and
  export, index management, schema analysis) are not built yet. They come after the features at
  the top of the [roadmap](#roadmap).

## Roadmap

The project focuses on features that free MongoDB tools do not already cover well, and then fills
in the everyday ones. Each item has an issue with a description and a "done when" checklist, and
each gets its own design and plan before code is written. Items are ordered by priority: **Next**
is the planned work (label `P2`), **Later** is the backlog (`P3`). Every one is a good place to
contribute.

### Built

- **Data Compare & Sync** ([#8](https://github.com/bjornharvold/curlybrace/issues/8)): compare collections or whole databases across servers, copy or
  delete differences with a review step, and export the changes. Nothing is left to build; feedback
  and fixes are welcome. See the [guide](docs/compare.md).

### In progress

- **Connection manager** ([#45](https://github.com/bjornharvold/curlybrace/issues/45)): saved connections with folders and timestamps, a manager list
  with filter and sort, an edit dialog, Test Connection, connection-string import and export, and
  import and export of the connection list without secrets. Its follow-ups are listed below.

### Next

- **Several connections at once** ([#46](https://github.com/bjornharvold/curlybrace/issues/46)) and a **production guard** with colour tags and a
  read-only flag ([#47](https://github.com/bjornharvold/curlybrace/issues/47)).
- **Copy collections and documents between connections** ([#48](https://github.com/bjornharvold/curlybrace/issues/48)).
- **Signing and notarization** ([#17](https://github.com/bjornharvold/curlybrace/issues/17)): planned (`P2`), so the app opens without a
  Gatekeeper warning.
- **Paths for `mongosh`, `mongodump` and `mongorestore`** ([#49](https://github.com/bjornharvold/curlybrace/issues/49)) and a **shell tab** that runs your
  own `mongosh` ([#50](https://github.com/bjornharvold/curlybrace/issues/50)).
- **Bulk update and delete** with a count preview ([#51](https://github.com/bjornharvold/curlybrace/issues/51)).
- **Create, rename and drop** databases, collections and views ([#52](https://github.com/bjornharvold/curlybrace/issues/52)).

### Later

- **Beyond the basics:** Data Masking and Reschema ([#9](https://github.com/bjornharvold/curlybrace/issues/9)), SQL and MongoDB ([#10](https://github.com/bjornharvold/curlybrace/issues/10)), Task
  Scheduler ([#11](https://github.com/bjornharvold/curlybrace/issues/11)).
- **Query and analysis:** aggregation builder ([#67](https://github.com/bjornharvold/curlybrace/issues/67)), explain plans ([#69](https://github.com/bjornharvold/curlybrace/issues/69)), schema analysis
  ([#70](https://github.com/bjornharvold/curlybrace/issues/70)), validation rules ([#71](https://github.com/bjornharvold/curlybrace/issues/71)), query history ([#72](https://github.com/bjornharvold/curlybrace/issues/72)), copy as driver code ([#73](https://github.com/bjornharvold/curlybrace/issues/73)),
  visual query builder ([#74](https://github.com/bjornharvold/curlybrace/issues/74)), query options ([#75](https://github.com/bjornharvold/curlybrace/issues/75)), an opt-in natural-language assistant
  ([#76](https://github.com/bjornharvold/curlybrace/issues/76)), data model diagrams ([#77](https://github.com/bjornharvold/curlybrace/issues/77)), a map view for geo fields ([#79](https://github.com/bjornharvold/curlybrace/issues/79)).
- **Data in and out:** export ([#54](https://github.com/bjornharvold/curlybrace/issues/54)), import ([#55](https://github.com/bjornharvold/curlybrace/issues/55)), backup and restore ([#53](https://github.com/bjornharvold/curlybrace/issues/53)), test data
  generator ([#58](https://github.com/bjornharvold/curlybrace/issues/58)), GridFS ([#57](https://github.com/bjornharvold/curlybrace/issues/57)).
- **Administration and monitoring:** index management ([#68](https://github.com/bjornharvold/curlybrace/issues/68)), users and roles ([#56](https://github.com/bjornharvold/curlybrace/issues/56)), live
  server statistics ([#59](https://github.com/bjornharvold/curlybrace/issues/59)), profiler ([#60](https://github.com/bjornharvold/curlybrace/issues/60)), replica set and sharded cluster views ([#61](https://github.com/bjornharvold/curlybrace/issues/61)),
  change streams ([#62](https://github.com/bjornharvold/curlybrace/issues/62)).
- **Enterprise and security:** field level encryption and Queryable Encryption ([#63](https://github.com/bjornharvold/curlybrace/issues/63)), AWS IAM and
  OIDC sign-in ([#64](https://github.com/bjornharvold/curlybrace/issues/64)), a Kerberos test against a real KDC ([#65](https://github.com/bjornharvold/curlybrace/issues/65)), audit log viewer ([#66](https://github.com/bjornharvold/curlybrace/issues/66)),
  Atlas Search indexes ([#78](https://github.com/bjornharvold/curlybrace/issues/78)).
- **Connection manager follow-ups:** remaining edit tabs ([#81](https://github.com/bjornharvold/curlybrace/issues/81)), start screen with recents
  ([#82](https://github.com/bjornharvold/curlybrace/issues/82)), nested folders ([#83](https://github.com/bjornharvold/curlybrace/issues/83)), export with secrets ([#84](https://github.com/bjornharvold/curlybrace/issues/84)).
- **Platforms:** [Intel Mac](https://github.com/bjornharvold/curlybrace/issues/14), [Linux](https://github.com/bjornharvold/curlybrace/issues/15), [Windows](https://github.com/bjornharvold/curlybrace/issues/16).
- **Trust and polish:** [Homebrew install](https://github.com/bjornharvold/curlybrace/issues/24) (blocked on
  signing), [inline table editing](https://github.com/bjornharvold/curlybrace/issues/12), [`ssh-agent` support](https://github.com/bjornharvold/curlybrace/issues/13), [TLS test containers](https://github.com/bjornharvold/curlybrace/issues/18),
  [real screenshots](https://github.com/bjornharvold/curlybrace/issues/20), [translations](https://github.com/bjornharvold/curlybrace/issues/21), [command palette](https://github.com/bjornharvold/curlybrace/issues/80).

Browse everything by the [`help wanted`](https://github.com/bjornharvold/curlybrace/issues/?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22) and
[`good first issue`](https://github.com/bjornharvold/curlybrace/issues/?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) labels, or by priority:
[`P2`](https://github.com/bjornharvold/curlybrace/issues/?q=is%3Aissue+is%3Aopen+label%3AP2) and
[`P3`](https://github.com/bjornharvold/curlybrace/issues/?q=is%3Aissue+is%3Aopen+label%3AP3).
The comparison with other MongoDB tools behind this list is in [#38](https://github.com/bjornharvold/curlybrace/issues/38).

## Build from source

You need macOS and Rust 1.87 or newer (see `rust-version` in `Cargo.toml`).

```sh
git clone https://github.com/bjornharvold/curlybrace.git
cd curlybrace
cargo run --release -p curlybrace      # build and launch
cargo test --workspace                  # unit tests (no network or docker needed)
```

Integration tests need Docker. See [docs/superpowers/notes/testing.md](docs/superpowers/notes/testing.md).

## Contributing

Contributions of every size are welcome, from a typo to a whole feature or a new platform. Start
with [CONTRIBUTING.md](CONTRIBUTING.md). In short:

- **Add a feature.** Pick something from the roadmap or open an issue to discuss your own idea first.
- **Support another platform.** Releases are built by a GitHub Actions matrix, and adding a row
  plus a packaging script is the whole job. The guide walks through it.
- **Fix bugs and improve docs.** Small pull requests are easy to review and get merged fast.

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) because the
changelog and version numbers are generated from them.

## Releases and versioning

Versions follow [Semantic Versioning](https://semver.org/) (0.x while the app is young). Releases
are automated with [release-please](https://github.com/googleapis/release-please): merging to `main`
keeps a release pull request up to date; merging that pull request bumps the version, updates
[CHANGELOG.md](CHANGELOG.md), tags `vX.Y.Z`, creates the GitHub release and attaches the
macOS arm64 app as a zip with its SHA-256 checksum.

## Security

Please report vulnerabilities privately, see [SECURITY.md](SECURITY.md).

## Design documents

The original design and implementation plan live in
[docs/superpowers/specs](docs/superpowers/specs) and [docs/superpowers/plans](docs/superpowers/plans).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project, as defined in the Apache-2.0 license, is dual licensed as above, without
any additional terms or conditions.

## Disclaimer

Curlybrace is an independent open-source project. It is not affiliated with, endorsed by, or
sponsored by MongoDB, Inc. "MongoDB" is a registered trademark of MongoDB, Inc. The app is built
from public documentation and open-source libraries and contains no proprietary code.
