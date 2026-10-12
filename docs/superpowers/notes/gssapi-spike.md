# GSSAPI (Kerberos) feasibility spike

Verdict: **supported-untested**

## Findings

| Item | Result |
|------|--------|
| Driver | `mongodb` 3.6.0 (Cargo.lock) |
| Feature name | `gssapi-auth` (driver); forwarded as `gssapi = ["mongodb/gssapi-auth"]` in `crates/core/Cargo.toml` |
| Compile, default | OK (`AuthMechanism::Gssapi` returns `CoreError::Unsupported`) |
| Compile, `--features gssapi` | OK (maps to the driver's `AuthMechanism::Gssapi`) |
| Clippy `-D warnings`, both configs | clean |
| Tests, both configs | pass (20 default; 19 with feature, as `gssapi_without_feature_is_unsupported` is cfg'd out) |
| Dependency chain | `mongodb` -> `cross-krb5 0.4.2` -> `libgssapi 0.9.1` -> `libgssapi-sys 0.3.4` |
| macOS system library | `/System/Library/Frameworks/GSS.framework` (confirmed by `otool -L` on the test binary built with the feature). No libkrb5 install or Homebrew package needed on macOS. Linux builds would need `libgssapi-krb5` dev headers (out of scope; this app targets macOS). |
| Runtime, real auth | **untested: no KDC available** (no `/etc/krb5.conf`, no ticket cache). |
| Runtime, negative probe | With the feature, `connect()` using Gssapi against `127.0.0.1:1` returned `CoreError::Database("...Server selection timeout... Connection refused")`: bounded, no panic. Done with a throwaway test (appended to `client.rs`, removed afterwards, not committed). |

## UI implication

- `mongostudio_core::GSSAPI_AVAILABLE` (`cfg!(feature = "gssapi")`) is exposed from `lib.rs` and unit tested in both configurations.
- Task 12's connection form must hide the Kerberos option when it is false.
- Recommendation: the app crate should enable `mongostudio-core/gssapi` by default for macOS release builds, since GSS.framework ships with the OS and linking adds no install burden. Before advertising Kerberos as supported, test against a real KDC + Kerberized mongod, so keep it labeled experimental until then.
