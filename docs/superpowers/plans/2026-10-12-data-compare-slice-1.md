# Data Compare & Sync, Slice 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The compare engine, a pair-aware compare session in the runtime, and a read-only Compare workspace in the UI (a single pair), so two collections can be compared and their differences reviewed without writing anything to a database.

**Architecture:** A new `curlybrace-core::compare` module holds pure, separately tested units (BSON key comparator, normalization, fingerprints, field diff, keyset scanner, merge-join engine). `DocumentStore` gains four read methods. The runtime gains a wrapped `Command::Compare(..)` / `Event::Compare(..)` protocol: a compare *session* owns two connections and runs pairs one at a time with isolated failures and a shared result budget. The UI gains a Compare workspace built from pure view-models and thin egui views, with state held in the existing reducer.

**Tech Stack:** Rust 1.87, `bson` 2.15 (the driver's own copy), `mongodb` 3.6, `sha2` 0.10 (already in the tree), `proptest` (dev only), `egui`/`egui_kittest` 0.32 for the UI tests.

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` (approved 2026-10-12). Issue #8.

## Spec addenda found while planning

Two points the approved spec did not settle. Both are decided here and They are written back into the spec in the same PR as this plan, so the spec and the code never disagree.

1. **`scan_sorted` is inclusive.** The spec says "strictly greater than `after`". That cannot detect a duplicate key that straddles a batch boundary: the second document with the key would be skipped by the query and never seen. So the contract is "key at or after `from`", and the scanner expects the previous batch's last document back as the first row, discards it, and treats a second row with the same key as `CompareDuplicateKey`.
2. **Keyset queries must be type-aware.** MongoDB's `$gt`/`$gte` only match values of the same BSON type class (numbers with numbers, strings with strings), but the sort order is total across types. A plain `$gt` keyset would silently skip everything after a change of type (for example an `_id` that is an ObjectId in some documents and a string in others). The keyset filter therefore adds, for each key field, "or the field's type class ranks higher". This is covered by a real-server test with mixed key types.
3. **New error `CompareKeyInvalid`.** A key value that is an array (the server sorts arrays by their smallest element, which breaks strict ordering) or a deprecated type (Undefined, Symbol, DbPointer, JavaScript, JavaScriptWithScope) is refused with this typed error. It joins the other new compare errors in the spec's error table.

## Global Constraints

- Target: macOS arm64 build; the crates are `curlybrace-core`, `curlybrace-ui`, `curlybrace` (bin), plus the dev-only `curlybrace-testkit`. `curlybrace-core` MUST NOT depend on egui/eframe.
- **Batch size** default `1_000` documents per scan batch; **result budget** default `max_differences = 1_000_000` and `max_result_bytes = 256 MiB` (estimated as key bytes plus 40 per entry), shared by all pairs of a session; `max_unindexed_documents = 50_000` per side.
- **Collation:** every keyset scan forces `collation: { locale: "simple" }` and `allowDiskUse: true`.
- **Fingerprint:** the first 16 bytes of SHA-256 over the canonical BSON bytes of the normalized document.
- **No document data is written to disk** by this feature. Results are keys and fingerprints in memory only. Logs carry job ids, counts, durations and error kinds, never keys, field values or documents.
- Secrets come only from the Keychain; compare sessions use the existing `Connector`, secret store, SSH tunnel and host-key gate.
- **Event rule:** every command still gets exactly one answer event; every pair gets exactly one terminal event; every run gets exactly one run-terminal event, after all of that run's pair terminals.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical); failures are typed `CoreError` variants with `errors.*` keys and static, secret-free messages.
- Immutability: new values, not in-place mutation of shared state (egui widget edits of view-local buffers are the accepted exception).
- Files < 800 lines (aim for 200-400), functions < 50 lines, nesting <= 4.
- TDD for every task (red, green, refactor); core coverage stays >= 80%. Prefer real `bson` documents over mocks; mock only the driver, SSH and Keychain boundaries.
- Default `cargo test --workspace` stays offline, database-free, docker-free and GPU-free. Anything needing a server is `#[ignore]` and env-gated (tier 1 local server, tier 2 docker, snapshots); GitHub CI never runs those (the policy guard enforces it).
- Commits: conventional format (`feat(compare): ...`), each ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Work on a branch, open a PR per task or per small group of tasks, and run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` before every push.
- Sub-agents default to the Sonnet model; the final whole-feature review uses a stronger model.

## Review Focus

Failure modes the spec implies but no task's tests would otherwise exercise, most likely first:

1. **Comparator disagrees with the server** on boundary values (Int64 above 2^53, Decimal128 with different exponents, `NaN`, `-0.0`, multibyte strings, sub-document field order). Expected: unit tests per pair, and a real-server conformance test; a mismatch must fail loudly. Pinned in Tasks 1 and 4.
2. **Duplicate keys that hide**: two documents whose keys are equal across numeric types (`Int32 1` and `Int64 1`), or that straddle a batch boundary, or sit at the very first or last row. Expected: `CompareDuplicateKey`, never a silent skip. Pinned in Task 5.
3. **Mixed-type keys** (an `_id` that is an ObjectId in some documents and a string in others) must page through completely, in server order. Pinned in Tasks 4 and 5.
4. **Missing, null, array or deprecated-type key values** produce typed errors (or, for a single null, work), never a panic or a wrong match. Pinned in Tasks 1 and 5.
5. **Budget leaks and stale answers**: discarding or dropping a pair's results must return its share of the budget, and answers for a closed or discarded job must be ignored. Expected: budget back to zero after drop; reducer drops unknown job ids. Pinned in Tasks 5, 8 and 9.
6. **Cancel and failure at every stage** (open, check, mid-scan, between pairs) leave every pair with exactly one terminal event, exactly one run terminal, and no leaked connection or tunnel. Pinned in Tasks 7 and 8.

## File Structure

```
crates/core/src/
  error.rs                        + 6 compare variants and i18n keys                 (Task 1)
  compare/mod.rs                  module root, re-exports                           (Task 1)
  compare/number.rs               exact numeric comparison (Int/Double/Decimal128)  (Task 1)
  compare/key.rs                  KeySpec, KeyTuple, compare_bson, extract_key      (Task 1)
  compare/rules.rs                CompareRules, FieldScope                          (Task 2)
  compare/normalize.rs            canonical form under the rules                    (Task 2)
  compare/fingerprint.rs          128-bit fingerprint                               (Task 2)
  compare/diff.rs                 field-level diff                                  (Task 3)
  compare/keyset.rs               keyset filter and index-usability (pure)          (Task 4)
  store.rs                        + IndexStatus, CollectionKind, 4 trait methods    (Task 4)
  mongo_store.rs                  + the four methods on the driver                  (Task 4)
  test_support/memory_store.rs    + the four methods in memory                      (Task 4)
  test_support/bson_order.rs      delegates to compare::key::compare_bson           (Task 1)
  compare/scan.rs                 KeyScanner                                        (Task 5)
  compare/results.rs              Budget, ResultSet, DiffEntry, Category            (Task 5)
  compare/engine.rs               merge-join, summary, progress, cancel             (Task 6)
  compare/protocol.rs             PairSpec, CompareRun, commands, events            (Task 7)
  runtime/compare/mod.rs          session registry, open/list/check/close           (Task 7)
  runtime/compare/runner.rs       pair runner, cancel, rows, detail, discard        (Task 8)
  runtime/mod.rs                  Command::Compare / Event::Compare wiring          (Task 7)
crates/core/tests/
  local_compare_store.rs          tier 1: conformance, keyset, index, kinds         (Task 4)
  local_compare_session.rs        tier 1: two-database compare end to end           (Task 8)
crates/ui/src/
  app/compare/                    state, reducer, actions for the workspace         (Task 9)
  viewmodel/compare/              setup form, results, detail, selection            (Task 9)
  views/compare/                  setup, preflight, running, results, detail        (Task 10)
  app/state.rs, reducer.rs, panels.rs, mod.rs   workspace switch and wiring        (Tasks 9, 10)
  locales/en.json                 compare.* keys                                    (Tasks 1, 9, 10)
crates/ui/tests/
  local_compare_flows.rs          click-driven compare flows                        (Task 11)
  snapshots.rs                    + compare views                                   (Task 11)
docs/superpowers/specs/...        the two addenda written back                      (Task 11)
```

---

### Task 1: Compare errors, exact numeric comparison and the BSON key comparator

**Files:**
- Create: `crates/core/src/compare/mod.rs`, `crates/core/src/compare/number.rs`, `crates/core/src/compare/key.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/error.rs`, `crates/core/src/test_support/bson_order.rs`, `crates/ui/locales/en.json`, `crates/ui/src/error_text.rs`, `crates/ui/src/i18n.rs`

**Interfaces:**
- Consumes: `CoreError`, `Result`.
- Produces:
  ```rust
  // error.rs: six unit variants, each with a static message and an i18n key
  CoreError::{CompareDuplicateKey, CompareKeyMissing, CompareKeyInvalid, CompareKeyOrder, CompareKeyNotIndexed, CompareCapExceeded}
  // compare/key.rs
  pub struct KeySpec { pub fields: Vec<String> }          // Clone, Debug, PartialEq, Eq, Serialize, Deserialize
  impl KeySpec { pub fn id() -> Self; pub fn new(fields: Vec<String>) -> Result<Self>; pub fn contains_id(&self) -> bool }
  pub struct KeyTuple(pub Vec<Bson>);                     // Clone, Debug, PartialEq
  pub fn compare_bson(a: &Bson, b: &Bson) -> std::cmp::Ordering;
  pub fn compare_tuples(a: &KeyTuple, b: &KeyTuple) -> std::cmp::Ordering;
  pub fn extract_key(doc: &Document, spec: &KeySpec) -> Result<KeyTuple>;
  // compare/number.rs (crate-private)
  pub(crate) enum Dec { NaN, NegInf, PosInf, Finite { neg: bool, digits: Vec<u8>, exp: i64 } }   // Ord
  pub(crate) fn compare_numbers(a: &Bson, b: &Bson) -> Ordering;
  impl Dec { pub(crate) fn of(d: &Decimal128) -> Dec; pub(crate) fn to_i64(&self) -> Option<i64>; pub(crate) fn to_decimal128(&self) -> Option<Decimal128> }
  ```

- [ ] **Step 1: Write the failing error test.** In `crates/core/src/error.rs`, in the test `every_variant_has_a_distinct_i18n_key`, add these six variants to the `errors` array (after `CoreError::TlsThroughTunnel`):

  ```rust
            CoreError::CompareDuplicateKey,
            CoreError::CompareKeyMissing,
            CoreError::CompareKeyInvalid,
            CoreError::CompareKeyOrder,
            CoreError::CompareKeyNotIndexed,
            CoreError::CompareCapExceeded,
  ```

- [ ] **Step 2: Run it and watch it fail.**
  Run: `cargo test -p curlybrace-core error::tests -- --nocapture`
  Expected: FAIL to compile, "no variant named `CompareDuplicateKey`".

- [ ] **Step 3: Add the variants.** In `crates/core/src/error.rs`, add to the enum after `TlsThroughTunnel`:

  ```rust
    /// Two documents on one side of a compare share a key tuple.
    #[error("compare: two documents share the same key")]
    CompareDuplicateKey,
    /// A document has no value for one of the key fields.
    #[error("compare: a document is missing a key field")]
    CompareKeyMissing,
    /// A key field holds a value that cannot serve as a key (an array or a deprecated type).
    #[error("compare: a key field holds a value that cannot be used as a key")]
    CompareKeyInvalid,
    /// A side's keyed stream was not strictly increasing: the comparator and the server disagree.
    #[error("compare: documents arrived out of key order")]
    CompareKeyOrder,
    /// No usable index covers the key and the collection is above the unindexed limit.
    #[error("compare: no usable index covers the key")]
    CompareKeyNotIndexed,
    /// The shared result budget (differences or bytes) was exhausted.
    #[error("compare: the result limit was reached")]
    CompareCapExceeded,
  ```

  and to `i18n_key`:

  ```rust
            Self::CompareDuplicateKey => "errors.compare_duplicate_key",
            Self::CompareKeyMissing => "errors.compare_key_missing",
            Self::CompareKeyInvalid => "errors.compare_key_invalid",
            Self::CompareKeyOrder => "errors.compare_key_order",
            Self::CompareKeyNotIndexed => "errors.compare_key_not_indexed",
            Self::CompareCapExceeded => "errors.compare_cap_exceeded",
  ```

  In `crates/ui/locales/en.json`, add these keys to the `errors` object, keeping it alphabetical (they sort between `auth_failed` and `database`):

  ```json
    "compare_cap_exceeded": "There are more differences than can be kept in memory, so the comparison stopped. Narrow it with a filter, or compare fewer fields.",
    "compare_duplicate_key": "Two documents on one side have the same key, so they cannot be matched. Choose a key that is unique, or look up which values repeat.",
    "compare_key_invalid": "A key field holds a value that cannot be used as a key (for example an array). Choose a different key.",
    "compare_key_missing": "A document has no value for one of the key fields. Choose key fields that every document has.",
    "compare_key_not_indexed": "No index covers this key, and the collection is too large to compare without one. Create an index on the key fields and try again.",
    "compare_key_order": "The documents did not arrive in the expected key order, so the comparison was stopped to avoid wrong results. This can happen with an unusual collation; please report it if it persists.",
  ```

  In `crates/ui/src/error_text.rs`, add the six variants to the `hidden` array in the test `only_user_input_and_server_messages_have_a_visible_detail`. In `crates/ui/src/i18n.rs`, add the six variants to the list in the completeness test (the array that already contains `CoreError::TlsThroughTunnel`). Compare errors never show raw detail, so `error_detail` needs no change.

- [ ] **Step 4: Run the error and locale tests.**
  Run: `cargo test -p curlybrace-core error::tests && cargo test -p curlybrace-ui error_text i18n`
  Expected: PASS.

- [ ] **Step 5: Write the failing number tests.** Create `crates/core/src/compare/number.rs` containing only the test module below (the implementation comes in Step 7; the file will not compile until then, which is the red state):

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use bson::Decimal128;
      use std::cmp::Ordering::{Equal, Greater, Less};
      use std::str::FromStr;

      fn dec(s: &str) -> Bson {
          Bson::Decimal128(Decimal128::from_str(s).expect("valid decimal"))
      }

      #[test]
      fn equal_values_of_different_types_compare_equal() {
          let one = [
              Bson::Int32(1),
              Bson::Int64(1),
              Bson::Double(1.0),
              dec("1"),
              dec("1.00"),
              dec("0.1E+1"),
          ];
          for a in &one {
              for b in &one {
                  assert_eq!(compare_numbers(a, b), Equal, "{a:?} vs {b:?}");
              }
          }
          assert_eq!(compare_numbers(&dec("1E+3"), &Bson::Int32(1000)), Equal);
          assert_eq!(compare_numbers(&dec("-0"), &Bson::Int32(0)), Equal);
          assert_eq!(compare_numbers(&Bson::Double(-0.0), &Bson::Double(0.0)), Equal);
          assert_eq!(compare_numbers(&Bson::Double(-0.0), &Bson::Int64(0)), Equal);
      }

      #[test]
      fn int64_is_compared_exactly_above_two_to_the_53() {
          let two53: i64 = 1 << 53;
          assert_eq!(
              compare_numbers(&Bson::Int64(two53 + 1), &Bson::Double(two53 as f64)),
              Greater
          );
          assert_eq!(
              compare_numbers(&Bson::Int64(two53), &Bson::Double(two53 as f64)),
              Equal
          );
          assert_eq!(
              compare_numbers(&Bson::Int64(i64::MAX), &Bson::Double(9_223_372_036_854_775_808.0)),
              Less
          );
          assert_eq!(
              compare_numbers(&Bson::Int64(i64::MIN), &Bson::Double(-9_223_372_036_854_775_808.0)),
              Equal
          );
          assert_eq!(
              compare_numbers(&Bson::Int64(i64::MIN), &Bson::Double(f64::NEG_INFINITY)),
              Greater
          );
          assert_eq!(compare_numbers(&Bson::Int64(0), &Bson::Double(0.5)), Less);
          assert_eq!(compare_numbers(&Bson::Int64(0), &Bson::Double(-0.5)), Greater);
      }

      #[test]
      fn nan_is_below_every_number_and_equals_itself() {
          let nan = Bson::Double(f64::NAN);
          assert_eq!(compare_numbers(&nan, &Bson::Double(f64::NAN)), Equal);
          assert_eq!(compare_numbers(&nan, &Bson::Double(f64::NEG_INFINITY)), Less);
          assert_eq!(compare_numbers(&nan, &Bson::Int64(i64::MIN)), Less);
          assert_eq!(compare_numbers(&nan, &dec("NaN")), Equal);
          assert_eq!(compare_numbers(&dec("NaN"), &dec("-Infinity")), Less);
          assert_eq!(compare_numbers(&Bson::Int32(0), &nan), Greater);
      }

      #[test]
      fn decimals_compare_by_exact_value() {
          assert_eq!(compare_numbers(&dec("1.5"), &dec("1.25")), Greater);
          assert_eq!(compare_numbers(&dec("1"), &dec("1.5")), Less);
          assert_eq!(compare_numbers(&dec("-1.5"), &dec("-1.25")), Less);
          assert_eq!(compare_numbers(&dec("-1"), &dec("0")), Less);
          assert_eq!(compare_numbers(&dec("0.001"), &dec("0.0001")), Greater);
          assert_eq!(compare_numbers(&dec("1E+6144"), &dec("9.999999999999999999999999999999999E+6143")), Greater);
          assert_eq!(compare_numbers(&dec("1E-6176"), &dec("0")), Greater);
          assert_eq!(compare_numbers(&dec("Infinity"), &Bson::Int64(i64::MAX)), Greater);
          assert_eq!(compare_numbers(&dec("-Infinity"), &Bson::Int64(i64::MIN)), Less);
      }

      #[test]
      fn a_double_is_compared_to_a_decimal_by_its_exact_binary_value() {
          // 0.1 as a double is slightly MORE than one tenth, so the decimal 0.1 is smaller.
          assert_eq!(compare_numbers(&dec("0.1"), &Bson::Double(0.1)), Less);
          assert_eq!(compare_numbers(&Bson::Double(0.1), &dec("0.1")), Greater);
          // 0.5 is exactly representable.
          assert_eq!(compare_numbers(&dec("0.5"), &Bson::Double(0.5)), Equal);
          assert_eq!(compare_numbers(&Bson::Double(1e30), &dec("1E+30")), Greater);
          assert_eq!(compare_numbers(&Bson::Double(5e-324), &dec("0")), Greater);
      }

      #[test]
      fn dec_converts_to_i64_only_when_integral_and_in_range() {
          assert_eq!(Dec::of(&Decimal128::from_str("12").unwrap()).to_i64(), Some(12));
          assert_eq!(Dec::of(&Decimal128::from_str("1.20E+1").unwrap()).to_i64(), Some(12));
          assert_eq!(Dec::of(&Decimal128::from_str("-0").unwrap()).to_i64(), Some(0));
          assert_eq!(Dec::of(&Decimal128::from_str("1.5").unwrap()).to_i64(), None);
          assert_eq!(
              Dec::of(&Decimal128::from_str("9223372036854775807").unwrap()).to_i64(),
              Some(i64::MAX)
          );
          assert_eq!(Dec::of(&Decimal128::from_str("9223372036854775808").unwrap()).to_i64(), None);
          assert_eq!(
              Dec::of(&Decimal128::from_str("-9223372036854775808").unwrap()).to_i64(),
              Some(i64::MIN)
          );
          assert_eq!(Dec::of(&Decimal128::from_str("NaN").unwrap()).to_i64(), None);
      }

      #[test]
      fn dec_round_trips_through_a_canonical_decimal128() {
          for s in ["1.5", "1.50", "-0.001", "1E+3", "123456789012345678901234567890", "1E-6176"] {
              let d = Dec::of(&Decimal128::from_str(s).unwrap());
              let back = d.to_decimal128().expect("finite values convert");
              assert_eq!(Dec::of(&back), d, "{s}");
          }
          // Equal values written differently normalise to the same Decimal128 bytes.
          let a = Dec::of(&Decimal128::from_str("1.5").unwrap()).to_decimal128().unwrap();
          let b = Dec::of(&Decimal128::from_str("1.50").unwrap()).to_decimal128().unwrap();
          assert_eq!(a.bytes(), b.bytes());
      }
  }
  ```

- [ ] **Step 6: Run and watch it fail.**
  Run: `cargo test -p curlybrace-core compare::number`
  Expected: FAIL to compile (`compare_numbers`, `Dec` not found). Add `pub mod compare;` to `crates/core/src/lib.rs` and create `crates/core/src/compare/mod.rs` with `pub mod key; pub(crate) mod number;` first so the failure is about the missing items, not the module path (create empty `key.rs` too).

- [ ] **Step 7: Implement `number.rs`.** Put this ABOVE the test module:

  ```rust
  //! Exact numeric comparison across Int32, Int64, Double and Decimal128, the way MongoDB orders
  //! numbers: by mathematical value, with NaN below every other number (and equal to itself) and
  //! `-0 == 0`. Int/Int, Double/Double and Int/Double take allocation-free fast paths; Decimal128
  //! values are compared as normalised decimals, and a Double meeting a Decimal128 is expanded to
  //! its exact decimal value (a rare path).

  use std::cmp::Ordering;

  use bson::{Bson, Decimal128};

  /// A decimal in canonical form: `digits` has no leading or trailing zeros (zero has none at all)
  /// and the value is `digits × 10^exp`.
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub(crate) enum Dec {
      NaN,
      NegInf,
      PosInf,
      Finite { neg: bool, digits: Vec<u8>, exp: i64 },
  }

  impl Dec {
      fn zero() -> Self {
          Self::Finite { neg: false, digits: Vec::new(), exp: 0 }
      }

      /// Strips leading and trailing zeros; `-0` becomes `0`.
      fn normalized(neg: bool, mut digits: Vec<u8>, mut exp: i64) -> Self {
          let leading = digits.iter().take_while(|d| **d == 0).count();
          digits.drain(..leading);
          if digits.is_empty() {
              return Self::zero();
          }
          while digits.last() == Some(&0) {
              digits.pop();
              exp += 1;
          }
          Self::Finite { neg, digits, exp }
      }

      fn parse(text: &str) -> Option<Self> {
          let text = text.trim();
          match text.to_ascii_lowercase().as_str() {
              "nan" | "+nan" | "-nan" | "snan" | "-snan" => return Some(Self::NaN),
              "infinity" | "+infinity" | "inf" | "+inf" => return Some(Self::PosInf),
              "-infinity" | "-inf" => return Some(Self::NegInf),
              _ => {}
          }
          let (neg, rest) = match text.strip_prefix('-') {
              Some(r) => (true, r),
              None => (false, text.strip_prefix('+').unwrap_or(text)),
          };
          let (mantissa, exp_part) = match rest.split_once(['e', 'E']) {
              Some((m, e)) => (m, e.parse::<i64>().ok()?),
              None => (rest, 0),
          };
          let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
          if int_part.is_empty() && frac_part.is_empty() {
              return None;
          }
          if !int_part.bytes().chain(frac_part.bytes()).all(|b| b.is_ascii_digit()) {
              return None;
          }
          let digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).map(|b| b - b'0').collect();
          Some(Self::normalized(neg, digits, exp_part - frac_part.len() as i64))
      }

      pub(crate) fn of(d: &Decimal128) -> Self {
          // A valid Decimal128 always prints in a form `parse` understands.
          Self::parse(&d.to_string()).unwrap_or(Self::NaN)
      }

      fn from_i64(n: i64) -> Self {
          let digits = n.unsigned_abs().to_string().bytes().map(|b| b - b'0').collect();
          Self::normalized(n < 0, digits, 0)
      }

      /// The exact decimal value of a finite double, expanded from its binary form.
      fn from_f64(d: f64) -> Self {
          if d.is_nan() {
              return Self::NaN;
          }
          if d.is_infinite() {
              return if d > 0.0 { Self::PosInf } else { Self::NegInf };
          }
          if d == 0.0 {
              return Self::zero();
          }
          let bits = d.to_bits();
          let neg = bits >> 63 == 1;
          let exp_bits = ((bits >> 52) & 0x7ff) as i64;
          let frac = bits & ((1u64 << 52) - 1);
          let (mantissa, e2) = if exp_bits == 0 {
              (frac, -1074)
          } else {
              (frac | (1u64 << 52), exp_bits - 1075)
          };
          let mut digits_le = digits_le(mantissa);
          let exp10 = if e2 >= 0 {
              for _ in 0..e2 {
                  mul_small(&mut digits_le, 2);
              }
              0
          } else {
              for _ in 0..(-e2) {
                  mul_small(&mut digits_le, 5);
              }
              e2
          };
          digits_le.reverse();
          Self::normalized(neg, digits_le, exp10)
      }

      /// The value as an `i64`, only when it is integral and in range.
      pub(crate) fn to_i64(&self) -> Option<i64> {
          let Self::Finite { neg, digits, exp } = self else {
              return None;
          };
          if digits.is_empty() {
              return Some(0);
          }
          if *exp < 0 {
              return None;
          }
          let total = digits.len() as i64 + exp;
          if total > 19 {
              return None;
          }
          let mut magnitude: u128 = 0;
          for d in digits {
              magnitude = magnitude * 10 + u128::from(*d);
          }
          for _ in 0..*exp {
              magnitude *= 10;
          }
          if *neg {
              (magnitude <= 1u128 << 63).then(|| (-(magnitude as i128)) as i64)
          } else {
              i64::try_from(magnitude).ok()
          }
      }

      /// A canonical `Decimal128` for a finite value (equal values give identical bytes).
      pub(crate) fn to_decimal128(&self) -> Option<Decimal128> {
          let (neg, digits, exp) = match self {
              Self::Finite { neg, digits, exp } => (*neg, digits, *exp),
              _ => return None,
          };
          let text = if digits.is_empty() {
              "0".to_string()
          } else {
              let ds: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
              format!("{}{ds}E{exp}", if neg { "-" } else { "" })
          };
          text.parse().ok()
      }

      fn class(&self) -> u8 {
          match self {
              Self::NaN => 0,
              Self::NegInf => 1,
              Self::Finite { .. } => 2,
              Self::PosInf => 3,
          }
      }
  }

  fn digits_le(mut n: u64) -> Vec<u8> {
      if n == 0 {
          return vec![0];
      }
      let mut out = Vec::new();
      while n > 0 {
          out.push((n % 10) as u8);
          n /= 10;
      }
      out
  }

  /// Multiplies a little-endian decimal digit vector by a small factor.
  fn mul_small(digits: &mut Vec<u8>, factor: u32) {
      let mut carry = 0u32;
      for d in digits.iter_mut() {
          let v = u32::from(*d) * factor + carry;
          *d = (v % 10) as u8;
          carry = v / 10;
      }
      while carry > 0 {
          digits.push((carry % 10) as u8);
          carry /= 10;
      }
  }

  impl Ord for Dec {
      fn cmp(&self, other: &Self) -> Ordering {
          let (Self::Finite { neg: n1, digits: d1, exp: e1 }, Self::Finite { neg: n2, digits: d2, exp: e2 }) =
              (self, other)
          else {
              return self.class().cmp(&other.class());
          };
          let sign = |neg: bool, digits: &[u8]| -> i8 {
              if digits.is_empty() {
                  0
              } else if neg {
                  -1
              } else {
                  1
              }
          };
          let (s1, s2) = (sign(*n1, d1), sign(*n2, d2));
          if s1 != s2 {
              return s1.cmp(&s2);
          }
          if s1 == 0 {
              return Ordering::Equal;
          }
          let adjusted = |digits: &[u8], exp: i64| exp + digits.len() as i64 - 1;
          let magnitude = adjusted(d1, *e1)
              .cmp(&adjusted(d2, *e2))
              .then_with(|| d1.cmp(d2));
          if s1 > 0 { magnitude } else { magnitude.reverse() }
      }
  }

  impl PartialOrd for Dec {
      fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
          Some(self.cmp(other))
      }
  }

  enum Num<'a> {
      Int(i64),
      Double(f64),
      Decimal(&'a Decimal128),
  }

  fn num(b: &Bson) -> Option<Num<'_>> {
      match b {
          Bson::Int32(n) => Some(Num::Int(i64::from(*n))),
          Bson::Int64(n) => Some(Num::Int(*n)),
          Bson::Double(d) => Some(Num::Double(*d)),
          Bson::Decimal128(d) => Some(Num::Decimal(d)),
          _ => None,
      }
  }

  fn cmp_f64(a: f64, b: f64) -> Ordering {
      match (a.is_nan(), b.is_nan()) {
          (true, true) => Ordering::Equal,
          (true, false) => Ordering::Less,
          (false, true) => Ordering::Greater,
          (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
      }
  }

  /// Exact comparison of an `i64` with a double, without converting the integer to a float.
  fn cmp_i64_f64(n: i64, d: f64) -> Ordering {
      if d.is_nan() {
          return Ordering::Greater;
      }
      if d >= 9_223_372_036_854_775_808.0 {
          return Ordering::Less;
      }
      if d < -9_223_372_036_854_775_808.0 {
          return Ordering::Greater;
      }
      let truncated = d.trunc();
      // In range and integral, so the cast is exact.
      match n.cmp(&(truncated as i64)) {
          Ordering::Equal => {
              let fraction = d - truncated;
              if fraction > 0.0 {
                  Ordering::Less
              } else if fraction < 0.0 {
                  Ordering::Greater
              } else {
                  Ordering::Equal
              }
          }
          other => other,
      }
  }

  /// Compares two numeric `Bson` values. Callers pass numbers only; anything else compares equal.
  pub(crate) fn compare_numbers(a: &Bson, b: &Bson) -> Ordering {
      use Num::{Decimal, Double, Int};
      let (Some(x), Some(y)) = (num(a), num(b)) else {
          return Ordering::Equal;
      };
      match (x, y) {
          (Int(p), Int(q)) => p.cmp(&q),
          (Double(p), Double(q)) => cmp_f64(p, q),
          (Int(n), Double(d)) => cmp_i64_f64(n, d),
          (Double(d), Int(n)) => cmp_i64_f64(n, d).reverse(),
          (Decimal(p), Decimal(q)) => Dec::of(p).cmp(&Dec::of(q)),
          (Decimal(p), Int(n)) => Dec::of(p).cmp(&Dec::from_i64(n)),
          (Int(n), Decimal(q)) => Dec::from_i64(n).cmp(&Dec::of(q)),
          (Decimal(p), Double(d)) => Dec::of(p).cmp(&Dec::from_f64(d)),
          (Double(d), Decimal(q)) => Dec::from_f64(d).cmp(&Dec::of(q)),
      }
  }
  ```

- [ ] **Step 8: Run the number tests.**
  Run: `cargo test -p curlybrace-core compare::number`
  Expected: PASS (7 tests). If `dec_converts_to_i64...` or the `0.1`-vs-double test fails, fix the implementation, not the test.

- [ ] **Step 9: Write the failing key tests.** Create `crates/core/src/compare/key.rs` containing only this test module (implementation in Step 11):

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use bson::oid::ObjectId;
      use bson::spec::BinarySubtype;
      use bson::{Binary, DateTime, Decimal128, Regex, Timestamp, doc};
      use std::cmp::Ordering::{Equal, Greater, Less};
      use std::str::FromStr;

      fn strictly_increasing(values: &[Bson]) {
          for pair in values.windows(2) {
              assert_eq!(compare_bson(&pair[0], &pair[1]), Less, "{:?} < {:?}", pair[0], pair[1]);
              assert_eq!(compare_bson(&pair[1], &pair[0]), Greater);
          }
      }

      #[test]
      fn types_follow_mongodbs_canonical_order() {
          strictly_increasing(&[
              Bson::MinKey,
              Bson::Null,
              Bson::Int32(-5),
              Bson::Double(-0.5),
              Bson::Int64(0),
              Bson::Decimal128(Decimal128::from_str("0.25").unwrap()),
              Bson::Int32(1),
              Bson::String("a".into()),
              Bson::String("b".into()),
              Bson::Document(doc! { "a": 1 }),
              Bson::Array(vec![]),
              Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![1] }),
              Bson::ObjectId(ObjectId::from_bytes([0; 12])),
              Bson::ObjectId(ObjectId::from_bytes([1; 12])),
              Bson::Boolean(false),
              Bson::Boolean(true),
              Bson::DateTime(DateTime::from_millis(0)),
              Bson::DateTime(DateTime::from_millis(1)),
              Bson::Timestamp(Timestamp { time: 0, increment: 1 }),
              Bson::Timestamp(Timestamp { time: 1, increment: 0 }),
              Bson::RegularExpression(Regex { pattern: "a".into(), options: "".into() }),
              Bson::MaxKey,
          ]);
      }

      #[test]
      fn null_and_undefined_are_equal_and_symbol_equals_string() {
          assert_eq!(compare_bson(&Bson::Null, &Bson::Undefined), Equal);
          assert_eq!(compare_bson(&Bson::Symbol("a".into()), &Bson::String("a".into())), Equal);
          assert_eq!(compare_bson(&Bson::MinKey, &Bson::MinKey), Equal);
          assert_eq!(compare_bson(&Bson::MaxKey, &Bson::MaxKey), Equal);
      }

      #[test]
      fn strings_compare_by_utf8_bytes() {
          assert_eq!(compare_bson(&Bson::String("B".into()), &Bson::String("a".into())), Less);
          assert_eq!(compare_bson(&Bson::String("abc".into()), &Bson::String("abd".into())), Less);
          assert_eq!(compare_bson(&Bson::String("z".into()), &Bson::String("é".into())), Less);
          assert_eq!(compare_bson(&Bson::String("a".into()), &Bson::String("ab".into())), Less);
          assert_eq!(compare_bson(&Bson::String("".into()), &Bson::String("a".into())), Less);
      }

      #[test]
      fn embedded_documents_compare_type_then_name_then_value() {
          // The name decides before the value does: {a: 2} sorts before {b: 1}.
          let a2 = Bson::Document(doc! { "a": 2 });
          let b1 = Bson::Document(doc! { "b": 1 });
          assert_eq!(compare_bson(&a2, &b1), Less);
          assert_eq!(compare_bson(&Bson::Document(doc! { "a": 1 }), &a2), Less);
          // A field of a lower type class sorts first, whatever the names are.
          let number = Bson::Document(doc! { "z": 1 });
          let string = Bson::Document(doc! { "a": "x" });
          assert_eq!(compare_bson(&number, &string), Less);
          // A document that is a prefix of another is smaller.
          let short = Bson::Document(doc! { "a": 1 });
          let long = Bson::Document(doc! { "a": 1, "b": 1 });
          assert_eq!(compare_bson(&short, &long), Less);
          assert_eq!(compare_bson(&Bson::Document(doc! {}), &short), Less);
      }

      #[test]
      fn arrays_compare_elementwise_and_a_prefix_is_smaller() {
          let one = Bson::Array(vec![Bson::Int32(1)]);
          let one_zero = Bson::Array(vec![Bson::Int32(1), Bson::Int32(0)]);
          let two = Bson::Array(vec![Bson::Int32(2)]);
          assert_eq!(compare_bson(&one, &one_zero), Less);
          assert_eq!(compare_bson(&one_zero, &two), Less);
          assert_eq!(compare_bson(&two, &Bson::Array(vec![Bson::Int32(1), Bson::Int32(9), Bson::Int32(9)])), Greater);
      }

      #[test]
      fn binary_compares_length_then_subtype_then_bytes() {
          let bin = |subtype, bytes: &[u8]| Bson::Binary(Binary { subtype, bytes: bytes.to_vec() });
          assert_eq!(compare_bson(&bin(BinarySubtype::Generic, &[0xff]), &bin(BinarySubtype::Generic, &[0, 0])), Less);
          assert_eq!(compare_bson(&bin(BinarySubtype::Generic, &[9]), &bin(BinarySubtype::Uuid, &[0])), Less);
          assert_eq!(compare_bson(&bin(BinarySubtype::Uuid, &[1]), &bin(BinarySubtype::Uuid, &[2])), Less);
      }

      #[test]
      fn tuples_compare_field_by_field() {
          let t = |a: i32, b: &str| KeyTuple(vec![Bson::Int32(a), Bson::String(b.into())]);
          assert_eq!(compare_tuples(&t(1, "b"), &t(1, "c")), Less);
          assert_eq!(compare_tuples(&t(1, "z"), &t(2, "a")), Less);
          assert_eq!(compare_tuples(&t(2, "a"), &t(2, "a")), Equal);
          assert_eq!(
              compare_tuples(
                  &KeyTuple(vec![Bson::Int32(1)]),
                  &KeyTuple(vec![Bson::Int64(1)])
              ),
              Equal
          );
      }

      #[test]
      fn key_spec_validation() {
          assert_eq!(KeySpec::id().fields, vec!["_id".to_string()]);
          assert!(KeySpec::id().contains_id());
          assert!(KeySpec::new(vec!["email".into(), "tenant.id".into()]).is_ok());
          assert!(!KeySpec::new(vec!["email".into()]).unwrap().contains_id());
          for bad in [vec![], vec!["".to_string()], vec!["a..b".to_string()], vec!["a.".to_string()], vec!["a".to_string(), "a".to_string()]] {
              assert!(KeySpec::new(bad.clone()).is_err(), "{bad:?}");
          }
      }

      #[test]
      fn extract_key_reads_plain_compound_and_dotted_fields() {
          let d = doc! { "_id": 7, "email": "a@x", "tenant": { "id": 3 } };
          assert_eq!(extract_key(&d, &KeySpec::id()).unwrap(), KeyTuple(vec![Bson::Int32(7)]));
          let spec = KeySpec::new(vec!["tenant.id".into(), "email".into()]).unwrap();
          assert_eq!(
              extract_key(&d, &spec).unwrap(),
              KeyTuple(vec![Bson::Int32(3), Bson::String("a@x".into())])
          );
      }

      #[test]
      fn extract_key_refuses_missing_and_unusable_values() {
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          assert_eq!(extract_key(&doc! { "x": 1 }, &spec), Err(CoreError::CompareKeyMissing));
          // A dotted path cannot reach through an array.
          let through = KeySpec::new(vec!["a.b".into()]).unwrap();
          assert_eq!(
              extract_key(&doc! { "a": [{ "b": 1 }] }, &through),
              Err(CoreError::CompareKeyMissing)
          );
          for bad in [
              Bson::Array(vec![Bson::Int32(1)]),
              Bson::Undefined,
              Bson::Symbol("s".into()),
              Bson::JavaScriptCode("x".into()),
          ] {
              assert_eq!(
                  extract_key(&doc! { "k": bad.clone() }, &spec),
                  Err(CoreError::CompareKeyInvalid),
                  "{bad:?}"
              );
          }
          // A single explicit null is a usable key; two of them are caught later as duplicates.
          assert_eq!(extract_key(&doc! { "k": Bson::Null }, &spec).unwrap(), KeyTuple(vec![Bson::Null]));
      }
  }
  ```

- [ ] **Step 10: Run and watch it fail.**
  Run: `cargo test -p curlybrace-core compare::key`
  Expected: FAIL to compile (`compare_bson`, `KeySpec`, ... not found).

- [ ] **Step 11: Implement `key.rs`.** Put this ABOVE the test module:

  ```rust
  //! Keys for the compare feature: which fields identify a document, and the client-side
  //! comparator that reproduces MongoDB's sort order for them.
  //!
  //! The merge-join in `engine` is only correct if this order matches the server's sort under
  //! simple (binary) collation exactly. The order is MongoDB's documented BSON comparison order:
  //! MinKey < Null < numbers < String/Symbol < Document < Array < Binary < ObjectId < Boolean <
  //! DateTime < Timestamp < Regex < (deprecated types) < MaxKey. Numbers compare by exact value
  //! across types (see `number`); strings compare by UTF-8 bytes; embedded documents compare field
  //! by field as (type class, field name, value).

  use std::cmp::Ordering;

  use bson::{Bson, Document};
  use serde::{Deserialize, Serialize};

  use super::number::compare_numbers;
  use crate::error::{CoreError, Result};

  /// The fields that identify a document: dotted paths, in sort order.
  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub struct KeySpec {
      pub fields: Vec<String>,
  }

  impl KeySpec {
      /// The default key: `_id`.
      pub fn id() -> Self {
          Self { fields: vec!["_id".into()] }
      }

      /// A key over `fields`: non-empty, no empty path segments, no field listed twice.
      pub fn new(fields: Vec<String>) -> Result<Self> {
          if fields.is_empty() {
              return Err(CoreError::InvalidDocument("the key needs at least one field".into()));
          }
          for (i, field) in fields.iter().enumerate() {
              if field.is_empty() || field.split('.').any(str::is_empty) {
                  return Err(CoreError::InvalidDocument(
                      "a key field name must not be empty".into(),
                  ));
              }
              if fields[..i].contains(field) {
                  return Err(CoreError::InvalidDocument("a key field is listed twice".into()));
              }
          }
          Ok(Self { fields })
      }

      pub fn contains_id(&self) -> bool {
          self.fields.iter().any(|f| f == "_id")
      }
  }

  /// One value per key field, in key order.
  #[derive(Clone, Debug, PartialEq)]
  pub struct KeyTuple(pub Vec<Bson>);

  /// Rank of a value's canonical type; values of different ranks never compare equal.
  fn type_rank(b: &Bson) -> u8 {
      match b {
          Bson::MinKey => 0,
          Bson::Null | Bson::Undefined => 1,
          Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => 2,
          Bson::String(_) | Bson::Symbol(_) => 3,
          Bson::Document(_) => 4,
          Bson::Array(_) => 5,
          Bson::Binary(_) => 6,
          Bson::ObjectId(_) => 7,
          Bson::Boolean(_) => 8,
          Bson::DateTime(_) => 9,
          Bson::Timestamp(_) => 10,
          Bson::RegularExpression(_) => 11,
          Bson::DbPointer(_) => 12,
          Bson::JavaScriptCode(_) => 13,
          Bson::JavaScriptCodeWithScope(_) => 14,
          Bson::MaxKey => 15,
      }
  }

  fn text(b: &Bson) -> &str {
      match b {
          Bson::String(s) | Bson::Symbol(s) => s,
          _ => "",
      }
  }

  fn compare_docs(a: &Document, b: &Document) -> Ordering {
      let mut left = a.iter();
      let mut right = b.iter();
      loop {
          match (left.next(), right.next()) {
              (None, None) => return Ordering::Equal,
              (None, Some(_)) => return Ordering::Less,
              (Some(_), None) => return Ordering::Greater,
              (Some((ka, va)), Some((kb, vb))) => {
                  let ord = type_rank(va)
                      .cmp(&type_rank(vb))
                      .then_with(|| ka.as_bytes().cmp(kb.as_bytes()))
                      .then_with(|| compare_bson(va, vb));
                  if ord != Ordering::Equal {
                      return ord;
                  }
              }
          }
      }
  }

  fn compare_arrays(a: &[Bson], b: &[Bson]) -> Ordering {
      a.iter()
          .zip(b)
          .map(|(x, y)| type_rank(x).cmp(&type_rank(y)).then_with(|| compare_bson(x, y)))
          .find(|o| *o != Ordering::Equal)
          .unwrap_or_else(|| a.len().cmp(&b.len()))
  }

  /// Total order over `Bson`, matching MongoDB's sort order under simple collation.
  pub fn compare_bson(a: &Bson, b: &Bson) -> Ordering {
      let (ra, rb) = (type_rank(a), type_rank(b));
      if ra != rb {
          return ra.cmp(&rb);
      }
      match (a, b) {
          (Bson::Document(x), Bson::Document(y)) => compare_docs(x, y),
          (Bson::Array(x), Bson::Array(y)) => compare_arrays(x, y),
          (Bson::Binary(x), Bson::Binary(y)) => (x.bytes.len(), u8::from(x.subtype), &x.bytes)
              .cmp(&(y.bytes.len(), u8::from(y.subtype), &y.bytes)),
          (Bson::ObjectId(x), Bson::ObjectId(y)) => x.bytes().cmp(&y.bytes()),
          (Bson::Boolean(x), Bson::Boolean(y)) => x.cmp(y),
          (Bson::DateTime(x), Bson::DateTime(y)) => x.timestamp_millis().cmp(&y.timestamp_millis()),
          (Bson::Timestamp(x), Bson::Timestamp(y)) => {
              (x.time, x.increment).cmp(&(y.time, y.increment))
          }
          (Bson::RegularExpression(x), Bson::RegularExpression(y)) => {
              (&x.pattern, &x.options).cmp(&(&y.pattern, &y.options))
          }
          _ => match ra {
              2 => compare_numbers(a, b),
              3 => text(a).as_bytes().cmp(text(b).as_bytes()),
              // Null/Undefined and MinKey/MaxKey are equal within their class. The deprecated
              // classes never occur as keys (extract_key refuses them); this fallback only
              // keeps the order total for a nested value.
              12..=14 => format!("{a:?}").cmp(&format!("{b:?}")),
              _ => Ordering::Equal,
          },
      }
  }

  /// Lexicographic comparison of two key tuples.
  pub fn compare_tuples(a: &KeyTuple, b: &KeyTuple) -> Ordering {
      a.0.iter()
          .zip(&b.0)
          .map(|(x, y)| compare_bson(x, y))
          .find(|o| *o != Ordering::Equal)
          .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
  }

  /// A dotted-path lookup that only descends through documents (never through arrays).
  fn lookup<'a>(doc: &'a Document, path: &str) -> Option<&'a Bson> {
      let mut parts = path.split('.');
      let mut current = doc.get(parts.next()?)?;
      for part in parts {
          match current {
              Bson::Document(inner) => current = inner.get(part)?,
              _ => return None,
          }
      }
      Some(current)
  }

  /// A key value must sort strictly: arrays (the server sorts them by their smallest element) and
  /// deprecated types are refused.
  fn check_key_value(value: &Bson) -> Result<()> {
      match value {
          Bson::Array(_)
          | Bson::Undefined
          | Bson::Symbol(_)
          | Bson::DbPointer(_)
          | Bson::JavaScriptCode(_)
          | Bson::JavaScriptCodeWithScope(_) => Err(CoreError::CompareKeyInvalid),
          _ => Ok(()),
      }
  }

  /// The key tuple of `doc`: `CompareKeyMissing` when a key field is absent (or unreachable
  /// because the path crosses an array), `CompareKeyInvalid` for an unusable value.
  pub fn extract_key(doc: &Document, spec: &KeySpec) -> Result<KeyTuple> {
      let mut values = Vec::with_capacity(spec.fields.len());
      for path in &spec.fields {
          let value = lookup(doc, path).ok_or(CoreError::CompareKeyMissing)?;
          check_key_value(value)?;
          values.push(value.clone());
      }
      Ok(KeyTuple(values))
  }
  ```

  Then wire the module. `crates/core/src/compare/mod.rs`:

  ```rust
  //! Data compare: key ordering, normalization, fingerprints, diff, scanning and the merge-join
  //! engine. Pure core logic with no UI dependency; the runtime drives it (see `runtime::compare`).

  pub mod key;
  pub(crate) mod number;
  ```

  and add `pub mod compare;` to `crates/core/src/lib.rs` (alphabetically, after `pub mod client;`). Replace the whole body of `crates/core/src/test_support/bson_order.rs` with:

  ```rust
  //! The in-memory store sorts and matches with the same comparator the compare feature uses, so
  //! the fake and the real merge-join agree on key order.
  pub use crate::compare::key::compare_bson as cmp_bson;
  ```

- [ ] **Step 12: Run the key tests and the whole core suite.**
  Run: `cargo test -p curlybrace-core --features test-support`
  Expected: PASS. If an existing in-memory-store test fails because the old comparator ordered sub-documents by value, fix the test's expectation only if it asserted the old (wrong) order, and say so in the report.

- [ ] **Step 13: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add the key comparator with exact numeric ordering and compare errors

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 2: Rules, normalization and fingerprints

**Files:**
- Create: `crates/core/src/compare/rules.rs`, `crates/core/src/compare/normalize.rs`, `crates/core/src/compare/fingerprint.rs`
- Modify: `crates/core/src/compare/mod.rs`, `crates/core/Cargo.toml`

**Interfaces:**
- Consumes: `KeySpec`, `CoreError`, `Result`, `Dec` (crate-private).
- Produces:
  ```rust
  pub enum FieldScope { All, AllExcept(Vec<String>), Only(Vec<String>) }          // Clone, Debug, PartialEq, Eq, Serialize, Deserialize; Default = All
  pub struct CompareRules { pub field_scope: FieldScope, pub ignore_array_order: bool,
      pub numbers_equal_across_types: bool, pub ignore_field_order: bool, pub include_id: bool }   // same derives
  impl CompareRules { pub fn for_key(key: &KeySpec) -> Self; pub fn validate(&self) -> Result<()> }
  pub fn normalize(doc: &Document, rules: &CompareRules) -> Result<Document>;
  pub struct Fingerprint(pub [u8; 16]);                                            // Clone, Copy, PartialEq, Eq, Hash, Debug (hex)
  pub fn fingerprint(normalized: &Document) -> Result<Fingerprint>;
  pub fn fingerprint_of(doc: &Document, rules: &CompareRules) -> Result<Fingerprint>;
  ```

- [ ] **Step 1: Add the dependency.** In `crates/core/Cargo.toml` `[dependencies]`, add `sha2 = "0.10.9"` (alphabetical, after `serde_json`). Run `cargo update -w` is NOT needed; `cargo check -p curlybrace-core` will add it to `Cargo.lock`.

- [ ] **Step 2: Write the failing rules tests.** Create `crates/core/src/compare/rules.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn defaults_follow_the_spec() {
          let r = CompareRules::for_key(&KeySpec::id());
          assert_eq!(r.field_scope, FieldScope::All);
          assert!(!r.ignore_array_order);
          assert!(!r.numbers_equal_across_types);
          assert!(r.ignore_field_order, "field order is ignored by default");
          assert!(r.include_id, "_id is part of the key, so it is compared");
          let business = CompareRules::for_key(&KeySpec::new(vec!["email".into()]).unwrap());
          assert!(!business.include_id, "_id differs across environments by construction");
      }

      #[test]
      fn validate_rejects_empty_paths_only() {
          let mut r = CompareRules::for_key(&KeySpec::id());
          assert!(r.validate().is_ok());
          r.field_scope = FieldScope::AllExcept(vec!["updatedAt".into(), "meta.synced".into()]);
          assert!(r.validate().is_ok());
          for bad in ["", "a..b", ".a", "a."] {
              r.field_scope = FieldScope::Only(vec![bad.into()]);
              assert!(r.validate().is_err(), "{bad:?}");
          }
          r.field_scope = FieldScope::Only(vec![]);
          assert!(r.validate().is_err(), "an empty 'only' list would compare nothing");
      }

      #[test]
      fn rules_round_trip_through_json_without_secrets() {
          let mut r = CompareRules::for_key(&KeySpec::id());
          r.field_scope = FieldScope::Only(vec!["a.b".into()]);
          let json = serde_json::to_string(&r).unwrap();
          assert_eq!(serde_json::from_str::<CompareRules>(&json).unwrap(), r);
      }
  }
  ```

- [ ] **Step 3: Write the failing normalize tests.** Create `crates/core/src/compare/normalize.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::compare::key::KeySpec;
      use bson::{Bson, Decimal128, doc};
      use std::str::FromStr;

      fn rules() -> CompareRules {
          let mut r = CompareRules::for_key(&KeySpec::new(vec!["k".into()]).unwrap());
          r.include_id = true;
          r.ignore_field_order = false;
          r
      }

      fn n(doc: Document, r: &CompareRules) -> Document {
          normalize(&doc, r).expect("normalizes")
      }

      #[test]
      fn the_input_is_never_modified() {
          let original = doc! { "b": 1, "a": [3, 1, 2], "_id": 1 };
          let copy = original.clone();
          let mut r = rules();
          r.ignore_field_order = true;
          r.ignore_array_order = true;
          r.include_id = false;
          let _ = n(original.clone(), &r);
          assert_eq!(original, copy);
      }

      #[test]
      fn all_except_removes_paths_including_through_arrays() {
          let mut r = rules();
          r.field_scope = FieldScope::AllExcept(vec!["updatedAt".into(), "items.price".into(), "meta.sync".into()]);
          let d = doc! {
              "k": 1, "updatedAt": 5,
              "items": [{ "price": 1, "qty": 2 }, { "price": 3, "qty": 4 }, 7],
              "meta": { "sync": true, "owner": "x" },
          };
          assert_eq!(
              n(d, &r),
              doc! { "k": 1, "items": [{ "qty": 2 }, { "qty": 4 }, 7], "meta": { "owner": "x" } }
          );
      }

      #[test]
      fn a_terminal_path_wins_over_a_longer_one() {
          let mut r = rules();
          r.field_scope = FieldScope::AllExcept(vec!["a".into(), "a.b".into()]);
          assert_eq!(n(doc! { "k": 1, "a": { "b": 1, "c": 2 } }, &r), doc! { "k": 1 });
      }

      #[test]
      fn only_keeps_just_the_listed_paths() {
          let mut r = rules();
          r.field_scope = FieldScope::Only(vec!["name".into(), "meta.owner".into(), "items.price".into()]);
          let d = doc! {
              "k": 1, "name": "n", "other": 1,
              "meta": { "owner": "x", "sync": true },
              "gone": { "z": 1 },
              "items": [{ "price": 1, "qty": 2 }, { "qty": 3 }, 7],
          };
          assert_eq!(
              n(d, &r),
              doc! { "name": "n", "meta": { "owner": "x" }, "items": [{ "price": 1 }, {}] }
          );
      }

      #[test]
      fn id_is_removed_unless_it_is_included() {
          let mut r = rules();
          r.include_id = false;
          assert_eq!(n(doc! { "_id": 9, "k": 1 }, &r), doc! { "k": 1 });
          r.include_id = true;
          assert_eq!(n(doc! { "_id": 9, "k": 1 }, &r), doc! { "_id": 9, "k": 1 });
      }

      #[test]
      fn numbers_collapse_only_when_asked() {
          let mut r = rules();
          let d = doc! { "k": 1, "a": 1_i32, "b": 1_i64, "c": 1.0_f64, "d": 1.5_f64 };
          let strict = n(d.clone(), &r);
          assert_eq!(strict.get("a"), Some(&Bson::Int32(1)));
          r.numbers_equal_across_types = true;
          let loose = n(d, &r);
          for field in ["a", "b", "c"] {
              assert_eq!(loose.get(field), Some(&Bson::Int64(1)), "{field}");
          }
          assert_eq!(loose.get("d"), Some(&Bson::Double(1.5)));
      }

      #[test]
      fn number_collapsing_is_exact() {
          let mut r = rules();
          r.numbers_equal_across_types = true;
          let dec = |s: &str| Bson::Decimal128(Decimal128::from_str(s).unwrap());
          let out = n(
              doc! { "k": 1, "i": dec("12.0"), "f": dec("1.50"), "g": dec("1.5"), "z": -0.0_f64, "big": 1e30_f64, "nan": f64::NAN },
              &r,
          );
          assert_eq!(out.get("i"), Some(&Bson::Int64(12)));
          // Equal decimals written differently become identical values.
          assert_eq!(out.get("f"), out.get("g"));
          assert_eq!(out.get("z"), Some(&Bson::Int64(0)));
          // A double is never equal to a decimal unless the decimal is exactly its value.
          assert_eq!(out.get("big"), Some(&Bson::Double(1e30)));
          match out.get("nan") {
              Some(Bson::Double(x)) => assert!(x.is_nan()),
              other => panic!("{other:?}"),
          }
          let tenth = n(doc! { "k": 1, "d": dec("0.1"), "x": 0.1_f64 }, &r);
          assert_ne!(tenth.get("d"), tenth.get("x"));
      }

      #[test]
      fn field_order_is_sorted_recursively_only_when_ignored() {
          let mut r = rules();
          let d = doc! { "k": 1, "b": { "y": 1, "x": 2 }, "a": 0 };
          assert_eq!(n(d.clone(), &r).keys().collect::<Vec<_>>(), ["k", "b", "a"]);
          r.ignore_field_order = true;
          let sorted = n(d, &r);
          assert_eq!(sorted.keys().collect::<Vec<_>>(), ["a", "b", "k"]);
          assert_eq!(
              sorted.get_document("b").unwrap().keys().collect::<Vec<_>>(),
              ["x", "y"]
          );
      }

      #[test]
      fn array_order_is_ignored_for_scalars_and_documents() {
          let mut r = rules();
          r.ignore_field_order = true;
          let a = doc! { "k": 1, "xs": [3, 1, 2], "ds": [{ "a": 2 }, { "a": 1 }] };
          let b = doc! { "k": 1, "xs": [2, 3, 1], "ds": [{ "a": 1 }, { "a": 2 }] };
          assert_ne!(n(a.clone(), &r), n(b.clone(), &r), "order matters by default");
          r.ignore_array_order = true;
          assert_eq!(n(a, &r), n(b, &r));
      }

      #[test]
      fn array_order_sorting_applies_inside_nested_arrays() {
          let mut r = rules();
          r.ignore_array_order = true;
          let a = doc! { "k": 1, "m": [[2, 1], [4, 3]] };
          let b = doc! { "k": 1, "m": [[3, 4], [1, 2]] };
          assert_eq!(n(a, &r), n(b, &r));
      }
  }
  ```

- [ ] **Step 4: Write the failing fingerprint tests.** Create `crates/core/src/compare/fingerprint.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::compare::key::KeySpec;
      use bson::doc;

      fn rules() -> CompareRules {
          CompareRules::for_key(&KeySpec::id())
      }

      #[test]
      fn equal_documents_share_a_fingerprint_and_a_change_alters_it() {
          let a = doc! { "_id": 1, "a": 1, "b": "x" };
          assert_eq!(fingerprint_of(&a, &rules()).unwrap(), fingerprint_of(&a.clone(), &rules()).unwrap());
          let changed = doc! { "_id": 1, "a": 2, "b": "x" };
          assert_ne!(fingerprint_of(&a, &rules()).unwrap(), fingerprint_of(&changed, &rules()).unwrap());
      }

      #[test]
      fn types_are_distinguished_under_strict_rules() {
          let int = doc! { "_id": 1, "n": 1_i32 };
          let long = doc! { "_id": 1, "n": 1_i64 };
          assert_ne!(fingerprint_of(&int, &rules()).unwrap(), fingerprint_of(&long, &rules()).unwrap());
          let mut loose = rules();
          loose.numbers_equal_across_types = true;
          assert_eq!(fingerprint_of(&int, &loose).unwrap(), fingerprint_of(&long, &loose).unwrap());
      }

      #[test]
      fn fields_outside_the_scope_never_affect_the_fingerprint() {
          let mut r = rules();
          r.field_scope = FieldScope::AllExcept(vec!["updatedAt".into()]);
          let a = doc! { "_id": 1, "v": 1, "updatedAt": 1 };
          let b = doc! { "_id": 1, "v": 1, "updatedAt": 999 };
          assert_eq!(fingerprint_of(&a, &r).unwrap(), fingerprint_of(&b, &r).unwrap());
      }

      #[test]
      fn the_fingerprint_is_sixteen_bytes_and_prints_as_hex() {
          let f = fingerprint_of(&doc! { "_id": 1 }, &rules()).unwrap();
          assert_eq!(f.0.len(), 16);
          assert_eq!(format!("{f:?}").len(), "Fingerprint()".len() + 32);
      }

      #[test]
      fn a_known_vector_pins_the_hash_so_it_cannot_change_silently() {
          // SHA-256 of the BSON bytes of {"_id": 1}, first 16 bytes. If this changes, every saved
          // comparison's fingerprints change meaning: bump deliberately, never by accident.
          let f = fingerprint(&doc! { "_id": 1_i32 }).unwrap();
          assert_eq!(hex(&f.0), EXPECTED_ID_1);
      }
  }
  ```
  (Leave `EXPECTED_ID_1` and `hex` to Step 8, where the real value is computed once and pinned.)

- [ ] **Step 5: Run and watch them fail.**
  Add to `crates/core/src/compare/mod.rs`: `pub mod fingerprint; pub mod normalize; pub mod rules;`.
  Run: `cargo test -p curlybrace-core compare::`
  Expected: FAIL to compile (types and functions not found).

- [ ] **Step 6: Implement `rules.rs`.** Above the test module:

  ```rust
  //! The rules a compare applies before two documents are called equal.

  use serde::{Deserialize, Serialize};

  use super::key::KeySpec;
  use crate::error::{CoreError, Result};

  /// Which fields take part in the comparison. Paths are dotted; a path that crosses an array
  /// addresses every element of it.
  #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
  pub enum FieldScope {
      /// Compare every field.
      #[default]
      All,
      /// Ignore these paths.
      AllExcept(Vec<String>),
      /// Compare only these paths.
      Only(Vec<String>),
  }

  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub struct CompareRules {
      pub field_scope: FieldScope,
      pub ignore_array_order: bool,
      pub numbers_equal_across_types: bool,
      pub ignore_field_order: bool,
      /// Compare `_id` too. When the key is a business key, `_id` differs across environments by
      /// construction, so it is left out unless this is switched on.
      pub include_id: bool,
  }

  impl CompareRules {
      /// The defaults for a compare keyed on `key`: strict BSON equality, except that the order
      /// of fields inside documents is ignored.
      pub fn for_key(key: &KeySpec) -> Self {
          Self {
              field_scope: FieldScope::All,
              ignore_array_order: false,
              numbers_equal_across_types: false,
              ignore_field_order: true,
              include_id: key.contains_id(),
          }
      }

      /// Checks the user-entered paths.
      pub fn validate(&self) -> Result<()> {
          let paths = match &self.field_scope {
              FieldScope::All => return Ok(()),
              FieldScope::AllExcept(paths) => paths,
              FieldScope::Only(paths) if paths.is_empty() => {
                  return Err(CoreError::InvalidDocument(
                      "choose at least one field to compare".into(),
                  ));
              }
              FieldScope::Only(paths) => paths,
          };
          for path in paths {
              if path.is_empty() || path.split('.').any(str::is_empty) {
                  return Err(CoreError::InvalidDocument("a field path must not be empty".into()));
              }
          }
          Ok(())
      }
  }
  ```

- [ ] **Step 7: Implement `normalize.rs`.** Above the test module:

  ```rust
  //! Turns a document into a canonical form under a set of [`CompareRules`], so that documents
  //! that are equal under the rules are byte-for-byte equal afterwards. The input is never
  //! modified. Order of operations: field scope, `_id`, numbers, then field and array order.

  use std::collections::BTreeMap;

  use bson::{Bson, Document};

  use super::number::Dec;
  use super::rules::{CompareRules, FieldScope};
  use crate::error::{CoreError, Result};

  #[derive(Default)]
  struct PathTree {
      terminal: bool,
      children: BTreeMap<String, PathTree>,
  }

  impl PathTree {
      fn from_paths(paths: &[String]) -> Self {
          let mut root = Self::default();
          for path in paths {
              let mut node = &mut root;
              for part in path.split('.') {
                  node = node.children.entry(part.to_string()).or_default();
              }
              node.terminal = true;
          }
          root
      }
  }

  fn drop_paths(doc: &Document, tree: &PathTree) -> Document {
      let mut out = Document::new();
      for (key, value) in doc {
          match tree.children.get(key) {
              None => {
                  out.insert(key.clone(), value.clone());
              }
              Some(child) if child.terminal => {}
              Some(child) => {
                  out.insert(key.clone(), drop_in_value(value, child));
              }
          }
      }
      out
  }

  fn drop_in_value(value: &Bson, child: &PathTree) -> Bson {
      match value {
          Bson::Document(d) => Bson::Document(drop_paths(d, child)),
          Bson::Array(items) => Bson::Array(items.iter().map(|i| drop_in_value(i, child)).collect()),
          other => other.clone(),
      }
  }

  fn keep_paths(doc: &Document, tree: &PathTree) -> Document {
      let mut out = Document::new();
      for (key, value) in doc {
          let Some(child) = tree.children.get(key) else {
              continue;
          };
          if child.terminal {
              out.insert(key.clone(), value.clone());
          } else if let Some(kept) = keep_in_value(value, child, false) {
              out.insert(key.clone(), kept);
          }
      }
      out
  }

  /// `in_array` keeps documents (possibly empty) so array positions stay aligned; a nested
  /// document that matches nothing is dropped.
  fn keep_in_value(value: &Bson, child: &PathTree, in_array: bool) -> Option<Bson> {
      match value {
          Bson::Document(d) => {
              let kept = keep_paths(d, child);
              (in_array || !kept.is_empty()).then_some(Bson::Document(kept))
          }
          Bson::Array(items) => {
              let kept: Vec<Bson> = items
                  .iter()
                  .filter_map(|i| keep_in_value(i, child, true))
                  .collect();
              (!kept.is_empty()).then_some(Bson::Array(kept))
          }
          _ => None,
      }
  }

  /// The canonical value of a number when "numbers equal across types" is on.
  fn canonical_number(value: &Bson) -> Bson {
      match value {
          Bson::Int32(n) => Bson::Int64(i64::from(*n)),
          Bson::Double(d) => {
              if d.is_nan() {
                  Bson::Double(f64::NAN)
              } else if d.is_finite()
                  && d.fract() == 0.0
                  && *d >= -9_223_372_036_854_775_808.0
                  && *d < 9_223_372_036_854_775_808.0
              {
                  Bson::Int64(*d as i64)
              } else {
                  Bson::Double(*d)
              }
          }
          Bson::Decimal128(d) => {
              let dec = Dec::of(d);
              if let Some(n) = dec.to_i64() {
                  Bson::Int64(n)
              } else if let Some(canonical) = dec.to_decimal128() {
                  Bson::Decimal128(canonical)
              } else {
                  // NaN and the infinities have one representation each.
                  value.clone()
              }
          }
          other => other.clone(),
      }
  }

  fn element_bytes(value: &Bson) -> Result<Vec<u8>> {
      let wrapped = Document::from_iter([("v".to_string(), value.clone())]);
      bson::to_vec(&wrapped)
          .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))
  }

  fn canonicalize(value: &Bson, rules: &CompareRules) -> Result<Bson> {
      Ok(match value {
          Bson::Document(d) => Bson::Document(canonical_doc(d, rules)?),
          Bson::Array(items) => {
              let mut out = items
                  .iter()
                  .map(|i| canonicalize(i, rules))
                  .collect::<Result<Vec<_>>>()?;
              if rules.ignore_array_order {
                  let mut keyed = out
                      .into_iter()
                      .map(|v| element_bytes(&v).map(|bytes| (bytes, v)))
                      .collect::<Result<Vec<_>>>()?;
                  keyed.sort_by(|a, b| a.0.cmp(&b.0));
                  out = keyed.into_iter().map(|(_, v)| v).collect();
              }
              Bson::Array(out)
          }
          Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_)
              if rules.numbers_equal_across_types =>
          {
              canonical_number(value)
          }
          other => other.clone(),
      })
  }

  fn canonical_doc(doc: &Document, rules: &CompareRules) -> Result<Document> {
      let mut pairs: Vec<(&String, &Bson)> = doc.iter().collect();
      if rules.ignore_field_order {
          pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
      }
      pairs
          .into_iter()
          .map(|(k, v)| canonicalize(v, rules).map(|c| (k.clone(), c)))
          .collect()
  }

  /// The canonical form of `doc` under `rules`.
  pub fn normalize(doc: &Document, rules: &CompareRules) -> Result<Document> {
      let scoped = match &rules.field_scope {
          FieldScope::All => doc.clone(),
          FieldScope::AllExcept(paths) => drop_paths(doc, &PathTree::from_paths(paths)),
          FieldScope::Only(paths) => keep_paths(doc, &PathTree::from_paths(paths)),
      };
      let scoped = if rules.include_id {
          scoped
      } else {
          let mut without = scoped;
          without.remove("_id");
          without
      };
      canonical_doc(&scoped, rules)
  }
  ```

- [ ] **Step 8: Implement `fingerprint.rs`, then pin the vector.** Above the test module:

  ```rust
  //! A 128-bit fingerprint of a normalized document: the first 16 bytes of SHA-256 over its BSON
  //! bytes. Apply (slice 2) recomputes it to detect a document that changed since the compare.

  use std::fmt;

  use bson::Document;
  use sha2::{Digest, Sha256};

  use super::normalize::normalize;
  use super::rules::CompareRules;
  use crate::error::{CoreError, Result};

  #[derive(Clone, Copy, PartialEq, Eq, Hash)]
  pub struct Fingerprint(pub [u8; 16]);

  fn hex(bytes: &[u8]) -> String {
      bytes.iter().map(|b| format!("{b:02x}")).collect()
  }

  impl fmt::Debug for Fingerprint {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
          write!(f, "Fingerprint({})", hex(&self.0))
      }
  }

  /// Fingerprint of an already normalized document.
  pub fn fingerprint(normalized: &Document) -> Result<Fingerprint> {
      let bytes = bson::to_vec(normalized)
          .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))?;
      let digest = Sha256::digest(&bytes);
      let mut out = [0u8; 16];
      out.copy_from_slice(&digest[..16]);
      Ok(Fingerprint(out))
  }

  /// Normalizes under `rules`, then fingerprints.
  pub fn fingerprint_of(doc: &Document, rules: &CompareRules) -> Result<Fingerprint> {
      fingerprint(&normalize(doc, rules)?)
  }
  ```

  Run the tests once; the last test will not compile because of `EXPECTED_ID_1`/`hex` (the test module uses `hex` from `super::*`, which now exists, and needs the constant). Compute the value: temporarily add `println!("{}", hex(&fingerprint(&doc! { "_id": 1_i32 }).unwrap().0));` in a scratch test, run `cargo test -p curlybrace-core compare::fingerprint -- --nocapture`, copy the 32 hex characters, remove the print, and add above the tests module `#[cfg(test)] const EXPECTED_ID_1: &str = "<the 32 hex characters>";`. Do NOT invent the value.

- [ ] **Step 9: Run all compare tests.**
  Run: `cargo test -p curlybrace-core compare::`
  Expected: PASS.

- [ ] **Step 10: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates Cargo.lock && git commit -m "feat(compare): add compare rules, normalization and fingerprints

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 3: Field-level diff

**Files:**
- Create: `crates/core/src/compare/diff.rs`
- Modify: `crates/core/src/compare/mod.rs`

**Interfaces:**
- Consumes: `normalize` output (callers normalize both documents with the same rules first), `CoreError`.
- Produces:
  ```rust
  pub enum ChangeKind { Added, Removed, Changed }   // Added: only in target; Removed: only in source
  pub struct FieldChange { pub path: String, pub kind: ChangeKind, pub source: Option<Bson>, pub target: Option<Bson> }
  pub fn diff_normalized(source: &Document, target: &Document, ignore_array_order: bool) -> Result<Vec<FieldChange>>;
  ```
  `FieldChange` derives `Clone, Debug, PartialEq`; `ChangeKind` derives `Clone, Copy, Debug, PartialEq, Eq`.

- [ ] **Step 1: Write the failing tests.** Create `crates/core/src/compare/diff.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use bson::doc;

      fn d(a: Document, b: Document) -> Vec<FieldChange> {
          diff_normalized(&a, &b, false).unwrap()
      }

      fn change(path: &str, kind: ChangeKind, source: Option<Bson>, target: Option<Bson>) -> FieldChange {
          FieldChange { path: path.into(), kind, source, target }
      }

      #[test]
      fn identical_documents_have_no_changes() {
          assert!(d(doc! { "a": 1, "b": { "c": [1, 2] } }, doc! { "a": 1, "b": { "c": [1, 2] } }).is_empty());
      }

      #[test]
      fn a_changed_scalar_reports_its_path() {
          assert_eq!(
              d(doc! { "a": { "b": 1 } }, doc! { "a": { "b": 2 } }),
              vec![change("a.b", ChangeKind::Changed, Some(Bson::Int32(1)), Some(Bson::Int32(2)))]
          );
      }

      #[test]
      fn fields_present_on_one_side_are_removed_or_added() {
          assert_eq!(
              d(doc! { "x": 1, "keep": 0 }, doc! { "keep": 0, "y": 2 }),
              vec![
                  change("x", ChangeKind::Removed, Some(Bson::Int32(1)), None),
                  change("y", ChangeKind::Added, None, Some(Bson::Int32(2))),
              ]
          );
      }

      #[test]
      fn a_type_only_difference_is_a_change() {
          assert_eq!(
              d(doc! { "n": 1_i32 }, doc! { "n": 1_i64 }),
              vec![change("n", ChangeKind::Changed, Some(Bson::Int32(1)), Some(Bson::Int64(1)))]
          );
      }

      #[test]
      fn a_value_that_changes_kind_is_one_change_at_that_path() {
          assert_eq!(
              d(doc! { "a": { "b": 1 } }, doc! { "a": [1] }),
              vec![change(
                  "a",
                  ChangeKind::Changed,
                  Some(Bson::Document(doc! { "b": 1 })),
                  Some(Bson::Array(vec![Bson::Int32(1)]))
              )]
          );
      }

      #[test]
      fn ordered_arrays_diff_by_index_and_report_length_differences() {
          assert_eq!(
              d(doc! { "a": [1, 2, 3] }, doc! { "a": [1, 9] }),
              vec![
                  change("a[1]", ChangeKind::Changed, Some(Bson::Int32(2)), Some(Bson::Int32(9))),
                  change("a[2]", ChangeKind::Removed, Some(Bson::Int32(3)), None),
              ]
          );
          assert_eq!(
              d(doc! { "a": [1] }, doc! { "a": [1, 5] }),
              vec![change("a[1]", ChangeKind::Added, None, Some(Bson::Int32(5)))]
          );
      }

      #[test]
      fn documents_inside_arrays_diff_recursively() {
          assert_eq!(
              d(doc! { "a": [{ "p": 1 }] }, doc! { "a": [{ "p": 2 }] }),
              vec![change("a[0].p", ChangeKind::Changed, Some(Bson::Int32(1)), Some(Bson::Int32(2)))]
          );
      }

      #[test]
      fn unordered_arrays_report_elements_present_on_only_one_side() {
          // Inputs are normalized, so unordered arrays arrive sorted.
          let out = diff_normalized(&doc! { "a": [1, 2, 3] }, &doc! { "a": [2, 3, 4] }, true).unwrap();
          assert_eq!(
              out,
              vec![
                  change("a[*]", ChangeKind::Removed, Some(Bson::Int32(1)), None),
                  change("a[*]", ChangeKind::Added, None, Some(Bson::Int32(4))),
              ]
          );
      }

      #[test]
      fn unordered_arrays_respect_multiplicity() {
          let out = diff_normalized(&doc! { "a": [1, 1, 2] }, &doc! { "a": [1, 2] }, true).unwrap();
          assert_eq!(out, vec![change("a[*]", ChangeKind::Removed, Some(Bson::Int32(1)), None)]);
      }

      #[test]
      fn nan_equals_nan_like_the_fingerprint_does() {
          assert!(d(doc! { "x": f64::NAN }, doc! { "x": f64::NAN }).is_empty());
      }
  }
  ```

- [ ] **Step 2: Run and watch it fail.** Add `pub mod diff;` to `crates/core/src/compare/mod.rs`.
  Run: `cargo test -p curlybrace-core compare::diff`
  Expected: FAIL to compile.

- [ ] **Step 3: Implement `diff.rs`.** Above the test module:

  ```rust
  //! The field-level difference between two normalized documents, for the detail view and (later)
  //! field-level apply. Both inputs must already be normalized with the same rules. Equality is
  //! byte equality of the BSON, the same notion the fingerprint uses, so `NaN` equals `NaN` and
  //! `Int32 1` differs from `Int64 1`.

  use bson::{Bson, Document};

  use crate::error::{CoreError, Result};

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum ChangeKind {
      /// Present only in the target.
      Added,
      /// Present only in the source.
      Removed,
      /// Present in both, with different values.
      Changed,
  }

  #[derive(Clone, Debug, PartialEq)]
  pub struct FieldChange {
      /// `a.b[2].c`; `a[*]` for an element of an unordered array.
      pub path: String,
      pub kind: ChangeKind,
      pub source: Option<Bson>,
      pub target: Option<Bson>,
  }

  fn bytes(value: &Bson) -> Result<Vec<u8>> {
      let wrapped = Document::from_iter([("v".to_string(), value.clone())]);
      bson::to_vec(&wrapped)
          .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))
  }

  fn join(path: &str, key: &str) -> String {
      if path.is_empty() {
          key.to_string()
      } else {
          format!("{path}.{key}")
      }
  }

  struct Walker {
      ignore_array_order: bool,
      out: Vec<FieldChange>,
  }

  impl Walker {
      fn push(&mut self, path: String, kind: ChangeKind, source: Option<&Bson>, target: Option<&Bson>) {
          self.out.push(FieldChange {
              path,
              kind,
              source: source.cloned(),
              target: target.cloned(),
          });
      }

      fn docs(&mut self, source: &Document, target: &Document, path: &str) -> Result<()> {
          for (key, sv) in source {
              let p = join(path, key);
              match target.get(key) {
                  None => self.push(p, ChangeKind::Removed, Some(sv), None),
                  Some(tv) => self.values(sv, tv, &p)?,
              }
          }
          for (key, tv) in target {
              if !source.contains_key(key) {
                  self.push(join(path, key), ChangeKind::Added, None, Some(tv));
              }
          }
          Ok(())
      }

      fn values(&mut self, source: &Bson, target: &Bson, path: &str) -> Result<()> {
          match (source, target) {
              (Bson::Document(a), Bson::Document(b)) => self.docs(a, b, path),
              (Bson::Array(a), Bson::Array(b)) if self.ignore_array_order => self.unordered(a, b, path),
              (Bson::Array(a), Bson::Array(b)) => self.ordered(a, b, path),
              _ => {
                  if bytes(source)? != bytes(target)? {
                      self.push(path.to_string(), ChangeKind::Changed, Some(source), Some(target));
                  }
                  Ok(())
              }
          }
      }

      fn ordered(&mut self, source: &[Bson], target: &[Bson], path: &str) -> Result<()> {
          for i in 0..source.len().max(target.len()) {
              let p = format!("{path}[{i}]");
              match (source.get(i), target.get(i)) {
                  (Some(s), Some(t)) => self.values(s, t, &p)?,
                  (Some(s), None) => self.push(p, ChangeKind::Removed, Some(s), None),
                  (None, Some(t)) => self.push(p, ChangeKind::Added, None, Some(t)),
                  (None, None) => {}
              }
          }
          Ok(())
      }

      /// Both arrays are sorted by canonical bytes (normalization did that), so a merge walk
      /// pairs equal elements and reports the rest, honouring multiplicity.
      fn unordered(&mut self, source: &[Bson], target: &[Bson], path: &str) -> Result<()> {
          let keyed = |items: &[Bson]| -> Result<Vec<(Vec<u8>, usize)>> {
              items.iter().enumerate().map(|(i, v)| bytes(v).map(|b| (b, i))).collect()
          };
          let (s, t) = (keyed(source)?, keyed(target)?);
          let p = format!("{path}[*]");
          let (mut i, mut j) = (0, 0);
          while i < s.len() || j < t.len() {
              let order = match (s.get(i), t.get(j)) {
                  (Some(a), Some(b)) => a.0.cmp(&b.0),
                  (Some(_), None) => std::cmp::Ordering::Less,
                  _ => std::cmp::Ordering::Greater,
              };
              match order {
                  std::cmp::Ordering::Equal => {
                      i += 1;
                      j += 1;
                  }
                  std::cmp::Ordering::Less => {
                      self.push(p.clone(), ChangeKind::Removed, Some(&source[s[i].1]), None);
                      i += 1;
                  }
                  std::cmp::Ordering::Greater => {
                      self.push(p.clone(), ChangeKind::Added, None, Some(&target[t[j].1]));
                      j += 1;
                  }
              }
          }
          Ok(())
      }
  }

  /// The changes that turn `source` into `target`, in document order.
  pub fn diff_normalized(
      source: &Document,
      target: &Document,
      ignore_array_order: bool,
  ) -> Result<Vec<FieldChange>> {
      let mut walker = Walker { ignore_array_order, out: Vec::new() };
      walker.docs(source, target, "")?;
      Ok(walker.out)
  }
  ```

- [ ] **Step 4: Run the tests.**
  Run: `cargo test -p curlybrace-core compare::diff`
  Expected: PASS (10 tests).

- [ ] **Step 5: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add the field-level document diff

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 4: Store contract: keyset scan, counts, index status and collection kinds

**Files:**
- Create: `crates/core/src/compare/keyset.rs`, `crates/core/tests/local_compare_store.rs`
- Modify: `crates/core/src/compare/mod.rs`, `crates/core/src/store.rs`, `crates/core/src/mongo_store.rs`, `crates/core/src/test_support/memory_store.rs`, `crates/core/src/test_support/query_eval.rs` (only if a matcher helper must be exposed)

**Interfaces:**
- Consumes: `KeySpec`, `KeyTuple`, `compare_tuples`, `extract_key`, `CoreError`.
- Produces:
  ```rust
  // store.rs
  pub enum IndexStatus { Indexed, NotIndexed }                 // Clone, Copy, Debug, PartialEq, Eq
  pub enum CollectionKind { Collection, View, Timeseries }     // Clone, Copy, Debug, PartialEq, Eq
  // DocumentStore: four methods with default bodies returning CoreError::Unsupported, so every
  // existing test double keeps compiling:
  async fn estimated_count(&self, ns: &Namespace) -> Result<u64>;
  async fn scan_sorted(&self, ns: &Namespace, filter: &Document, key: &KeySpec,
                       from: Option<&KeyTuple>, limit: u32) -> Result<Vec<Document>>;   // key >= from, ascending, simple collation
  async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus>;
  async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>>;
  // compare/keyset.rs (pure)
  pub fn keyset_filter(user_filter: &Document, key: &KeySpec, from: Option<&KeyTuple>) -> Document;
  pub fn type_class_names(rank: u8) -> &'static [&'static str];
  pub struct IndexInfo { pub keys: Document, pub sparse: bool, pub has_partial_filter: bool, pub collation_is_simple: bool }
  pub fn index_serves_sort(index: &IndexInfo, collection_default_is_simple: bool, key: &KeySpec) -> bool;
  ```

- [ ] **Step 1: Write the failing keyset tests (pure).** Create `crates/core/src/compare/keyset.rs` with only the test module below (the implementation is Step 4):

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use bson::{Bson, doc};

      fn key(fields: &[&str]) -> KeySpec {
          KeySpec::new(fields.iter().map(|s| s.to_string()).collect()).unwrap()
      }

      #[test]
      fn with_no_start_the_filter_is_the_users_filter() {
          let f = keyset_filter(&doc! { "tenant": "a" }, &key(&["_id"]), None);
          assert_eq!(f, doc! { "tenant": "a" });
          assert_eq!(keyset_filter(&doc! {}, &key(&["_id"]), None), doc! {});
      }

      #[test]
      fn a_single_key_is_at_or_after_or_of_a_higher_type_class() {
          let from = KeyTuple(vec![Bson::Int32(5)]);
          let f = keyset_filter(&doc! {}, &key(&["_id"]), Some(&from));
          // _id >= 5 among numbers, OR the value is not of a type class ranked at or below numbers.
          assert_eq!(
              f,
              doc! { "$or": [
                  { "_id": { "$gte": 5 } },
                  { "_id": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal"] } } },
              ] }
          );
      }

      #[test]
      fn the_user_filter_is_combined_with_and() {
          let from = KeyTuple(vec![Bson::String("m".into())]);
          let f = keyset_filter(&doc! { "tenant": "a" }, &key(&["k"]), Some(&from));
          assert_eq!(f.get_array("$and").unwrap().len(), 2);
          assert_eq!(f.get_array("$and").unwrap()[0], Bson::Document(doc! { "tenant": "a" }));
      }

      #[test]
      fn a_compound_key_is_a_lexicographic_or_chain_with_an_inclusive_last_field() {
          let from = KeyTuple(vec![Bson::Int32(1), Bson::String("x".into())]);
          let f = keyset_filter(&doc! {}, &key(&["a", "b"]), Some(&from));
          let branches = f.get_array("$or").unwrap();
          assert_eq!(branches.len(), 2);
          // Branch 0: a is strictly greater (or a higher class).
          assert_eq!(
              branches[0],
              Bson::Document(doc! { "$or": [
                  { "a": { "$gt": 1 } },
                  { "a": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal"] } } },
              ] })
          );
          // Branch 1: a equals, and b is at or after.
          assert_eq!(
              branches[1],
              Bson::Document(doc! { "a": 1, "$or": [
                  { "b": { "$gte": "x" } },
                  { "b": { "$not": { "$type": ["minKey", "null", "undefined", "int", "long", "double", "decimal", "string", "symbol"] } } },
              ] })
          );
      }

      #[test]
      fn the_type_classes_cover_every_rank_in_order() {
          assert_eq!(type_class_names(0), ["minKey"]);
          assert_eq!(type_class_names(1), ["minKey", "null", "undefined"]);
          assert!(type_class_names(15).contains(&"maxKey"));
          for r in 1..=15 {
              assert!(type_class_names(r).len() >= type_class_names(r - 1).len(), "rank {r}");
          }
      }

      fn index(keys: Document) -> IndexInfo {
          IndexInfo { keys, sparse: false, has_partial_filter: false, collation_is_simple: true }
      }

      #[test]
      fn an_index_serves_a_prefix_in_the_same_or_the_reverse_direction() {
          assert!(index_serves_sort(&index(doc! { "a": 1 }), true, &key(&["a"])));
          assert!(index_serves_sort(&index(doc! { "a": -1 }), true, &key(&["a"])), "reverse scan");
          assert!(index_serves_sort(&index(doc! { "a": 1, "b": 1, "c": 1 }), true, &key(&["a", "b"])));
          assert!(index_serves_sort(&index(doc! { "a": -1, "b": -1 }), true, &key(&["a", "b"])));
          assert!(!index_serves_sort(&index(doc! { "a": 1, "b": -1 }), true, &key(&["a", "b"])), "mixed directions");
          assert!(!index_serves_sort(&index(doc! { "b": 1, "a": 1 }), true, &key(&["a", "b"])), "wrong order");
          assert!(!index_serves_sort(&index(doc! { "a": 1 }), true, &key(&["a", "b"])), "too short");
      }

      #[test]
      fn special_indexes_cannot_serve_the_sort() {
          assert!(!index_serves_sort(&index(doc! { "a": "hashed" }), true, &key(&["a"])));
          assert!(!index_serves_sort(&index(doc! { "a": "text" }), true, &key(&["a"])));
          let sparse = IndexInfo { sparse: true, ..index(doc! { "a": 1 }) };
          assert!(!index_serves_sort(&sparse, true, &key(&["a"])));
          let partial = IndexInfo { has_partial_filter: true, ..index(doc! { "a": 1 }) };
          assert!(!index_serves_sort(&partial, true, &key(&["a"])));
      }

      #[test]
      fn a_non_simple_collation_cannot_serve_a_simple_sort() {
          let ci = IndexInfo { collation_is_simple: false, ..index(doc! { "a": 1 }) };
          assert!(!index_serves_sort(&ci, true, &key(&["a"])));
          // An index with no collation inherits the collection's default.
          assert!(!index_serves_sort(&index(doc! { "a": 1 }), false, &key(&["a"])));
      }
  }
  ```

- [ ] **Step 2: Run and watch it fail.** Add `pub mod keyset;` to `compare/mod.rs`.
  Run: `cargo test -p curlybrace-core compare::keyset`
  Expected: FAIL to compile.

- [ ] **Step 3: Write the failing store tests.** Add to the test module in `crates/core/src/test_support/memory_store.rs` (inside the existing `#[cfg(test)] mod tests`, reusing its imports; add `use crate::compare::key::{KeySpec, KeyTuple};` and `use crate::store::{CollectionKind, IndexStatus};`):

  ```rust
      fn keyed(values: &[i32]) -> InMemoryStore {
          let ns = Namespace::new("db", "c");
          InMemoryStore::new().with_docs(
              &ns,
              values.iter().map(|v| doc! { "_id": *v, "n": *v, "tag": if v % 2 == 0 { "even" } else { "odd" } }).collect(),
          )
      }

      #[tokio::test]
      async fn scan_sorted_returns_documents_in_key_order_from_an_inclusive_start() {
          let store = keyed(&[5, 1, 3, 2, 4]);
          let ns = Namespace::new("db", "c");
          let all = store.scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 100).await.unwrap();
          let ids: Vec<i32> = all.iter().map(|d| d.get_i32("_id").unwrap()).collect();
          assert_eq!(ids, [1, 2, 3, 4, 5]);
          let from = KeyTuple(vec![bson::Bson::Int32(3)]);
          let tail = store.scan_sorted(&ns, &doc! {}, &KeySpec::id(), Some(&from), 2).await.unwrap();
          let ids: Vec<i32> = tail.iter().map(|d| d.get_i32("_id").unwrap()).collect();
          assert_eq!(ids, [3, 4], "the start key itself is included, then the limit applies");
      }

      #[tokio::test]
      async fn scan_sorted_applies_the_filter_and_compound_keys() {
          let store = keyed(&[1, 2, 3, 4]);
          let ns = Namespace::new("db", "c");
          let evens = store.scan_sorted(&ns, &doc! { "tag": "even" }, &KeySpec::id(), None, 100).await.unwrap();
          assert_eq!(evens.len(), 2);
          let compound = KeySpec::new(vec!["tag".into(), "n".into()]).unwrap();
          let ordered = store.scan_sorted(&ns, &doc! {}, &compound, None, 100).await.unwrap();
          let ns_order: Vec<i32> = ordered.iter().map(|d| d.get_i32("n").unwrap()).collect();
          assert_eq!(ns_order, [2, 4, 1, 3], "tag then n");
      }

      #[tokio::test]
      async fn scan_sorted_orders_mixed_types_like_the_server() {
          let ns = Namespace::new("db", "c");
          let store = InMemoryStore::new().with_docs(
              &ns,
              vec![
                  doc! { "_id": "b" },
                  doc! { "_id": 2_i64 },
                  doc! { "_id": bson::oid::ObjectId::from_bytes([1; 12]) },
                  doc! { "_id": 1.5_f64 },
                  doc! { "_id": "a" },
              ],
          );
          let all = store.scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 100).await.unwrap();
          let ids: Vec<bson::Bson> = all.iter().map(|d| d.get("_id").unwrap().clone()).collect();
          assert_eq!(ids[0], bson::Bson::Double(1.5));
          assert_eq!(ids[1], bson::Bson::Int64(2));
          assert_eq!(ids[2], bson::Bson::String("a".into()));
          assert_eq!(ids[3], bson::Bson::String("b".into()));
          assert!(matches!(ids[4], bson::Bson::ObjectId(_)));
      }

      #[tokio::test]
      async fn scan_sorted_refuses_missing_and_invalid_keys() {
          let ns = Namespace::new("db", "c");
          let store = InMemoryStore::new().with_docs(&ns, vec![doc! { "_id": 1 }, doc! { "_id": 2, "k": [1] }]);
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          assert_eq!(
              store.scan_sorted(&ns, &doc! {}, &spec, None, 10).await,
              Err(CoreError::CompareKeyMissing)
          );
      }

      #[tokio::test]
      async fn a_missing_namespace_behaves_as_empty() {
          let store = InMemoryStore::new();
          let ns = Namespace::new("nope", "nothing");
          assert_eq!(store.estimated_count(&ns).await.unwrap(), 0);
          assert!(store.scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 10).await.unwrap().is_empty());
          assert_eq!(store.key_index_status(&ns, &KeySpec::id()).await.unwrap(), IndexStatus::Indexed);
      }

      #[tokio::test]
      async fn counts_and_kinds_reflect_the_stored_collections() {
          let store = keyed(&[1, 2, 3]);
          let ns = Namespace::new("db", "c");
          assert_eq!(store.estimated_count(&ns).await.unwrap(), 3);
          assert_eq!(
              store.list_collection_kinds("db").await.unwrap(),
              vec![("c".to_string(), CollectionKind::Collection)]
          );
      }
  ```

- [ ] **Step 4: Implement `keyset.rs`.** Above the test module:

  ```rust
  //! Pure helpers behind `MongoStore::scan_sorted` and `key_index_status`, kept free of the driver
  //! so they are unit-testable.
  //!
  //! MongoDB's `$gt` and `$gte` only match values of the SAME type class (numbers with numbers,
  //! strings with strings), while its sort order is total across classes. A plain `$gt` keyset
  //! would therefore skip everything after a change of type class. Each key field's condition is
  //! "at or after `v` within its class, OR of a higher class", where the higher classes are
  //! written as "not one of the classes ranked at or below v's".

  use bson::{Bson, Document, doc};

  use super::key::{KeySpec, KeyTuple};

  const CLASSES: [&[&str]; 16] = [
      &["minKey"],
      &["null", "undefined"],
      &["int", "long", "double", "decimal"],
      &["string", "symbol"],
      &["object"],
      &["array"],
      &["binData"],
      &["objectId"],
      &["bool"],
      &["date"],
      &["timestamp"],
      &["regex"],
      &["dbPointer"],
      &["javascript"],
      &["javascriptWithScope"],
      &["maxKey"],
  ];

  /// The `$type` names of every class ranked at or below `rank` (see `key::type_rank`).
  pub fn type_class_names(rank: u8) -> &'static [&'static str] {
      // Built once per rank on first use: a rank-indexed prefix of CLASSES, flattened.
      static TABLE: std::sync::OnceLock<Vec<Vec<&'static str>>> = std::sync::OnceLock::new();
      let table = TABLE.get_or_init(|| {
          (0..CLASSES.len())
              .map(|r| CLASSES[..=r].iter().flat_map(|c| c.iter().copied()).collect())
              .collect()
      });
      let idx = usize::from(rank).min(table.len() - 1);
      &table[idx]
  }

  fn rank_of(value: &Bson) -> u8 {
      match value {
          Bson::MinKey => 0,
          Bson::Null | Bson::Undefined => 1,
          Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_) => 2,
          Bson::String(_) | Bson::Symbol(_) => 3,
          Bson::Document(_) => 4,
          Bson::Array(_) => 5,
          Bson::Binary(_) => 6,
          Bson::ObjectId(_) => 7,
          Bson::Boolean(_) => 8,
          Bson::DateTime(_) => 9,
          Bson::Timestamp(_) => 10,
          Bson::RegularExpression(_) => 11,
          Bson::DbPointer(_) => 12,
          Bson::JavaScriptCode(_) => 13,
          Bson::JavaScriptCodeWithScope(_) => 14,
          Bson::MaxKey => 15,
      }
  }

  /// "`field` is after `value`": comparison within the class, or any higher class. `inclusive`
  /// makes the in-class comparison `>=`.
  fn after(field: &str, value: &Bson, inclusive: bool) -> Document {
      let operator = if inclusive { "$gte" } else { "$gt" };
      let higher = doc! { field: { "$not": { "$type": type_class_names(rank_of(value)).iter().map(|n| Bson::String((*n).into())).collect::<Vec<_>>() } } };
      // `null`, `minKey` and `maxKey` only compare equal to themselves in a query, so the
      // in-class branch is omitted for them (an exclusive start has nothing in its own class).
      let own_class_branch = match value {
          Bson::Null | Bson::Undefined | Bson::MinKey | Bson::MaxKey => {
              inclusive.then(|| doc! { field: value.clone() })
          }
          _ => Some(doc! { field: { operator: value.clone() } }),
      };
      match own_class_branch {
          Some(own) => doc! { "$or": [own, higher] },
          None => higher,
      }
  }

  /// The filter for "documents matching `user_filter` whose key is at or after `from`".
  pub fn keyset_filter(user_filter: &Document, key: &KeySpec, from: Option<&KeyTuple>) -> Document {
      let Some(from) = from else {
          return user_filter.clone();
      };
      let last = key.fields.len() - 1;
      let mut branches: Vec<Bson> = Vec::new();
      for i in 0..key.fields.len() {
          let mut branch = Document::new();
          for (field, value) in key.fields[..i].iter().zip(&from.0) {
              branch.insert(field.clone(), value.clone());
          }
          let tail = after(&key.fields[i], &from.0[i], i == last);
          if branch.is_empty() {
              branch = tail;
          } else {
              for (k, v) in tail {
                  branch.insert(k, v);
              }
          }
          branches.push(Bson::Document(branch));
      }
      let keyset = if branches.len() == 1 {
          match branches.remove(0) {
              Bson::Document(d) => d,
              _ => Document::new(),
          }
      } else {
          doc! { "$or": branches }
      };
      if user_filter.is_empty() {
          keyset
      } else {
          doc! { "$and": [user_filter.clone(), keyset] }
      }
  }

  /// What `key_index_status` needs to know about one index.
  #[derive(Clone, Debug)]
  pub struct IndexInfo {
      pub keys: Document,
      pub sparse: bool,
      pub has_partial_filter: bool,
      /// The index carries an explicit simple collation.
      pub collation_is_simple: bool,
  }

  /// Whether `index` can serve `sort(key fields ascending)` under simple collation: a plain
  /// (not sparse, partial, hashed or text) index whose leading fields are the key fields in order,
  /// all in the same direction (a uniformly reversed index is read backwards), with no
  /// non-simple collation.
  pub fn index_serves_sort(
      index: &IndexInfo,
      collection_default_is_simple: bool,
      key: &KeySpec,
  ) -> bool {
      if index.sparse || index.has_partial_filter {
          return false;
      }
      // An index created without a collation inherits the collection's default.
      if !index.collation_is_simple && !collection_default_is_simple {
          return false;
      }
      if key.fields.len() > index.keys.len() {
          return false;
      }
      let mut direction: Option<i64> = None;
      for ((name, value), wanted) in index.keys.iter().zip(&key.fields) {
          if name != wanted {
              return false;
          }
          let this = match value {
              Bson::Int32(n) => i64::from(*n),
              Bson::Int64(n) => *n,
              Bson::Double(d) if *d == 1.0 || *d == -1.0 => *d as i64,
              _ => return false,
          };
          if this != 1 && this != -1 {
              return false;
          }
          match direction {
              None => direction = Some(this),
              Some(d) if d != this => return false,
              _ => {}
          }
      }
      true
  }
  ```

  Reconcile the unit test expectations: the first `keyset_filter` test expects `with no start the filter is the user's filter`; the `$or` single-key shape expects `{ "$or": [ {"_id": {"$gte": 5}}, {"_id": {"$not": {"$type": [...]}}} ] }`. The compound test's expected `branches[1]` key order is `"a"` then `"$or"`. If your `doc!` key order differs from the expectation, fix the implementation to build exactly what the tests state (they are the contract), not the other way round. NOTE the `IndexInfo::collation_is_simple` test (`an index with no collation inherits the collection default`) passes `collation_is_simple: true` for a plain index in the helper; the helper's field means "explicit simple collation OR no collation" ONLY if the collection default is simple; `MongoStore` therefore sets it to `true` only when the index has an explicit simple collation and `false` when the index has an explicit non-simple one, and passes `None`-collation indexes as `collation_is_simple: true` together with `collection_default_is_simple` taken from the collection options. Document this at the field.

- [ ] **Step 5: Add the types and trait methods.** In `crates/core/src/store.rs` add (below `Page`):

  ```rust
  /// Whether an index can serve a compare's key-ordered scan.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum IndexStatus {
      Indexed,
      NotIndexed,
  }

  /// What a collection is. A view can be read but never written.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum CollectionKind {
      Collection,
      View,
      Timeseries,
  }
  ```

  and, inside `trait DocumentStore` (after `delete`), the four methods with default bodies (add `use crate::compare::key::{KeySpec, KeyTuple};` and `use crate::error::CoreError;`):

  ```rust
      /// Estimated number of documents (fast, approximate). A missing collection counts as 0.
      async fn estimated_count(&self, _ns: &Namespace) -> Result<u64> {
          Err(CoreError::Unsupported("this store cannot count documents".into()))
      }

      /// Documents matching `filter` whose key tuple is at or after `from` (all of them when
      /// `None`), in ascending key order under BINARY (simple) collation, at most `limit`.
      /// The start key is INCLUDED so a caller can detect a duplicate that straddles two batches.
      /// A document missing a key field yields `CompareKeyMissing`; an unusable key value
      /// yields `CompareKeyInvalid`. A missing collection behaves as empty.
      async fn scan_sorted(
          &self,
          _ns: &Namespace,
          _filter: &Document,
          _key: &KeySpec,
          _from: Option<&KeyTuple>,
          _limit: u32,
      ) -> Result<Vec<Document>> {
          Err(CoreError::Unsupported("this store cannot scan in key order".into()))
      }

      /// Whether an index can serve `scan_sorted` for `key`. A missing collection is `Indexed`
      /// (there is nothing to sort).
      async fn key_index_status(&self, _ns: &Namespace, _key: &KeySpec) -> Result<IndexStatus> {
          Err(CoreError::Unsupported("this store cannot inspect indexes".into()))
      }

      /// The collections of `db` with their kind (`system.*` included).
      async fn list_collection_kinds(&self, _db: &str) -> Result<Vec<(String, CollectionKind)>> {
          Err(CoreError::Unsupported("this store cannot list collection kinds".into()))
      }
  ```

- [ ] **Step 6: Implement the in-memory store methods.** In `crates/core/src/test_support/memory_store.rs` add to `impl DocumentStore for InMemoryStore`:

  ```rust
      async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
          let guard = self.lock()?;
          Ok(guard.get(&key(ns)).map_or(0, |docs| docs.len() as u64))
      }

      async fn scan_sorted(
          &self,
          ns: &Namespace,
          filter: &Document,
          key_spec: &KeySpec,
          from: Option<&KeyTuple>,
          limit: u32,
      ) -> Result<Vec<Document>> {
          let guard = self.lock()?;
          let docs = guard.get(&key(ns)).map(Vec::as_slice).unwrap_or_default();
          let mut keyed: Vec<(KeyTuple, &Document)> = Vec::new();
          for doc in docs {
              if !query_eval::matches_filter(doc, filter)? {
                  continue;
              }
              keyed.push((extract_key(doc, key_spec)?, doc));
          }
          keyed.sort_by(|a, b| compare_tuples(&a.0, &b.0));
          Ok(keyed
              .into_iter()
              .filter(|(k, _)| from.is_none_or(|f| compare_tuples(k, f) != std::cmp::Ordering::Less))
              .take(limit as usize)
              .map(|(_, d)| d.clone())
              .collect())
      }

      async fn key_index_status(&self, _ns: &Namespace, _key: &KeySpec) -> Result<IndexStatus> {
          Ok(IndexStatus::Indexed)
      }

      async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
          let guard = self.lock()?;
          let mut names: Vec<String> = guard
              .keys()
              .filter(|(d, _)| d == db)
              .map(|(_, c)| c.clone())
              .collect();
          names.sort();
          Ok(names.into_iter().map(|n| (n, CollectionKind::Collection)).collect())
      }
  ```

  `query_eval::matches_filter` is the existing equality-filter matcher the in-memory `find` already uses. Open `query_eval.rs`, find that function (its exact name may differ, for example `matches`), and make it `pub(super)` with the signature `fn matches_filter(doc: &Document, filter: &Document) -> Result<bool>`; if it does not exist in that form, add a thin wrapper with that signature around the existing logic and test it. Add the needed imports at the top of `memory_store.rs` (`use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key};`, `use crate::store::{CollectionKind, IndexStatus};`, `use super::query_eval;`).

- [ ] **Step 7: Run the store tests.**
  Run: `cargo test -p curlybrace-core --features test-support`
  Expected: PASS.

- [ ] **Step 8: Implement the Mongo store methods.** In `crates/core/src/mongo_store.rs` add imports (`use crate::compare::key::{KeySpec, KeyTuple}; use crate::compare::keyset::{IndexInfo, index_serves_sort, keyset_filter}; use crate::store::{CollectionKind, IndexStatus}; use mongodb::options::Collation;`) and, inside `impl DocumentStore for MongoStore`:

  ```rust
      async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
          self.collection(ns).estimated_document_count().await.map_err(db_err)
      }

      async fn scan_sorted(
          &self,
          ns: &Namespace,
          filter: &Document,
          key: &KeySpec,
          from: Option<&KeyTuple>,
          limit: u32,
      ) -> Result<Vec<Document>> {
          let sort: Document = key.fields.iter().map(|f| (f.clone(), Bson::Int32(1))).collect();
          let cursor = self
              .collection(ns)
              .find(keyset_filter(filter, key, from))
              .sort(sort)
              .limit(i64::from(limit))
              .collation(Collation::builder().locale("simple").build())
              .allow_disk_use(true)
              .await
              .map_err(db_err)?;
          cursor.try_collect().await.map_err(db_err)
      }

      async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus> {
          use mongodb::results::CollectionType;
          let db = self.client.database(&ns.db);
          // A missing collection has nothing to sort.
          let mut specs = db
              .list_collections()
              .filter(doc! { "name": &ns.collection })
              .await
              .map_err(db_err)?;
          let Some(spec) = specs.try_next().await.map_err(db_err)? else {
              return Ok(IndexStatus::Indexed);
          };
          if spec.collection_type == CollectionType::View {
              return Ok(IndexStatus::NotIndexed);
          }
          let default_is_simple = spec
              .options
              .collation
              .as_ref()
              .is_none_or(|c| c.locale == "simple");
          let mut indexes = self.collection(ns).list_indexes().await.map_err(db_err)?;
          while let Some(model) = indexes.try_next().await.map_err(db_err)? {
              let options = model.options.unwrap_or_default();
              let info = IndexInfo {
                  keys: model.keys,
                  sparse: options.sparse.unwrap_or(false),
                  has_partial_filter: options.partial_filter_expression.is_some(),
                  collation_is_simple: options
                      .collation
                      .as_ref()
                      .is_none_or(|c| c.locale == "simple"),
              };
              if index_serves_sort(&info, default_is_simple, key) {
                  return Ok(IndexStatus::Indexed);
              }
          }
          Ok(IndexStatus::NotIndexed)
      }

      async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
          use mongodb::results::CollectionType;
          let mut specs = self.client.database(db).list_collections().await.map_err(db_err)?;
          let mut out = Vec::new();
          while let Some(spec) = specs.try_next().await.map_err(db_err)? {
              let kind = match spec.collection_type {
                  CollectionType::View => CollectionKind::View,
                  CollectionType::Timeseries => CollectionKind::Timeseries,
                  _ => CollectionKind::Collection,
              };
              out.push((spec.name, kind));
          }
          out.sort_by(|a, b| a.0.cmp(&b.0));
          Ok(out)
      }
  ```

  Build with `cargo check -p curlybrace-core`. If the driver's builder method names differ slightly (`allow_disk_use`, `collation`, `list_collections().filter(..)`), adapt to the installed driver's API; the contract above does not change.

- [ ] **Step 9: Write the tier-1 integration tests.** Create `crates/core/tests/local_compare_store.rs`:

  ```rust
  //! Tier 1 (local server): the compare store contract against a real MongoDB. Writes only to
  //! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.
  //!
  //! These tests are the main defence against the client comparator disagreeing with the server's
  //! sort order. If one fails, DO NOT weaken it: report the failing pair.

  mod support;

  use std::cmp::Ordering;

  use bson::oid::ObjectId;
  use bson::spec::BinarySubtype;
  use bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, doc};
  use curlybrace_core::compare::key::{KeySpec, KeyTuple, compare_bson, extract_key};
  use curlybrace_core::store::{CollectionKind, DocumentStore, IndexStatus, Namespace};
  use futures::TryStreamExt;
  use mongodb::options::Collation;
  use std::str::FromStr;
  use support::{local_enabled, local_mongo};

  fn dec(s: &str) -> Bson {
      Bson::Decimal128(Decimal128::from_str(s).unwrap())
  }

  /// Keys of every type class, with boundary numbers. Each appears once; some are equal across
  /// numeric types and are therefore listed in `EQUAL_GROUPS` instead of here.
  fn fixture_keys() -> Vec<Bson> {
      vec![
          Bson::MinKey,
          Bson::Null,
          Bson::Double(f64::NEG_INFINITY),
          Bson::Int64(i64::MIN),
          Bson::Int32(-5),
          Bson::Double(-0.5),
          Bson::Int32(0),
          dec("0.25"),
          Bson::Int32(1),
          Bson::Int64((1 << 53) + 1),
          Bson::Double(9_007_199_254_740_994.0),
          Bson::Int64(i64::MAX),
          Bson::Double(f64::INFINITY),
          Bson::String("".into()),
          Bson::String("B".into()),
          Bson::String("a".into()),
          Bson::String("ab".into()),
          Bson::String("z".into()),
          Bson::String("é".into()),
          Bson::Document(doc! { "a": 1 }),
          Bson::Document(doc! { "a": 2 }),
          Bson::Document(doc! { "b": 1 }),
          Bson::Document(doc! { "a": 1, "b": 1 }),
          Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![0xff] }),
          Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 0] }),
          Bson::Binary(Binary { subtype: BinarySubtype::Uuid, bytes: vec![0, 0] }),
          Bson::ObjectId(ObjectId::from_bytes([0; 12])),
          Bson::ObjectId(ObjectId::from_bytes([1; 12])),
          Bson::Boolean(false),
          Bson::Boolean(true),
          Bson::DateTime(DateTime::from_millis(-1)),
          Bson::DateTime(DateTime::from_millis(1)),
          Bson::Timestamp(Timestamp { time: 0, increment: 1 }),
          Bson::Timestamp(Timestamp { time: 1, increment: 0 }),
          Bson::RegularExpression(Regex { pattern: "a".into(), options: "".into() }),
          Bson::MaxKey,
      ]
  }

  async fn seed(client: &mongodb::Client, ns: &Namespace, keys: &[Bson]) {
      let docs: Vec<Document> = keys.iter().map(|k| doc! { "k": k.clone() }).collect();
      client.database(&ns.db).collection::<Document>(&ns.collection).insert_many(docs).await.unwrap();
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn the_client_comparator_orders_every_key_type_like_the_server() {
      if !local_enabled("the_client_comparator_orders_every_key_type_like_the_server") {
          return;
      }
      let (dbs, client, _store) = local_mongo().await;
      let ns = dbs.ns("order");
      let keys = fixture_keys();
      seed(&client, &ns, &keys).await;
      let cursor = client
          .database(&ns.db)
          .collection::<Document>(&ns.collection)
          .find(doc! {})
          .sort(doc! { "k": 1 })
          .collation(Collation::builder().locale("simple").build())
          .await
          .unwrap();
      let from_server: Vec<Document> = cursor.try_collect().await.unwrap();
      let server_order: Vec<Bson> = from_server.iter().map(|d| d.get("k").unwrap().clone()).collect();
      // Equal-comparing keys may come back in either order, so compare the SEQUENCE of classes of
      // equality: every adjacent pair must be non-decreasing under OUR comparator.
      for pair in server_order.windows(2) {
          assert_ne!(
              compare_bson(&pair[0], &pair[1]),
              Ordering::Greater,
              "the server put {:?} before {:?} but the client comparator says the opposite",
              pair[0],
              pair[1]
          );
      }
      assert_eq!(server_order.len(), keys.len());
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn equal_numbers_of_different_types_are_equal_to_the_server() {
      if !local_enabled("equal_numbers_of_different_types_are_equal_to_the_server") {
          return;
      }
      let (dbs, client, _store) = local_mongo().await;
      let ns = dbs.ns("equal");
      let group = [Bson::Int32(7), Bson::Int64(7), Bson::Double(7.0), dec("7.00")];
      seed(&client, &ns, &group).await;
      for probe in &group {
          let n = client
              .database(&ns.db)
              .collection::<Document>(&ns.collection)
              .count_documents(doc! { "k": probe.clone() })
              .await
              .unwrap();
          assert_eq!(n, group.len() as u64, "{probe:?} should match all four");
      }
      for a in &group {
          for b in &group {
              assert_eq!(compare_bson(a, b), Ordering::Equal, "{a:?} vs {b:?}");
          }
      }
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn scan_sorted_pages_through_mixed_type_keys_completely_and_in_order() {
      if !local_enabled("scan_sorted_pages_through_mixed_type_keys_completely_and_in_order") {
          return;
      }
      let (dbs, client, store) = local_mongo().await;
      let ns = dbs.ns("mixed");
      let keys = vec![
          Bson::Int32(3), Bson::Int32(1), Bson::String("b".into()), Bson::String("a".into()),
          Bson::ObjectId(ObjectId::from_bytes([5; 12])), Bson::Boolean(true), Bson::Double(2.5),
          Bson::DateTime(DateTime::from_millis(10)), Bson::Null,
      ];
      seed(&client, &ns, &keys).await;
      let spec = KeySpec::new(vec!["k".into()]).unwrap();
      let mut seen: Vec<Bson> = Vec::new();
      let mut from: Option<KeyTuple> = None;
      loop {
          let batch = store.scan_sorted(&ns, &doc! {}, &spec, from.as_ref(), 2).await.unwrap();
          if batch.is_empty() {
              break;
          }
          // The inclusive start returns the previous last row again: drop it.
          let fresh: Vec<&Document> = batch
              .iter()
              .skip(usize::from(from.is_some()))
              .collect();
          for d in &fresh {
              seen.push(d.get("k").unwrap().clone());
          }
          let last = extract_key(batch.last().unwrap(), &spec).unwrap();
          if batch.len() < 2 {
              break;
          }
          from = Some(last);
      }
      assert_eq!(seen.len(), keys.len(), "no document may be skipped at a type change: {seen:?}");
      for pair in seen.windows(2) {
          assert_ne!(compare_bson(&pair[0], &pair[1]), Ordering::Greater, "{pair:?}");
      }
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn scan_sorted_applies_the_user_filter_and_compound_keys() {
      if !local_enabled("scan_sorted_applies_the_user_filter_and_compound_keys") {
          return;
      }
      let (dbs, client, store) = local_mongo().await;
      let ns = dbs.ns("compound");
      let docs: Vec<Document> = (0..20)
          .map(|n| doc! { "tenant": if n % 2 == 0 { "a" } else { "b" }, "n": n })
          .collect();
      client.database(&ns.db).collection::<Document>(&ns.collection).insert_many(docs).await.unwrap();
      let spec = KeySpec::new(vec!["tenant".into(), "n".into()]).unwrap();
      let all = store.scan_sorted(&ns, &doc! { "tenant": "a" }, &spec, None, 100).await.unwrap();
      assert_eq!(all.len(), 10);
      let from = extract_key(&all[3], &spec).unwrap();
      let tail = store.scan_sorted(&ns, &doc! { "tenant": "a" }, &spec, Some(&from), 100).await.unwrap();
      assert_eq!(tail.len(), 7, "from the 4th document, inclusive");
      assert_eq!(tail[0], all[3]);
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn index_status_follows_the_real_indexes_and_collations() {
      if !local_enabled("index_status_follows_the_real_indexes_and_collations") {
          return;
      }
      let (dbs, client, store) = local_mongo().await;
      let db = client.database(&dbs.db);
      let spec = KeySpec::new(vec!["email".into()]).unwrap();
      // _id is always indexed.
      let ns_id = dbs.ns("with_id");
      db.collection::<Document>("with_id").insert_one(doc! { "email": "a" }).await.unwrap();
      assert_eq!(store.key_index_status(&ns_id, &KeySpec::id()).await.unwrap(), IndexStatus::Indexed);
      assert_eq!(store.key_index_status(&ns_id, &spec).await.unwrap(), IndexStatus::NotIndexed);
      let coll = db.collection::<Document>("with_id");
      coll.create_index(mongodb::IndexModel::builder().keys(doc! { "email": 1 }).build()).await.unwrap();
      assert_eq!(store.key_index_status(&ns_id, &spec).await.unwrap(), IndexStatus::Indexed);
      // A case-insensitive index cannot serve a simple-collation sort.
      let ns_ci = dbs.ns("ci");
      let ci = db.collection::<Document>("ci");
      ci.insert_one(doc! { "email": "a" }).await.unwrap();
      ci.create_index(
          mongodb::IndexModel::builder()
              .keys(doc! { "email": 1 })
              .options(
                  mongodb::options::IndexOptions::builder()
                      .collation(Collation::builder().locale("en").strength(mongodb::collation::CollationStrength::Secondary).build())
                      .build(),
              )
              .build(),
      )
      .await
      .unwrap();
      assert_eq!(store.key_index_status(&ns_ci, &spec).await.unwrap(), IndexStatus::NotIndexed);
      // A missing collection has nothing to sort.
      assert_eq!(store.key_index_status(&dbs.ns("absent"), &spec).await.unwrap(), IndexStatus::Indexed);
  }

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn counts_kinds_and_missing_collections_behave() {
      if !local_enabled("counts_kinds_and_missing_collections_behave") {
          return;
      }
      let (dbs, client, store) = local_mongo().await;
      let db = client.database(&dbs.db);
      db.collection::<Document>("plain").insert_many((0..5).map(|n| doc! { "n": n })).await.unwrap();
      db.create_collection("a_view")
          .view_on("plain")
          .pipeline(vec![doc! { "$match": {} }])
          .await
          .unwrap();
      assert_eq!(store.estimated_count(&dbs.ns("plain")).await.unwrap(), 5);
      assert_eq!(store.estimated_count(&dbs.ns("absent")).await.unwrap(), 0);
      assert!(store.scan_sorted(&dbs.ns("absent"), &doc! {}, &KeySpec::id(), None, 10).await.unwrap().is_empty());
      let kinds = store.list_collection_kinds(&dbs.db).await.unwrap();
      assert!(kinds.contains(&("plain".to_string(), CollectionKind::Collection)), "{kinds:?}");
      assert!(kinds.contains(&("a_view".to_string(), CollectionKind::View)), "{kinds:?}");
  }
  ```

  If a builder call differs in the installed driver (for example `create_collection(..).view_on(..)` or the index option builders), adapt the call but keep the assertion.

- [ ] **Step 10: Run the tier-1 tests against your OWN throwaway container.**
  ```bash
  N="cb-it-dev-$(uuidgen | tr -d - | cut -c1-10 | tr A-Z a-z)"
  docker run -d --name "$N" -p 127.0.0.1::27017 mongo:7 >/dev/null
  PORT=$(docker port "$N" 27017/tcp | head -1 | sed 's/.*://')
  # wait until it accepts connections, then:
  CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" CURLYBRACE_IT=1 \
    cargo test -p curlybrace-core --test local_compare_store -- --include-ignored --nocapture
  docker rm -f -v "$N"
  ```
  NEVER run integration tests against `127.0.0.1:27017` or any container you did not start (the user's own containers `local7979` and `local265` exist).
  Expected: PASS. **If `the_client_comparator_orders_every_key_type_like_the_server` fails**, the message names the pair. Stop and report DONE_WITH_CONCERNS with that pair; do not edit the fixture or loosen the assertion. The most likely candidates are Double-versus-Decimal128 boundary values, in which case the comparator rule (exact value) and the server disagree and the controller will decide whether to remove the pair or adapt `compare_numbers`.

- [ ] **Step 11: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add keyset scan, counts, index status and collection kinds to the store

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```


---

### Task 5: Key scanner and result budget

**Files:**
- Create: `crates/core/src/compare/scan.rs`, `crates/core/src/compare/results.rs`
- Modify: `crates/core/src/compare/mod.rs`

**Interfaces:**
- Consumes: `DocumentStore::scan_sorted`, `KeySpec`, `KeyTuple`, `extract_key`, `compare_tuples`, `Fingerprint`.
- Produces:
  ```rust
  // scan.rs
  pub struct KeyScanner { /* private */ }
  impl KeyScanner {
      pub fn new(store: Arc<dyn DocumentStore>, ns: Namespace, filter: Document, key: KeySpec, batch: u32) -> Self;
      pub async fn peek(&mut self) -> Result<Option<&(KeyTuple, Document)>>;
      pub async fn next(&mut self) -> Result<Option<(KeyTuple, Document)>>;
      pub fn scanned(&self) -> u64;               // documents handed out so far
  }
  // results.rs
  pub enum Category { OnlyInSource, OnlyInTarget, Different }          // Clone, Copy, Debug, PartialEq, Eq, Hash
  pub struct DiffEntry { pub key: KeyTuple, pub source: Option<Fingerprint>, pub target: Option<Fingerprint> }  // Clone, Debug, PartialEq
  pub struct Limits { pub max_differences: usize, pub max_result_bytes: usize, pub batch_size: u32, pub max_unindexed_documents: u64 }   // Default per Global Constraints; Clone, Copy, Debug
  pub struct Budget { /* Arc inside; Clone */ }
  impl Budget { pub fn new(limits: Limits) -> Self; pub fn limits(&self) -> Limits; pub fn used_entries(&self) -> usize; pub fn used_bytes(&self) -> usize }
  pub struct ResultSet { /* private */ }
  impl ResultSet {
      pub fn new(budget: Budget) -> Self;
      pub fn push(&mut self, category: Category, entry: DiffEntry) -> Result<()>;   // CompareCapExceeded when over budget
      pub fn count(&self, category: Category) -> usize;
      pub fn page(&self, category: Category, offset: usize, limit: usize) -> &[DiffEntry];
      pub fn find(&self, category: Category, key: &KeyTuple) -> Option<&DiffEntry>;
  }                                                    // Drop returns the share to the budget
  ```

- [ ] **Step 1: Write the failing scanner tests.** Create `crates/core/src/compare/scan.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::test_support::InMemoryStore;
      use bson::{Bson, doc};
      use std::sync::atomic::{AtomicUsize, Ordering};

      fn ns() -> Namespace {
          Namespace::new("db", "c")
      }

      fn scanner(docs: Vec<Document>, key: KeySpec, batch: u32) -> KeyScanner {
          let store = InMemoryStore::new().with_docs(&ns(), docs);
          KeyScanner::new(Arc::new(store), ns(), doc! {}, key, batch)
      }

      async fn drain(s: &mut KeyScanner) -> Result<Vec<Document>> {
          let mut out = Vec::new();
          while let Some((_, d)) = s.next().await? {
              out.push(d);
          }
          Ok(out)
      }

      #[tokio::test]
      async fn yields_every_document_once_in_key_order_across_batches() {
          for batch in [1, 2, 3, 7, 100] {
              let docs: Vec<Document> = [5, 3, 9, 1, 7, 2, 8].iter().map(|n| doc! { "_id": *n }).collect();
              let mut s = scanner(docs, KeySpec::id(), batch);
              let ids: Vec<i32> = drain(&mut s).await.unwrap().iter().map(|d| d.get_i32("_id").unwrap()).collect();
              assert_eq!(ids, [1, 2, 3, 5, 7, 8, 9], "batch {batch}");
              assert_eq!(s.scanned(), 7);
          }
      }

      #[tokio::test]
      async fn an_empty_collection_yields_nothing() {
          let mut s = scanner(vec![], KeySpec::id(), 3);
          assert!(s.peek().await.unwrap().is_none());
          assert!(s.next().await.unwrap().is_none());
      }

      #[tokio::test]
      async fn peek_does_not_consume() {
          let mut s = scanner(vec![doc! { "_id": 1 }, doc! { "_id": 2 }], KeySpec::id(), 1);
          assert_eq!(s.peek().await.unwrap().unwrap().0, KeyTuple(vec![Bson::Int32(1)]));
          assert_eq!(s.peek().await.unwrap().unwrap().0, KeyTuple(vec![Bson::Int32(1)]));
          assert_eq!(s.next().await.unwrap().unwrap().0, KeyTuple(vec![Bson::Int32(1)]));
          assert_eq!(s.next().await.unwrap().unwrap().0, KeyTuple(vec![Bson::Int32(2)]));
          assert!(s.next().await.unwrap().is_none());
      }

      #[tokio::test]
      async fn a_duplicate_key_is_an_error_wherever_it_sits() {
          // Within a batch, straddling a batch boundary (batch 1 and 2), as the first and as the last rows.
          let dup = |extra: Vec<Document>| {
              let mut d = vec![doc! { "_id": 1, "k": "a" }, doc! { "_id": 2, "k": "a" }];
              d.extend(extra);
              d
          };
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          for batch in [1, 2, 3, 100] {
              for docs in [
                  dup(vec![]),
                  dup(vec![doc! { "_id": 3, "k": "b" }, doc! { "_id": 4, "k": "c" }]),
                  {
                      let mut d = vec![doc! { "_id": 7, "k": "0" }, doc! { "_id": 8, "k": "1" }];
                      d.extend(dup(vec![]));
                      d
                  },
              ] {
                  let mut s = scanner(docs, spec.clone(), batch);
                  assert_eq!(drain(&mut s).await, Err(CoreError::CompareDuplicateKey), "batch {batch}");
              }
          }
      }

      #[tokio::test]
      async fn keys_equal_across_numeric_types_are_duplicates() {
          let docs = vec![doc! { "_id": 1_i32 }, doc! { "_id": 1_i64 }];
          let mut s = scanner(docs, KeySpec::id(), 1);
          assert_eq!(drain(&mut s).await, Err(CoreError::CompareDuplicateKey));
      }

      #[tokio::test]
      async fn a_missing_key_field_is_an_error() {
          let mut s = scanner(vec![doc! { "_id": 1 }], KeySpec::new(vec!["k".into()]).unwrap(), 5);
          assert_eq!(drain(&mut s).await, Err(CoreError::CompareKeyMissing));
      }

      /// A store that ignores the requested order, to prove the scanner does not trust the server.
      struct Misordered(AtomicUsize);

      #[async_trait::async_trait]
      impl DocumentStore for Misordered {
          async fn list_databases(&self) -> Result<Vec<String>> { Ok(vec![]) }
          async fn list_collections(&self, _db: &str) -> Result<Vec<String>> { Ok(vec![]) }
          async fn find(&self, _ns: &Namespace, _q: &crate::store::Query) -> Result<crate::store::Page> {
              Err(CoreError::Unsupported("n/a".into()))
          }
          async fn insert(&self, _ns: &Namespace, _d: Document) -> Result<Bson> { Err(CoreError::Unsupported("n/a".into())) }
          async fn replace(&self, _ns: &Namespace, _id: &Bson, _d: Document) -> Result<()> { Err(CoreError::Unsupported("n/a".into())) }
          async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> { Err(CoreError::Unsupported("n/a".into())) }
          async fn scan_sorted(&self, _: &Namespace, _: &Document, _: &KeySpec, _: Option<&KeyTuple>, _: u32) -> Result<Vec<Document>> {
              self.0.fetch_add(1, Ordering::SeqCst);
              Ok(vec![doc! { "_id": 5 }, doc! { "_id": 2 }])
          }
      }

      #[tokio::test]
      async fn an_out_of_order_stream_is_refused() {
          let mut s = KeyScanner::new(Arc::new(Misordered(AtomicUsize::new(0))), ns(), doc! {}, KeySpec::id(), 10);
          assert_eq!(drain(&mut s).await, Err(CoreError::CompareKeyOrder));
      }

      #[tokio::test]
      async fn it_never_requests_more_than_one_batch_plus_the_overlap_row() {
          struct Counting { inner: InMemoryStore, max_limit: AtomicUsize }
          #[async_trait::async_trait]
          impl DocumentStore for Counting {
              async fn list_databases(&self) -> Result<Vec<String>> { self.inner.list_databases().await }
              async fn list_collections(&self, db: &str) -> Result<Vec<String>> { self.inner.list_collections(db).await }
              async fn find(&self, ns: &Namespace, q: &crate::store::Query) -> Result<crate::store::Page> { self.inner.find(ns, q).await }
              async fn insert(&self, ns: &Namespace, d: Document) -> Result<Bson> { self.inner.insert(ns, d).await }
              async fn replace(&self, ns: &Namespace, id: &Bson, d: Document) -> Result<()> { self.inner.replace(ns, id, d).await }
              async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> { self.inner.delete(ns, id).await }
              async fn scan_sorted(&self, ns: &Namespace, f: &Document, k: &KeySpec, from: Option<&KeyTuple>, limit: u32) -> Result<Vec<Document>> {
                  self.max_limit.fetch_max(limit as usize, Ordering::SeqCst);
                  self.inner.scan_sorted(ns, f, k, from, limit).await
              }
          }
          let docs: Vec<Document> = (0..500).map(|n| doc! { "_id": n }).collect();
          let store = Arc::new(Counting { inner: InMemoryStore::new().with_docs(&ns(), docs), max_limit: AtomicUsize::new(0) });
          let mut s = KeyScanner::new(store.clone(), ns(), doc! {}, KeySpec::id(), 50);
          assert_eq!(drain(&mut s).await.unwrap().len(), 500);
          assert_eq!(store.max_limit.load(Ordering::SeqCst), 51);
      }
  }
  ```

- [ ] **Step 2: Write the failing results tests.** Create `crates/core/src/compare/results.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use bson::Bson;

      fn entry(n: i32) -> DiffEntry {
          DiffEntry { key: KeyTuple(vec![Bson::Int32(n)]), source: None, target: None }
      }

      fn limits(max_differences: usize) -> Limits {
          Limits { max_differences, ..Limits::default() }
      }

      #[test]
      fn defaults_follow_the_spec() {
          let l = Limits::default();
          assert_eq!(l.max_differences, 1_000_000);
          assert_eq!(l.max_result_bytes, 256 * 1024 * 1024);
          assert_eq!(l.batch_size, 1_000);
          assert_eq!(l.max_unindexed_documents, 50_000);
      }

      #[test]
      fn entries_are_counted_paged_and_found_per_category() {
          let mut set = ResultSet::new(Budget::new(Limits::default()));
          for n in 0..5 {
              set.push(Category::OnlyInSource, entry(n)).unwrap();
          }
          set.push(Category::Different, entry(99)).unwrap();
          assert_eq!(set.count(Category::OnlyInSource), 5);
          assert_eq!(set.count(Category::OnlyInTarget), 0);
          assert_eq!(set.page(Category::OnlyInSource, 1, 2), &[entry(1), entry(2)]);
          assert!(set.page(Category::OnlyInSource, 10, 2).is_empty());
          assert_eq!(set.find(Category::Different, &entry(99).key), Some(&entry(99)));
          assert_eq!(set.find(Category::Different, &entry(1).key), None);
      }

      #[test]
      fn the_difference_cap_is_enforced_and_names_the_typed_error() {
          let mut set = ResultSet::new(Budget::new(limits(2)));
          set.push(Category::Different, entry(1)).unwrap();
          set.push(Category::Different, entry(2)).unwrap();
          assert_eq!(set.push(Category::Different, entry(3)), Err(CoreError::CompareCapExceeded));
          assert_eq!(set.count(Category::Different), 2, "the refused entry is not stored");
      }

      #[test]
      fn the_byte_cap_is_enforced() {
          let budget = Budget::new(Limits { max_result_bytes: 100, ..Limits::default() });
          let mut set = ResultSet::new(budget);
          let big = DiffEntry { key: KeyTuple(vec![Bson::String("x".repeat(500))]), source: None, target: None };
          assert_eq!(set.push(Category::Different, big), Err(CoreError::CompareCapExceeded));
      }

      #[test]
      fn the_budget_is_shared_between_sets_and_returned_on_drop() {
          let budget = Budget::new(limits(3));
          let mut a = ResultSet::new(budget.clone());
          let mut b = ResultSet::new(budget.clone());
          a.push(Category::Different, entry(1)).unwrap();
          a.push(Category::Different, entry(2)).unwrap();
          b.push(Category::Different, entry(3)).unwrap();
          assert_eq!(b.push(Category::Different, entry(4)), Err(CoreError::CompareCapExceeded));
          assert_eq!(budget.used_entries(), 3);
          drop(a);
          assert_eq!(budget.used_entries(), 1);
          b.push(Category::Different, entry(4)).unwrap();
          drop(b);
          assert_eq!(budget.used_entries(), 0);
          assert_eq!(budget.used_bytes(), 0, "no byte leaks");
      }
  }
  ```

- [ ] **Step 3: Run and watch both fail.** Add `pub mod results; pub mod scan;` to `compare/mod.rs`.
  Run: `cargo test -p curlybrace-core --features test-support compare::scan compare::results`
  Expected: FAIL to compile.

- [ ] **Step 4: Implement `results.rs`.** Above its test module:

  ```rust
  //! Where a compare keeps what it found: keys and fingerprints, never documents. All result sets
  //! of one compare session draw on one shared [`Budget`], and give their share back when dropped.

  use std::collections::HashMap;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicUsize, Ordering};

  use bson::Bson;

  use super::fingerprint::Fingerprint;
  use super::key::KeyTuple;
  use crate::error::{CoreError, Result};

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum Category {
      OnlyInSource,
      OnlyInTarget,
      Different,
  }

  #[derive(Clone, Debug, PartialEq)]
  pub struct DiffEntry {
      pub key: KeyTuple,
      pub source: Option<Fingerprint>,
      pub target: Option<Fingerprint>,
  }

  #[derive(Clone, Copy, Debug)]
  pub struct Limits {
      pub max_differences: usize,
      pub max_result_bytes: usize,
      pub batch_size: u32,
      pub max_unindexed_documents: u64,
  }

  impl Default for Limits {
      fn default() -> Self {
          Self {
              max_differences: 1_000_000,
              max_result_bytes: 256 * 1024 * 1024,
              batch_size: 1_000,
              max_unindexed_documents: 50_000,
          }
      }
  }

  struct BudgetInner {
      limits: Limits,
      entries: AtomicUsize,
      bytes: AtomicUsize,
  }

  /// A shared allowance of result entries and bytes.
  #[derive(Clone)]
  pub struct Budget(Arc<BudgetInner>);

  /// Fixed overhead per entry (vector slot, two optional fingerprints, key vector header).
  const ENTRY_OVERHEAD: usize = 40;

  fn entry_cost(entry: &DiffEntry) -> usize {
      let values: usize = entry
          .key
          .0
          .iter()
          .map(|v| {
              let wrapped = bson::Document::from_iter([("v".to_string(), v.clone())]);
              bson::to_vec(&wrapped).map_or(64, |b| b.len())
          })
          .sum();
      ENTRY_OVERHEAD + values
  }

  impl Budget {
      pub fn new(limits: Limits) -> Self {
          Self(Arc::new(BudgetInner {
              limits,
              entries: AtomicUsize::new(0),
              bytes: AtomicUsize::new(0),
          }))
      }

      pub fn limits(&self) -> Limits {
          self.0.limits
      }

      pub fn used_entries(&self) -> usize {
          self.0.entries.load(Ordering::SeqCst)
      }

      pub fn used_bytes(&self) -> usize {
          self.0.bytes.load(Ordering::SeqCst)
      }

      fn reserve(&self, bytes: usize) -> Result<()> {
          let inner = &self.0;
          let entries = inner.entries.fetch_add(1, Ordering::SeqCst) + 1;
          let total = inner.bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
          if entries > inner.limits.max_differences || total > inner.limits.max_result_bytes {
              self.release(1, bytes);
              return Err(CoreError::CompareCapExceeded);
          }
          Ok(())
      }

      fn release(&self, entries: usize, bytes: usize) {
          self.0.entries.fetch_sub(entries, Ordering::SeqCst);
          self.0.bytes.fetch_sub(bytes, Ordering::SeqCst);
      }
  }

  /// The differences of one pair.
  pub struct ResultSet {
      budget: Budget,
      entries: HashMap<Category, Vec<DiffEntry>>,
      held_bytes: usize,
  }

  impl ResultSet {
      pub fn new(budget: Budget) -> Self {
          Self { budget, entries: HashMap::new(), held_bytes: 0 }
      }

      pub fn push(&mut self, category: Category, entry: DiffEntry) -> Result<()> {
          let cost = entry_cost(&entry);
          self.budget.reserve(cost)?;
          self.held_bytes += cost;
          self.entries.entry(category).or_default().push(entry);
          Ok(())
      }

      pub fn count(&self, category: Category) -> usize {
          self.entries.get(&category).map_or(0, Vec::len)
      }

      pub fn page(&self, category: Category, offset: usize, limit: usize) -> &[DiffEntry] {
          let all = self.entries.get(&category).map_or(&[][..], Vec::as_slice);
          let start = offset.min(all.len());
          &all[start..(start + limit).min(all.len())]
      }

      /// Entries arrive in key order, so a lookup is a binary search.
      pub fn find(&self, category: Category, key: &KeyTuple) -> Option<&DiffEntry> {
          let all = self.entries.get(&category)?;
          all.binary_search_by(|e| super::key::compare_tuples(&e.key, key))
              .ok()
              .map(|i| &all[i])
      }
  }

  impl Drop for ResultSet {
      fn drop(&mut self) {
          let count: usize = self.entries.values().map(Vec::len).sum();
          self.budget.release(count, self.held_bytes);
      }
  }

  #[allow(dead_code)]
  fn _assert_bson_used(_: &Bson) {}
  ```
  Remove the `_assert_bson_used` helper and the `use bson::Bson;` import if the compiler reports Bson unused outside tests (the tests import it themselves).

- [ ] **Step 5: Implement `scan.rs`.** Above its test module:

  ```rust
  //! Streams a collection's documents in key order, one batch at a time, and checks the stream.
  //!
  //! The scanner never trusts the server: every document must have a usable key, and consecutive
  //! keys must be strictly increasing under the client comparator. Equal keys are a duplicate
  //! (`CompareDuplicateKey`); a decrease is `CompareKeyOrder`. Because `scan_sorted` includes the
  //! start key, each fetch after the first begins with the row already handed out; it is
  //! discarded, and a SECOND row with the same key is a duplicate straddling the batch boundary.

  use std::cmp::Ordering;
  use std::collections::VecDeque;
  use std::sync::Arc;

  use bson::Document;

  use super::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
  use crate::error::{CoreError, Result};
  use crate::store::{DocumentStore, Namespace};

  pub struct KeyScanner {
      store: Arc<dyn DocumentStore>,
      ns: Namespace,
      filter: Document,
      key: KeySpec,
      batch: u32,
      buffer: VecDeque<(KeyTuple, Document)>,
      /// The key of the last row handed out (or buffered): the next fetch starts here.
      last: Option<KeyTuple>,
      exhausted: bool,
      scanned: u64,
  }

  impl KeyScanner {
      pub fn new(
          store: Arc<dyn DocumentStore>,
          ns: Namespace,
          filter: Document,
          key: KeySpec,
          batch: u32,
      ) -> Self {
          Self {
              store,
              ns,
              filter,
              key,
              batch: batch.max(1),
              buffer: VecDeque::new(),
              last: None,
              exhausted: false,
              scanned: 0,
          }
      }

      pub fn scanned(&self) -> u64 {
          self.scanned
      }

      pub async fn peek(&mut self) -> Result<Option<&(KeyTuple, Document)>> {
          self.fill().await?;
          Ok(self.buffer.front())
      }

      pub async fn next(&mut self) -> Result<Option<(KeyTuple, Document)>> {
          self.fill().await?;
          let row = self.buffer.pop_front();
          if row.is_some() {
              self.scanned += 1;
          }
          Ok(row)
      }

      async fn fill(&mut self) -> Result<()> {
          while self.buffer.is_empty() && !self.exhausted {
              self.fetch().await?;
          }
          Ok(())
      }

      async fn fetch(&mut self) -> Result<()> {
          // After the first fetch the start key comes back as an overlap row, hence one extra.
          let limit = if self.last.is_some() { self.batch + 1 } else { self.batch };
          let docs = self
              .store
              .scan_sorted(&self.ns, &self.filter, &self.key, self.last.as_ref(), limit)
              .await?;
          self.exhausted = docs.len() < limit as usize;
          let mut skipped_overlap = self.last.is_none();
          for doc in docs {
              let key = extract_key(&doc, &self.key)?;
              match &self.last {
                  None => {}
                  Some(previous) => match compare_tuples(previous, &key) {
                      Ordering::Less => {}
                      Ordering::Equal if !skipped_overlap => {
                          skipped_overlap = true;
                          continue;
                      }
                      Ordering::Equal => return Err(CoreError::CompareDuplicateKey),
                      Ordering::Greater => return Err(CoreError::CompareKeyOrder),
                  },
              }
              skipped_overlap = true;
              self.last = Some(key.clone());
              self.buffer.push_back((key, doc));
          }
          Ok(())
      }
  }
  ```

  Walk-through to confirm before running: first fetch has `last = None`, so no row is skipped and rows must strictly increase against the running `last` (updated per row). Later fetches start with `skipped_overlap = false`: the first equal row is the overlap, discarded; a later equal row is a duplicate. If the overlap row has been deleted meanwhile, the first row is `Less`-than-`key` and is kept (fine). Run the tests; if the duplicate test with batch 1 fails, re-check `limit = batch + 1`.

- [ ] **Step 6: Run the tests.**
  Run: `cargo test -p curlybrace-core --features test-support compare::`
  Expected: PASS.

- [ ] **Step 7: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add the key-ordered scanner and the shared result budget

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 6: The merge-join engine

**Files:**
- Create: `crates/core/src/compare/engine.rs`
- Modify: `crates/core/src/compare/mod.rs`, `crates/core/Cargo.toml` (dev-dependency `proptest = "1.5"`)

**Interfaces:**
- Consumes: `KeyScanner`, `ResultSet`, `Budget`, `Limits`, `CompareRules`, `fingerprint_of`, `compare_tuples`.
- Produces:
  ```rust
  pub struct PairInput {
      pub source: Arc<dyn DocumentStore>, pub source_ns: Namespace, pub source_filter: Document,
      pub target: Arc<dyn DocumentStore>, pub target_ns: Namespace, pub target_filter: Document,
      pub key: KeySpec, pub rules: CompareRules, pub budget: Budget,
  }
  pub struct Summary { pub equal: u64, pub only_in_source: u64, pub only_in_target: u64, pub different: u64,
                       pub scanned_source: u64, pub scanned_target: u64 }       // Clone, Copy, Debug, Default, PartialEq, Eq
  pub struct Progress { pub scanned_source: u64, pub scanned_target: u64, pub differences: u64 }   // Clone, Copy, Debug, PartialEq, Eq
  pub enum EngineOutcome { Completed, Cancelled }                                // Clone, Copy, Debug, PartialEq, Eq
  pub struct CancelFlag(Arc<AtomicBool>);   // Clone, Default; fn cancel(&self); fn is_cancelled(&self) -> bool
  pub async fn run_pair(input: PairInput, cancel: &CancelFlag, on_progress: &mut dyn FnMut(Progress))
      -> Result<(EngineOutcome, Summary, ResultSet)>;
  ```

- [ ] **Step 1: Add the dev-dependency** `proptest = "1.5"` under `[dev-dependencies]` in `crates/core/Cargo.toml`.

- [ ] **Step 2: Write the failing tests.** Create `crates/core/src/compare/engine.rs` with only:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::compare::fingerprint::fingerprint_of;
      use crate::compare::results::{Category, Limits};
      use crate::test_support::InMemoryStore;
      use bson::{Bson, doc};
      use proptest::prelude::*;
      use std::collections::BTreeMap;

      fn ns(c: &str) -> Namespace {
          Namespace::new("db", c)
      }

      fn input(source: Vec<Document>, target: Vec<Document>, limits: Limits) -> PairInput {
          let s = InMemoryStore::new().with_docs(&ns("a"), source);
          let t = InMemoryStore::new().with_docs(&ns("b"), target);
          PairInput {
              source: Arc::new(s),
              source_ns: ns("a"),
              source_filter: doc! {},
              target: Arc::new(t),
              target_ns: ns("b"),
              target_filter: doc! {},
              key: KeySpec::id(),
              rules: CompareRules::for_key(&KeySpec::id()),
              budget: Budget::new(limits),
          }
      }

      async fn run(i: PairInput) -> Result<(EngineOutcome, Summary, ResultSet)> {
          run_pair(i, &CancelFlag::default(), &mut |_| {}).await
      }

      fn ids(set: &ResultSet, c: Category) -> Vec<i32> {
          set.page(c, 0, usize::MAX)
              .iter()
              .map(|e| match &e.key.0[0] { Bson::Int32(n) => *n, other => panic!("{other:?}") })
              .collect()
      }

      #[tokio::test]
      async fn identical_collections_are_all_equal_and_keep_nothing() {
          let docs: Vec<Document> = (0..250).map(|n| doc! { "_id": n, "v": n * 2 }).collect();
          let limits = Limits { batch_size: 40, ..Limits::default() };
          let i = input(docs.clone(), docs, limits);
          let budget = i.budget.clone();
          let (outcome, summary, set) = run(i).await.unwrap();
          assert_eq!(outcome, EngineOutcome::Completed);
          assert_eq!(summary, Summary { equal: 250, scanned_source: 250, scanned_target: 250, ..Summary::default() });
          assert_eq!(budget.used_entries(), 0, "equal documents cost no memory");
          assert_eq!(set.count(Category::Different), 0);
      }

      #[tokio::test]
      async fn classifies_missing_extra_and_different_documents() {
          let source = vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 2 }, doc! { "_id": 3, "v": 3 }, doc! { "_id": 5, "v": 5 }];
          let target = vec![doc! { "_id": 2, "v": 2 }, doc! { "_id": 3, "v": 99 }, doc! { "_id": 4, "v": 4 }, doc! { "_id": 5, "v": 5 }];
          let (_, summary, set) = run(input(source, target, Limits { batch_size: 2, ..Limits::default() })).await.unwrap();
          assert_eq!(ids(&set, Category::OnlyInSource), [1]);
          assert_eq!(ids(&set, Category::OnlyInTarget), [4]);
          assert_eq!(ids(&set, Category::Different), [3]);
          assert_eq!((summary.equal, summary.only_in_source, summary.only_in_target, summary.different), (2, 1, 1, 1));
          let diff = set.page(Category::Different, 0, 1)[0].clone();
          assert!(diff.source.is_some() && diff.target.is_some() && diff.source != diff.target);
      }

      #[tokio::test]
      async fn one_empty_side_and_two_empty_sides() {
          let docs: Vec<Document> = (0..5).map(|n| doc! { "_id": n }).collect();
          let (_, s, set) = run(input(docs.clone(), vec![], Limits::default())).await.unwrap();
          assert_eq!((s.only_in_source, ids(&set, Category::OnlyInSource).len()), (5, 5));
          let (_, s, _) = run(input(vec![], docs, Limits::default())).await.unwrap();
          assert_eq!(s.only_in_target, 5);
          let (outcome, s, _) = run(input(vec![], vec![], Limits::default())).await.unwrap();
          assert_eq!((outcome, s), (EngineOutcome::Completed, Summary::default()));
      }

      #[tokio::test]
      async fn rules_decide_what_counts_as_different() {
          let source = vec![doc! { "_id": 1, "n": 1_i32, "ts": 1 }];
          let target = vec![doc! { "_id": 1, "n": 1_i64, "ts": 2 }];
          let strict = run(input(source.clone(), target.clone(), Limits::default())).await.unwrap();
          assert_eq!(strict.1.different, 1);
          let mut i = input(source, target, Limits::default());
          i.rules.numbers_equal_across_types = true;
          i.rules.field_scope = crate::compare::rules::FieldScope::AllExcept(vec!["ts".into()]);
          assert_eq!(run(i).await.unwrap().1.equal, 1);
      }

      #[tokio::test]
      async fn a_business_key_matches_documents_with_different_ids() {
          let key = KeySpec::new(vec!["email".into()]).unwrap();
          let mut i = input(
              vec![doc! { "_id": 1, "email": "a", "v": 1 }],
              vec![doc! { "_id": 77, "email": "a", "v": 1 }],
              Limits::default(),
          );
          i.rules = CompareRules::for_key(&key);
          i.key = key;
          assert_eq!(run(i).await.unwrap().1.equal, 1, "_id is not compared when the key is a business key");
      }

      #[tokio::test]
      async fn the_difference_cap_aborts_with_the_typed_error() {
          let source: Vec<Document> = (0..10).map(|n| doc! { "_id": n }).collect();
          let i = input(source, vec![], Limits { max_differences: 3, ..Limits::default() });
          assert!(matches!(run(i).await, Err(CoreError::CompareCapExceeded)));
      }

      #[tokio::test]
      async fn a_duplicate_on_either_side_aborts() {
          let key = KeySpec::new(vec!["k".into()]).unwrap();
          let dup = vec![doc! { "_id": 1, "k": 1 }, doc! { "_id": 2, "k": 1 }];
          let mut i = input(dup.clone(), vec![], Limits::default());
          i.key = key.clone();
          i.rules = CompareRules::for_key(&key);
          assert!(matches!(run(i).await, Err(CoreError::CompareDuplicateKey)));
          let mut i = input(vec![], dup, Limits::default());
          i.key = key.clone();
          i.rules = CompareRules::for_key(&key);
          assert!(matches!(run(i).await, Err(CoreError::CompareDuplicateKey)));
      }

      #[tokio::test]
      async fn cancel_stops_the_run_and_reports_the_partial_summary() {
          let docs: Vec<Document> = (0..500).map(|n| doc! { "_id": n }).collect();
          let i = input(docs, vec![], Limits { batch_size: 10, ..Limits::default() });
          let cancel = CancelFlag::default();
          let mut seen = 0;
          let c2 = cancel.clone();
          let (outcome, summary, _) = run_pair(i, &cancel, &mut |_| {
              seen += 1;
              if seen == 2 {
                  c2.cancel();
              }
          })
          .await
          .unwrap();
          assert_eq!(outcome, EngineOutcome::Cancelled);
          assert!(summary.only_in_source < 500);
      }

      #[tokio::test]
      async fn progress_is_reported_and_monotonic() {
          let docs: Vec<Document> = (0..120).map(|n| doc! { "_id": n }).collect();
          let i = input(docs.clone(), docs, Limits { batch_size: 25, ..Limits::default() });
          let mut reports: Vec<Progress> = Vec::new();
          run_pair(i, &CancelFlag::default(), &mut |p| reports.push(p)).await.unwrap();
          assert!(reports.len() >= 3);
          for pair in reports.windows(2) {
              assert!(pair[1].scanned_source >= pair[0].scanned_source);
          }
      }

      fn brute_force(source: &BTreeMap<i32, i32>, target: &BTreeMap<i32, i32>) -> (Vec<i32>, Vec<i32>, Vec<i32>, u64) {
          let (mut only_s, mut only_t, mut diff, mut equal) = (vec![], vec![], vec![], 0);
          for (k, v) in source {
              match target.get(k) {
                  None => only_s.push(*k),
                  Some(t) if t != v => diff.push(*k),
                  Some(_) => equal += 1,
              }
          }
          only_t.extend(target.keys().filter(|k| !source.contains_key(k)));
          (only_s, only_t, diff, equal)
      }

      proptest! {
          #![proptest_config(ProptestConfig::with_cases(64))]
          #[test]
          fn the_merge_join_agrees_with_a_brute_force_comparison(
              source in proptest::collection::btree_map(0..40i32, 0..3i32, 0..30),
              target in proptest::collection::btree_map(0..40i32, 0..3i32, 0..30),
              batch in 1u32..8,
          ) {
              let to_docs = |m: &BTreeMap<i32, i32>| m.iter().map(|(k, v)| doc! { "_id": *k, "v": *v }).collect::<Vec<_>>();
              let i = input(to_docs(&source), to_docs(&target), Limits { batch_size: batch, ..Limits::default() });
              let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
              let (_, summary, set) = rt.block_on(run(i)).unwrap();
              let (os, ot, d, eq) = brute_force(&source, &target);
              prop_assert_eq!(ids(&set, Category::OnlyInSource), os);
              prop_assert_eq!(ids(&set, Category::OnlyInTarget), ot);
              prop_assert_eq!(ids(&set, Category::Different), d);
              prop_assert_eq!(summary.equal, eq);
          }
      }

      #[test]
      fn fingerprints_in_results_match_the_documents() {
          let rules = CompareRules::for_key(&KeySpec::id());
          let d = doc! { "_id": 3, "v": 3 };
          assert!(fingerprint_of(&d, &rules).is_ok());
      }
  }
  ```

- [ ] **Step 3: Run and watch it fail.** Add `pub mod engine;` to `compare/mod.rs`.
  Run: `cargo test -p curlybrace-core --features test-support compare::engine`
  Expected: FAIL to compile.

- [ ] **Step 4: Implement `engine.rs`.** Above the test module:

  ```rust
  //! The sorted merge-join: two key-ordered streams are walked together, so memory use is bounded
  //! by the batch size plus the differences found, never by the collection size.

  use std::cmp::Ordering;
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

  use bson::Document;

  use super::fingerprint::fingerprint_of;
  use super::key::{KeySpec, compare_tuples};
  use super::results::{Budget, Category, DiffEntry, ResultSet};
  use super::rules::CompareRules;
  use super::scan::KeyScanner;
  use crate::error::Result;
  use crate::store::{DocumentStore, Namespace};

  pub struct PairInput {
      pub source: Arc<dyn DocumentStore>,
      pub source_ns: Namespace,
      pub source_filter: Document,
      pub target: Arc<dyn DocumentStore>,
      pub target_ns: Namespace,
      pub target_filter: Document,
      pub key: KeySpec,
      pub rules: CompareRules,
      pub budget: Budget,
  }

  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
  pub struct Summary {
      pub equal: u64,
      pub only_in_source: u64,
      pub only_in_target: u64,
      pub different: u64,
      pub scanned_source: u64,
      pub scanned_target: u64,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct Progress {
      pub scanned_source: u64,
      pub scanned_target: u64,
      pub differences: u64,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum EngineOutcome {
      Completed,
      Cancelled,
  }

  /// A cheap, cloneable cancellation signal.
  #[derive(Clone, Default)]
  pub struct CancelFlag(Arc<AtomicBool>);

  impl CancelFlag {
      pub fn cancel(&self) {
          self.0.store(true, AtomicOrdering::SeqCst);
      }

      pub fn is_cancelled(&self) -> bool {
          self.0.load(AtomicOrdering::SeqCst)
      }
  }

  /// Walks both streams to the end (or until cancelled) and returns the outcome, the counts and
  /// the differences. Errors (duplicate key, order, cap, store failures) abort the pair.
  pub async fn run_pair(
      input: PairInput,
      cancel: &CancelFlag,
      on_progress: &mut dyn FnMut(Progress),
  ) -> Result<(EngineOutcome, Summary, ResultSet)> {
      let batch = input.budget.limits().batch_size;
      let mut source = KeyScanner::new(input.source, input.source_ns, input.source_filter, input.key.clone(), batch);
      let mut target = KeyScanner::new(input.target, input.target_ns, input.target_filter, input.key, batch);
      let mut set = ResultSet::new(input.budget);
      let mut summary = Summary::default();
      let mut steps = 0u64;
      loop {
          if cancel.is_cancelled() {
              finish(&mut summary, &source, &target);
              return Ok((EngineOutcome::Cancelled, summary, set));
          }
          let order = match (source.peek().await?, target.peek().await?) {
              (None, None) => break,
              (Some(_), None) => Ordering::Less,
              (None, Some(_)) => Ordering::Greater,
              (Some(s), Some(t)) => compare_tuples(&s.0, &t.0),
          };
          match order {
              Ordering::Less => {
                  let (key, doc) = source.next().await?.expect("peeked");
                  let source_fp = Some(fingerprint_of(&doc, &input.rules)?);
                  set.push(Category::OnlyInSource, DiffEntry { key, source: source_fp, target: None })?;
                  summary.only_in_source += 1;
              }
              Ordering::Greater => {
                  let (key, doc) = target.next().await?.expect("peeked");
                  let target_fp = Some(fingerprint_of(&doc, &input.rules)?);
                  set.push(Category::OnlyInTarget, DiffEntry { key, source: None, target: target_fp })?;
                  summary.only_in_target += 1;
              }
              Ordering::Equal => {
                  let (key, s_doc) = source.next().await?.expect("peeked");
                  let (_, t_doc) = target.next().await?.expect("peeked");
                  let (sf, tf) = (fingerprint_of(&s_doc, &input.rules)?, fingerprint_of(&t_doc, &input.rules)?);
                  if sf == tf {
                      summary.equal += 1;
                  } else {
                      set.push(Category::Different, DiffEntry { key, source: Some(sf), target: Some(tf) })?;
                      summary.different += 1;
                  }
              }
          }
          steps += 1;
          if steps % u64::from(batch) == 0 {
              finish(&mut summary, &source, &target);
              on_progress(Progress {
                  scanned_source: summary.scanned_source,
                  scanned_target: summary.scanned_target,
                  differences: summary.only_in_source + summary.only_in_target + summary.different,
              });
          }
      }
      finish(&mut summary, &source, &target);
      Ok((EngineOutcome::Completed, summary, set))
  }

  fn finish(summary: &mut Summary, source: &KeyScanner, target: &KeyScanner) {
      summary.scanned_source = source.scanned();
      summary.scanned_target = target.scanned();
  }
  ```

  `input.rules` is used after `input.source` etc. were moved out field-by-field; that is allowed for a struct with no `Drop`. If the borrow checker objects, destructure `PairInput { source, ... } = input;` at the top instead.

- [ ] **Step 5: Run the tests.**
  Run: `cargo test -p curlybrace-core --features test-support compare::`
  Expected: PASS, including the property test (64 cases). If the property test finds a counter-example, the output shrinks it; fix the engine, never loosen the property.

- [ ] **Step 6: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates Cargo.lock && git commit -m "feat(compare): add the sorted merge-join engine

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 7: Protocol and compare sessions in the runtime (open, list, check, close)

**Files:**
- Create: `crates/core/src/compare/protocol.rs`, `crates/core/src/runtime/compare/mod.rs`, `crates/core/src/runtime/compare/session.rs`, `crates/core/src/runtime/compare/tests.rs`
- Modify: `crates/core/src/compare/mod.rs`, `crates/core/src/runtime/mod.rs`, `crates/core/src/runtime/tunneling.rs`, `crates/core/src/runtime/handlers.rs`, `crates/core/src/runtime/prompt_gate_tests.rs` (only if the gate constructor is used there)

**Interfaces:**
- Consumes: `Connector`, `TunnelOpener`, `ConnectionConfig`, `Budget`, `Limits`, `CollectionKind`, `IndexStatus`.
- Produces:
  ```rust
  // compare/protocol.rs  (all Clone, Debug; serde not needed)
  pub type SessionId = u64;  pub type PairId = u32;  pub type RunId = u64;
  pub enum Side { Source, Target }                                         // Copy, PartialEq, Eq, Hash
  pub struct PairSpec { pub id: PairId, pub source: Namespace, pub target: Namespace,
                        pub source_filter: Document, pub target_filter: Document,
                        pub key: KeySpec, pub rules: CompareRules }
  pub struct Preflight { pub source_count: u64, pub target_count: u64,
                         pub source_index: IndexStatus, pub target_index: IndexStatus,
                         pub unindexed_too_large: bool }
  pub enum CompareCommand {
      Open { session: SessionId, source: ConnectionConfig, target: ConnectionConfig },
      Databases { session: SessionId, side: Side },
      Collections { session: SessionId, side: Side, db: String },
      Check { session: SessionId, pair: PairSpec },
      Start { session: SessionId, run: RunId, pairs: Vec<PairSpec> },       // Task 8
      Cancel { session: SessionId, run: RunId },                            // Task 8
      Rows { session: SessionId, pair: PairId, category: Category, offset: usize, limit: usize },   // Task 8
      Detail { session: SessionId, pair: PairId, key: KeyTuple },           // Task 8
      Discard { session: SessionId, pair: PairId },                         // Task 8
      Close { session: SessionId },
  }
  pub enum CompareEvent {
      Opened { session }, OpenFailed { session, error: CoreError },
      Databases { session, side, names: Vec<String> },
      Collections { session, side, db: String, entries: Vec<(String, CollectionKind)> },
      Checked { session, pair: PairId, preflight: Preflight },
      Failed { session, context: &'static str, error: CoreError },          // command-level failure
      // Task 8: Progress, PairDone, RunDone, Rows, Detail
  }
  ```
  `Command::Compare(CompareCommand)` and `Event::Compare(CompareEvent)` are added to the runtime enums. `Command::context()` returns `"compare"` for it. **Answer rule for compare commands:** `Open` → `Opened` or `OpenFailed`; `Databases`/`Collections`/`Check`/`Rows`/`Detail` → their answer or `Failed{context}`; `Start` → pair events then exactly one `RunDone` (or `Failed` when refused); `Cancel`, `Discard` and `Close` have no answer of their own (their effect is observed through the events above). Session ids are chosen by the UI and must not be reused.

- [ ] **Step 1: Write the failing runtime tests (offline).** Create `crates/core/src/runtime/compare/tests.rs` (declared with `#[cfg(test)] mod tests;` in `runtime/compare/mod.rs`):

  ```rust
  //! Compare sessions against in-memory stores: no server, no network.

  use std::collections::HashMap;
  use std::sync::Arc;
  use std::time::Duration;

  use bson::{Document, doc};

  use crate::compare::key::KeySpec;
  use crate::compare::protocol::{CompareCommand, CompareEvent, PairSpec, Side};
  use crate::compare::rules::CompareRules;
  use crate::connection::ConnectionConfig;
  use crate::error::{CoreError, Result};
  use crate::runtime::{Command, Connector, Event, RuntimeConfig, RuntimeHandle, spawn_runtime_with_connector};
  use crate::store::{CollectionKind, DocumentStore, IndexStatus, Namespace};
  use crate::test_support::InMemoryStore;

  /// Hands out the in-memory store registered for the connection's URI.
  pub(super) struct MapConnector(pub HashMap<String, Arc<dyn DocumentStore>>);

  #[async_trait::async_trait]
  impl Connector for MapConnector {
      async fn connect(
          &self,
          cfg: &ConnectionConfig,
          _password: Option<&str>,
          _tunnel: Option<std::net::SocketAddr>,
      ) -> Result<Arc<dyn DocumentStore>> {
          self.0.get(&cfg.uri).cloned().ok_or(CoreError::Database("no such server".into()))
      }
  }

  pub(super) fn cfg(uri: &str) -> ConnectionConfig {
      ConnectionConfig::new(uri, uri)
  }

  pub(super) fn ns(db: &str, c: &str) -> Namespace {
      Namespace::new(db, c)
  }

  pub(super) fn pair(id: u32, source: Namespace, target: Namespace) -> PairSpec {
      PairSpec {
          id,
          source,
          target,
          source_filter: doc! {},
          target_filter: doc! {},
          key: KeySpec::id(),
          rules: CompareRules::for_key(&KeySpec::id()),
      }
  }

  pub(super) fn start(
      servers: Vec<(&str, InMemoryStore)>,
  ) -> (tokio::sync::mpsc::UnboundedSender<Command>, RuntimeHandle) {
      let map = servers
          .into_iter()
          .map(|(uri, store)| (uri.to_string(), Arc::new(store) as Arc<dyn DocumentStore>))
          .collect();
      let handle = spawn_runtime_with_connector(
          None,
          None,
          Arc::new(MapConnector(map)),
          Arc::new(|| {}),
          RuntimeConfig::default(),
      );
      (handle.tx.clone(), handle)
  }

  pub(super) async fn next_compare(h: &mut RuntimeHandle) -> CompareEvent {
      loop {
          let event = tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
              .await
              .expect("an event arrives")
              .expect("the channel is open");
          if let Event::Compare(e) = event {
              return e;
          }
      }
  }

  pub(super) fn seeded() -> Vec<(&'static str, InMemoryStore)> {
      let a = InMemoryStore::new().with_docs(&ns("d", "c"), (0..5).map(|n| doc! { "_id": n }).collect());
      let b = InMemoryStore::new().with_docs(&ns("d", "c"), (2..7).map(|n| doc! { "_id": n }).collect());
      vec![("mongodb://a", a), ("mongodb://b", b)]
  }

  pub(super) async fn open(h: &mut RuntimeHandle, tx: &tokio::sync::mpsc::UnboundedSender<Command>, session: u64) {
      tx.send(Command::Compare(CompareCommand::Open { session, source: cfg("mongodb://a"), target: cfg("mongodb://b") })).unwrap();
      assert_eq!(next_compare(h).await, CompareEvent::Opened { session });
  }

  #[tokio::test]
  async fn open_connects_both_sides_and_answers_once() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
  }

  #[tokio::test]
  async fn open_with_a_failing_side_reports_open_failed_and_leaves_no_session() {
      let (tx, mut h) = start(seeded());
      tx.send(Command::Compare(CompareCommand::Open { session: 1, source: cfg("mongodb://a"), target: cfg("mongodb://missing") })).unwrap();
      assert!(matches!(next_compare(&mut h).await, CompareEvent::OpenFailed { session: 1, .. }));
      tx.send(Command::Compare(CompareCommand::Databases { session: 1, side: Side::Source })).unwrap();
      assert!(matches!(next_compare(&mut h).await, CompareEvent::Failed { session: 1, context: "compare_databases", .. }));
  }

  #[tokio::test]
  async fn listing_uses_the_chosen_side() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Collections { session: 1, side: Side::Target, db: "d".into() })).unwrap();
      match next_compare(&mut h).await {
          CompareEvent::Collections { session: 1, side: Side::Target, db, entries } => {
              assert_eq!(db, "d");
              assert_eq!(entries, vec![("c".to_string(), CollectionKind::Collection)]);
          }
          other => panic!("{other:?}"),
      }
  }

  #[tokio::test]
  async fn check_reports_counts_and_index_status() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Check { session: 1, pair: pair(7, ns("d", "c"), ns("d", "c")) })).unwrap();
      match next_compare(&mut h).await {
          CompareEvent::Checked { session: 1, pair: 7, preflight } => {
              assert_eq!((preflight.source_count, preflight.target_count), (5, 5));
              assert_eq!(preflight.source_index, IndexStatus::Indexed);
              assert!(!preflight.unindexed_too_large);
          }
          other => panic!("{other:?}"),
      }
  }

  #[tokio::test]
  async fn commands_for_an_unknown_or_closed_session_fail_with_their_context() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Close { session: 1 })).unwrap();
      tx.send(Command::Compare(CompareCommand::Check { session: 1, pair: pair(1, ns("d", "c"), ns("d", "c")) })).unwrap();
      assert!(matches!(next_compare(&mut h).await, CompareEvent::Failed { session: 1, context: "compare_check", .. }));
      let _: Document = doc! {};
  }

  #[tokio::test]
  async fn the_main_connection_is_untouched_by_compare_sessions() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      tx.send(Command::ListDatabases).unwrap();
      let event = tokio::time::timeout(Duration::from_secs(5), h.rx.recv()).await.unwrap().unwrap();
      assert!(matches!(event, Event::Failed { context: "list_databases", .. }), "not connected is still not connected: {event:?}");
  }
  ```

- [ ] **Step 2: Run and watch it fail.** Wire modules first so the failure is about missing items: add `pub mod protocol;` to `compare/mod.rs`, `mod compare;` to `runtime/mod.rs`.
  Run: `cargo test -p curlybrace-core --features test-support runtime::compare`
  Expected: FAIL to compile.

- [ ] **Step 3: Write `compare/protocol.rs`.** This task defines the Task 7 subset; Task 8 extends it.

  ```rust
  //! The messages of the compare feature, wrapped by `runtime::Command::Compare` /
  //! `runtime::Event::Compare`. Pure data: no behaviour.

  use bson::Document;

  use super::engine::Summary;
  use super::key::{KeySpec, KeyTuple};
  use super::results::Category;
  use super::rules::CompareRules;
  use crate::connection::ConnectionConfig;
  use crate::error::CoreError;
  use crate::store::{CollectionKind, IndexStatus, Namespace};

  pub type SessionId = u64;
  pub type PairId = u32;
  pub type RunId = u64;

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum Side {
      Source,
      Target,
  }

  #[derive(Clone, Debug)]
  pub struct PairSpec {
      pub id: PairId,
      pub source: Namespace,
      pub target: Namespace,
      pub source_filter: Document,
      pub target_filter: Document,
      pub key: KeySpec,
      pub rules: CompareRules,
  }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Preflight {
      pub source_count: u64,
      pub target_count: u64,
      pub source_index: IndexStatus,
      pub target_index: IndexStatus,
      /// A side has no usable index and is above `max_unindexed_documents`.
      pub unindexed_too_large: bool,
  }

  #[derive(Clone, Debug)]
  pub enum CompareCommand {
      Open { session: SessionId, source: ConnectionConfig, target: ConnectionConfig },
      Databases { session: SessionId, side: Side },
      Collections { session: SessionId, side: Side, db: String },
      Check { session: SessionId, pair: PairSpec },
      Start { session: SessionId, run: RunId, pairs: Vec<PairSpec> },
      Cancel { session: SessionId, run: RunId },
      Rows { session: SessionId, pair: PairId, category: Category, offset: usize, limit: usize },
      Detail { session: SessionId, pair: PairId, key: KeyTuple },
      Discard { session: SessionId, pair: PairId },
      Close { session: SessionId },
  }

  /// How one pair ended. A run reports exactly one of these per pair.
  #[derive(Clone, Debug, PartialEq)]
  pub enum PairOutcome {
      Completed(Summary),
      Cancelled(Summary),
      Failed(CoreError),
  }

  #[derive(Clone, Debug, PartialEq)]
  pub struct RowView {
      pub key: KeyTuple,
      pub has_source: bool,
      pub has_target: bool,
  }

  #[derive(Clone, Debug, PartialEq)]
  pub enum CompareEvent {
      Opened { session: SessionId },
      OpenFailed { session: SessionId, error: CoreError },
      Databases { session: SessionId, side: Side, names: Vec<String> },
      Collections { session: SessionId, side: Side, db: String, entries: Vec<(String, CollectionKind)> },
      Checked { session: SessionId, pair: PairId, preflight: Preflight },
      Progress { session: SessionId, run: RunId, pair: PairId, scanned_source: u64, scanned_target: u64, differences: u64 },
      PairDone { session: SessionId, run: RunId, pair: PairId, outcome: PairOutcome },
      RunDone { session: SessionId, run: RunId },
      Rows { session: SessionId, pair: PairId, category: Category, offset: usize, total: usize, rows: Vec<RowView> },
      Detail { session: SessionId, pair: PairId, key: KeyTuple, source: Option<Document>, target: Option<Document>, changes: Vec<crate::compare::diff::FieldChange> },
      /// A command failed as a whole. `context` is an i18n suffix such as `"compare_check"`.
      Failed { session: SessionId, context: &'static str, error: CoreError },
  }
  ```
  `Summary` needs `PartialEq` (it has it); `FieldChange` has `PartialEq`; `Document` has it.

- [ ] **Step 4: Generalize the host-key prompt gate.** In `crates/core/src/runtime/tunneling.rs` replace `GatedPrompt`'s `shared`/`generation` with a currentness check, so compare sessions can reuse it:

  ```rust
  /// Whether the job a question belongs to is still wanted.
  pub(crate) trait Currentness: Send + Sync {
      fn is_current(&self) -> bool;
  }

  /// The main connection: current until a newer Connect/Disconnect replaced it.
  pub(super) struct ConnectionCurrent {
      pub shared: Arc<Shared>,
      pub generation: u64,
  }

  impl Currentness for ConnectionCurrent {
      fn is_current(&self) -> bool {
          self.shared.is_current(self.generation)
      }
  }

  struct GatedPrompt {
      inner: Arc<dyn HostKeyPrompt>,
      job: Arc<dyn Currentness>,
  }

  impl HostKeyPrompt for GatedPrompt {
      fn decide(&self, host: &str, fingerprint: &str) -> HostKeyDecision {
          if !self.job.is_current() {
              return HostKeyDecision::Reject;
          }
          let decision = self.inner.decide(host, fingerprint);
          if self.job.is_current() { decision } else { HostKeyDecision::Reject }
      }
  }
  ```
  Change `open_tunnel_if_configured(shared, generation, cfg)` to `open_tunnel_if_configured(shared: &Arc<Shared>, job: Arc<dyn Currentness>, cfg: &ConnectionConfig)`; it builds `GatedPrompt { inner: config.host_key_prompt.clone(), job }`. In `handlers.rs::connect_store` pass `Arc::new(ConnectionCurrent { shared: shared.clone(), generation })`. Update any test in `prompt_gate_tests.rs` / `tunnel_tests.rs` that builds `GatedPrompt` or calls `open_tunnel_if_configured` directly. Run `cargo test -p curlybrace-core runtime::` now: the existing runtime tests must stay green before continuing (this is a pure refactor; no behaviour changes).

- [ ] **Step 5: Implement the session registry.** Create `crates/core/src/runtime/compare/session.rs`:

  ```rust
  //! One compare session: two live connections (each with its own SSH tunnel when configured),
  //! the shared result budget, and the results of finished pairs.

  use std::collections::HashMap;
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::{Arc, Mutex};

  use tokio::sync::OnceCell;

  use crate::compare::engine::CancelFlag;
  use crate::compare::protocol::{PairId, RunId, Side};
  use crate::compare::results::{Budget, Limits, ResultSet};
  use crate::runtime::TunnelHandle;
  use crate::runtime::tunneling::Currentness;
  use crate::store::DocumentStore;

  pub(super) struct Connections {
      pub source: Arc<dyn DocumentStore>,
      pub target: Arc<dyn DocumentStore>,
      /// Kept alive for the session; dropping them closes the tunnels.
      pub _tunnels: Vec<Box<dyn TunnelHandle>>,
  }

  impl Connections {
      pub fn store(&self, side: Side) -> Arc<dyn DocumentStore> {
          match side {
              Side::Source => self.source.clone(),
              Side::Target => self.target.clone(),
          }
      }
  }

  pub(super) struct ActiveRun {
      pub id: RunId,
      pub cancel: CancelFlag,
  }

  /// A finished pair's results plus what is needed to look a row up again.
  pub(super) struct StoredPair {
      pub spec: crate::compare::protocol::PairSpec,
      pub results: ResultSet,
  }

  pub(crate) struct Session {
      /// False once the session was closed; also answers "is this job still wanted".
      pub open: AtomicBool,
      pub connections: OnceCell<Connections>,
      pub budget: Budget,
      pub(super) pairs: Mutex<HashMap<PairId, StoredPair>>,
      pub(super) run: Mutex<Option<ActiveRun>>,
  }

  impl Session {
      pub fn new(limits: Limits) -> Arc<Self> {
          Arc::new(Self {
              open: AtomicBool::new(true),
              connections: OnceCell::new(),
              budget: Budget::new(limits),
              pairs: Mutex::new(HashMap::new()),
              run: Mutex::new(None),
          })
      }

      pub fn is_open(&self) -> bool {
          self.open.load(Ordering::SeqCst)
      }

      /// Marks the session closed, cancels any run and drops every stored result.
      pub fn close(&self) {
          self.open.store(false, Ordering::SeqCst);
          if let Some(run) = self.run.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
              run.cancel.cancel();
          }
          self.pairs.lock().unwrap_or_else(|e| e.into_inner()).clear();
      }
  }

  impl Currentness for Session {
      fn is_current(&self) -> bool {
          self.is_open()
      }
  }

  /// The sessions of one runtime.
  #[derive(Default)]
  pub(crate) struct Registry {
      sessions: Mutex<HashMap<u64, Arc<Session>>>,
  }

  impl Registry {
      /// Creates the session record synchronously (so a `Close` that follows is ordered after it).
      pub fn create(&self, id: u64, limits: Limits) -> Arc<Session> {
          let session = Session::new(limits);
          let replaced = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(id, session.clone());
          if let Some(old) = replaced {
              old.close();
          }
          session
      }

      pub fn get(&self, id: u64) -> Option<Arc<Session>> {
          self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
      }

      pub fn remove(&self, id: u64) {
          let removed = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
          if let Some(session) = removed {
              session.close();
          }
      }
  }
  ```
  `Session::connections` is an async `OnceCell` filled by the open task; commands that arrive before it is set are answered with `Failed` ("not open yet") because the UI waits for `Opened` before issuing them.

- [ ] **Step 6: Implement the handlers.** Create `crates/core/src/runtime/compare/mod.rs`:

  ```rust
  //! Compare sessions in the runtime. They are independent of the main browsing connection: their
  //! own connections, their own lifetime, never touching `Shared::store`, the connection epoch or
  //! the "exactly one event per command" bookkeeping of the data commands.
  //!
  //! Answers: `Open` → `Opened` | `OpenFailed`; `Databases`/`Collections`/`Check`/`Rows`/`Detail`
  //! → their answer | `Failed { context }`; `Start` → one `PairDone` per pair, then one `RunDone`
  //! (or `Failed` when refused); `Cancel`, `Discard`, `Close` have no answer of their own.

  mod runner;
  mod session;
  #[cfg(test)]
  mod tests;

  use std::sync::Arc;

  use tokio::sync::mpsc::UnboundedSender;

  use crate::compare::protocol::{CompareCommand, CompareEvent, PairSpec, Preflight, Side, SessionId};
  use crate::compare::results::Limits;
  use crate::connection::ConnectionConfig;
  use crate::error::{CoreError, Result};
  use crate::store::{DocumentStore, IndexStatus};

  use super::tunneling::{Currentness, open_tunnel_if_configured};
  use super::{Event, Repaint, Shared, TunnelHandle};
  pub(super) use session::Registry;
  use session::{Connections, Session};

  pub(super) type Events = UnboundedSender<Event>;

  /// Sends a compare event and repaints. Returns false once the receiver is gone.
  pub(super) fn emit(events: &Events, repaint: &Repaint, event: CompareEvent) -> bool {
      let sent = events.send(Event::Compare(event)).is_ok();
      if sent {
          repaint();
      }
      sent
  }

  /// Entry point from `runtime::dispatch`. Session bookkeeping happens synchronously here so that
  /// command ORDER decides outcomes (an `Open` followed by a `Close` always closes); the slow work
  /// runs in a spawned task.
  pub(super) fn dispatch(shared: Arc<Shared>, events: Events, repaint: Repaint, command: CompareCommand) {
      match command {
          CompareCommand::Open { session, source, target } => {
              let record = shared.compare.create(session, Limits::default());
              tokio::spawn(open_session(shared, events, repaint, session, record, [source, target]));
          }
          CompareCommand::Close { session } => shared.compare.remove(session),
          CompareCommand::Cancel { session, run } => runner::cancel(&shared, session, run),
          CompareCommand::Discard { session, pair } => runner::discard(&shared, session, pair),
          other => {
              tokio::spawn(async move {
                  let (session, context) = (command_session(&other), command_context(&other));
                  if let Err(error) = handle(&shared, &events, &repaint, other).await {
                      emit(&events, &repaint, CompareEvent::Failed { session, context, error });
                  }
              });
          }
      }
  }

  fn command_session(c: &CompareCommand) -> SessionId {
      match c {
          CompareCommand::Open { session, .. }
          | CompareCommand::Databases { session, .. }
          | CompareCommand::Collections { session, .. }
          | CompareCommand::Check { session, .. }
          | CompareCommand::Start { session, .. }
          | CompareCommand::Cancel { session, .. }
          | CompareCommand::Rows { session, .. }
          | CompareCommand::Detail { session, .. }
          | CompareCommand::Discard { session, .. }
          | CompareCommand::Close { session } => *session,
      }
  }

  fn command_context(c: &CompareCommand) -> &'static str {
      match c {
          CompareCommand::Databases { .. } => "compare_databases",
          CompareCommand::Collections { .. } => "compare_collections",
          CompareCommand::Check { .. } => "compare_check",
          CompareCommand::Start { .. } => "compare_start",
          CompareCommand::Rows { .. } => "compare_rows",
          CompareCommand::Detail { .. } => "compare_detail",
          _ => "compare",
      }
  }

  async fn handle(shared: &Arc<Shared>, events: &Events, repaint: &Repaint, command: CompareCommand) -> Result<()> {
      let session_id = command_session(&command);
      let session = shared
          .compare
          .get(session_id)
          .filter(|s| s.is_open())
          .ok_or_else(|| CoreError::Unsupported("the comparison is closed".into()))?;
      let limit = shared.config.command_timeout;
      match command {
          CompareCommand::Databases { side, .. } => {
              let store = connections(&session)?.store(side);
              let names = bounded(limit, async { store.list_databases().await }).await?;
              emit(events, repaint, CompareEvent::Databases { session: session_id, side, names });
          }
          CompareCommand::Collections { side, db, .. } => {
              let store = connections(&session)?.store(side);
              let entries = bounded(limit, async { store.list_collection_kinds(&db).await }).await?;
              emit(events, repaint, CompareEvent::Collections { session: session_id, side, db, entries });
          }
          CompareCommand::Check { pair, .. } => {
              let preflight = bounded(limit, check(&session, &pair)).await?;
              emit(events, repaint, CompareEvent::Checked { session: session_id, pair: pair.id, preflight });
          }
          CompareCommand::Start { run, pairs, .. } => {
              runner::start(shared.clone(), events.clone(), repaint.clone(), session, session_id, run, pairs)?;
          }
          CompareCommand::Rows { pair, category, offset, limit: rows, .. } => {
              runner::rows(&session, events, repaint, session_id, pair, category, offset, rows)?;
          }
          CompareCommand::Detail { pair, key, .. } => {
              let event = bounded(limit, runner::detail(&session, session_id, pair, key)).await?;
              emit(events, repaint, event);
          }
          CompareCommand::Open { .. } | CompareCommand::Cancel { .. } | CompareCommand::Discard { .. } | CompareCommand::Close { .. } => {}
      }
      Ok(())
  }

  async fn bounded<T>(limit: std::time::Duration, work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
      tokio::time::timeout(limit, work)
          .await
          .map_err(|_| CoreError::Database("timeout".into()))?
  }

  pub(super) fn connections(session: &Session) -> Result<&Connections> {
      session
          .connections
          .get()
          .ok_or_else(|| CoreError::Unsupported("the comparison is still opening".into()))
  }

  async fn check(session: &Session, pair: &PairSpec) -> Result<Preflight> {
      let conns = connections(session)?;
      let (source_count, target_count) = tokio::try_join!(
          conns.source.estimated_count(&pair.source),
          conns.target.estimated_count(&pair.target)
      )?;
      let (source_index, target_index) = tokio::try_join!(
          conns.source.key_index_status(&pair.source, &pair.key),
          conns.target.key_index_status(&pair.target, &pair.key)
      )?;
      let cap = session.budget.limits().max_unindexed_documents;
      let too_large = |index: IndexStatus, count: u64| index == IndexStatus::NotIndexed && count > cap;
      Ok(Preflight {
          source_count,
          target_count,
          source_index,
          target_index,
          unindexed_too_large: too_large(source_index, source_count) || too_large(target_index, target_count),
      })
  }

  async fn open_session(
      shared: Arc<Shared>,
      events: Events,
      repaint: Repaint,
      id: SessionId,
      session: Arc<Session>,
      configs: [ConnectionConfig; 2],
  ) {
      let result = tokio::time::timeout(shared.config.connect_timeout, async {
          let [source, target] = configs;
          let (store_s, tunnel_s) = open_side(&shared, &session, &source).await?;
          let (store_t, tunnel_t) = open_side(&shared, &session, &target).await?;
          let tunnels: Vec<Box<dyn TunnelHandle>> = [tunnel_s, tunnel_t].into_iter().flatten().collect();
          Ok::<_, CoreError>(Connections { source: store_s, target: store_t, _tunnels: tunnels })
      })
      .await
      .unwrap_or_else(|_| Err(CoreError::Database("timeout".into())));
      if !session.is_open() {
          // Closed (or replaced) while connecting: whatever was opened is dropped here.
          return;
      }
      match result {
          Ok(connections) if session.connections.set(connections).is_ok() => {
              emit(&events, &repaint, CompareEvent::Opened { session: id });
          }
          Ok(_) => {}
          Err(error) => {
              shared.compare.remove(id);
              emit(&events, &repaint, CompareEvent::OpenFailed { session: id, error });
          }
      }
  }

  async fn open_side(
      shared: &Arc<Shared>,
      session: &Arc<Session>,
      cfg: &ConnectionConfig,
  ) -> Result<(Arc<dyn DocumentStore>, Option<Box<dyn TunnelHandle>>)> {
      cfg.validate()?;
      let password = super::handlers::read_password(shared, cfg).await?;
      let job: Arc<dyn Currentness> = session.clone();
      let tunnel = open_tunnel_if_configured(shared, job, cfg).await?;
      let addr = tunnel.as_ref().map(|t| t.local_addr());
      let store = shared.connector.connect(cfg, password.as_deref(), addr).await?;
      Ok((store, tunnel))
  }

  #[allow(dead_code)]
  fn _types(_: Side) {}
  ```
  Remove the `_types` helper and any unused import the compiler flags. In `runtime/handlers.rs` change `async fn read_password` to `pub(super) async fn read_password`. In `runtime/mod.rs`: add `use` of `compare::Registry`; add `compare: compare::Registry` to `Shared` (initialised with `Default::default()` in `spawn_runtime_with_connector`); add `Compare(crate::compare::protocol::CompareCommand)` to `Command` and `Compare(crate::compare::protocol::CompareEvent)` to `Event`; `context()` returns `"compare"` for it; and at the TOP of `dispatch` add:

  ```rust
      if let Command::Compare(c) = command {
          compare::dispatch(shared, events, repaint, c);
          return;
      }
  ```
  `Event` derives `PartialEq`; `CompareEvent` does (Task 7 protocol) so it compiles.

- [ ] **Step 7: Stub the runner so the module compiles, then run the tests.** Create `crates/core/src/runtime/compare/runner.rs` with the Task 7 surface only (Task 8 fills it):

  ```rust
  //! Runs pairs and serves their results (completed in Task 8).

  use std::sync::Arc;

  use crate::compare::protocol::{PairId, PairSpec, RunId, SessionId};
  use crate::compare::results::Category;
  use crate::compare::key::KeyTuple;
  use crate::error::{CoreError, Result};

  use super::session::Session;
  use super::{Events, Repaint, Shared};

  pub(super) fn cancel(_: &Shared, _: SessionId, _: RunId) {}
  pub(super) fn discard(_: &Shared, _: SessionId, _: PairId) {}

  pub(super) fn start(
      _: Arc<Shared>, _: Events, _: Repaint, _: Arc<Session>, _: SessionId, _: RunId, _: Vec<PairSpec>,
  ) -> Result<()> {
      Err(CoreError::Unsupported("not implemented yet".into()))
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) fn rows(
      _: &Session, _: &Events, _: &Repaint, _: SessionId, _: PairId, _: Category, _: usize, _: usize,
  ) -> Result<()> {
      Err(CoreError::Unsupported("not implemented yet".into()))
  }

  pub(super) async fn detail(
      _: &Session, _: SessionId, _: PairId, _: KeyTuple,
  ) -> Result<crate::compare::protocol::CompareEvent> {
      Err(CoreError::Unsupported("not implemented yet".into()))
  }
  ```
  Run: `cargo test -p curlybrace-core --features test-support runtime::`
  Expected: PASS, the new compare tests and every existing runtime test (the gate refactor must not have changed behaviour).

- [ ] **Step 8: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add compare sessions to the runtime protocol

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 8: Running pairs: start, cancel, rows, detail and discard

**Files:**
- Modify: `crates/core/src/runtime/compare/runner.rs`, `crates/core/src/runtime/compare/tests.rs`
- Create: `crates/core/tests/local_compare_session.rs`

**Interfaces:**
- Consumes: Task 7's `Session`, `emit`, `run_pair`, `CancelFlag`, `StoredPair`.
- Produces: the behaviour of `Start`, `Cancel`, `Rows`, `Detail`, `Discard` as documented in Task 7's answer rule. **Invariants:** per run, each pair id gets exactly one `PairDone`; `RunDone` is sent after the last `PairDone`; a second `Start` while a run is active is refused (`Failed{context:"compare_start"}`); a closed session emits nothing further; results of a failed or cancelled pair are never stored.

- [ ] **Step 1: Write the failing tests.** Append to `crates/core/src/runtime/compare/tests.rs`:

  ```rust
  use crate::compare::protocol::PairOutcome;
  use crate::compare::results::Category;

  async fn collect_run(h: &mut RuntimeHandle, pairs: usize) -> (Vec<(u32, PairOutcome)>, bool) {
      let mut done = Vec::new();
      loop {
          match next_compare(h).await {
              CompareEvent::PairDone { pair, outcome, .. } => done.push((pair, outcome)),
              CompareEvent::RunDone { .. } => return (done, true),
              CompareEvent::Progress { .. } => {}
              other => panic!("unexpected {other:?} after {} pair events of {pairs}", done.len()),
          }
      }
  }

  #[tokio::test]
  async fn a_run_reports_one_terminal_per_pair_then_the_run_terminal() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      let pairs = vec![pair(1, ns("d", "c"), ns("d", "c")), pair(2, ns("d", "c"), ns("d", "c"))];
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 10, pairs })).unwrap();
      let (done, finished) = collect_run(&mut h, 2).await;
      assert!(finished);
      assert_eq!(done.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2]);
      for (_, outcome) in &done {
          match outcome {
              PairOutcome::Completed(s) => assert_eq!((s.equal, s.only_in_source, s.only_in_target), (3, 2, 2)),
              other => panic!("{other:?}"),
          }
      }
  }

  #[tokio::test]
  async fn one_failing_pair_does_not_stop_the_others() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      // The second pair keys on a field no document has.
      let mut bad = pair(2, ns("d", "c"), ns("d", "c"));
      bad.key = KeySpec::new(vec!["nope".into()]).unwrap();
      bad.rules = CompareRules::for_key(&bad.key);
      let pairs = vec![pair(1, ns("d", "c"), ns("d", "c")), bad, pair(3, ns("d", "c"), ns("d", "c"))];
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 10, pairs })).unwrap();
      let (done, _) = collect_run(&mut h, 3).await;
      assert!(matches!(done[0].1, PairOutcome::Completed(_)));
      assert!(matches!(done[1].1, PairOutcome::Failed(CoreError::CompareKeyMissing)));
      assert!(matches!(done[2].1, PairOutcome::Completed(_)));
  }

  #[tokio::test]
  async fn rows_page_the_differences_and_detail_shows_the_field_changes() {
      let a = InMemoryStore::new().with_docs(&ns("d", "c"), vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 2 }]);
      let b = InMemoryStore::new().with_docs(&ns("d", "c"), vec![doc! { "_id": 1, "v": 9 }, doc! { "_id": 3, "v": 3 }]);
      let (tx, mut h) = start(vec![("mongodb://a", a), ("mongodb://b", b)]);
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 1, pairs: vec![pair(1, ns("d", "c"), ns("d", "c"))] })).unwrap();
      collect_run(&mut h, 1).await;
      tx.send(Command::Compare(CompareCommand::Rows { session: 1, pair: 1, category: Category::Different, offset: 0, limit: 10 })).unwrap();
      let key = match next_compare(&mut h).await {
          CompareEvent::Rows { total: 1, rows, .. } => {
              assert!(rows[0].has_source && rows[0].has_target);
              rows[0].key.clone()
          }
          other => panic!("{other:?}"),
      };
      tx.send(Command::Compare(CompareCommand::Detail { session: 1, pair: 1, key })).unwrap();
      match next_compare(&mut h).await {
          CompareEvent::Detail { changes, source, target, .. } => {
              assert_eq!(changes.len(), 1);
              assert_eq!(changes[0].path, "v");
              assert!(source.is_some() && target.is_some());
          }
          other => panic!("{other:?}"),
      }
  }

  #[tokio::test]
  async fn discard_returns_the_budget_and_makes_rows_fail() {
      let (tx, mut h) = start(seeded());
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 1, pairs: vec![pair(1, ns("d", "c"), ns("d", "c"))] })).unwrap();
      collect_run(&mut h, 1).await;
      tx.send(Command::Compare(CompareCommand::Discard { session: 1, pair: 1 })).unwrap();
      tx.send(Command::Compare(CompareCommand::Rows { session: 1, pair: 1, category: Category::Different, offset: 0, limit: 5 })).unwrap();
      assert!(matches!(next_compare(&mut h).await, CompareEvent::Failed { context: "compare_rows", .. }));
  }

  #[tokio::test]
  async fn a_second_start_while_running_is_refused() {
      let docs = (0..3000).map(|n| doc! { "_id": n }).collect();
      let a = InMemoryStore::new().with_docs(&ns("d", "c"), docs);
      let b = InMemoryStore::new();
      let (tx, mut h) = start(vec![("mongodb://a", a), ("mongodb://b", b)]);
      open(&mut h, &tx, 1).await;
      let p = || vec![pair(1, ns("d", "c"), ns("d", "c"))];
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 1, pairs: p() })).unwrap();
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 2, pairs: p() })).unwrap();
      let mut refused = false;
      let mut run_done = 0;
      while run_done == 0 {
          match next_compare(&mut h).await {
              CompareEvent::Failed { context: "compare_start", .. } => refused = true,
              CompareEvent::RunDone { run: 1, .. } => run_done += 1,
              _ => {}
          }
      }
      assert!(refused);
  }

  #[tokio::test]
  async fn cancel_ends_every_remaining_pair_as_cancelled_and_the_run_once() {
      let docs = || (0..20_000).map(|n| doc! { "_id": n }).collect::<Vec<_>>();
      let a = InMemoryStore::new().with_docs(&ns("d", "c"), docs());
      let (tx, mut h) = start(vec![("mongodb://a", a), ("mongodb://b", InMemoryStore::new())]);
      open(&mut h, &tx, 1).await;
      let pairs = (1..=3).map(|i| pair(i, ns("d", "c"), ns("d", "c"))).collect();
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 5, pairs })).unwrap();
      tx.send(Command::Compare(CompareCommand::Cancel { session: 1, run: 5 })).unwrap();
      let (done, _) = collect_run(&mut h, 3).await;
      assert_eq!(done.len(), 3, "one terminal event per pair, even though the run was cancelled");
      assert!(done.iter().all(|(_, o)| matches!(o, PairOutcome::Cancelled(_) | PairOutcome::Completed(_))));
      assert!(done.iter().any(|(_, o)| matches!(o, PairOutcome::Cancelled(_))));
  }

  #[tokio::test]
  async fn closing_mid_run_emits_nothing_after_the_close() {
      let docs = (0..50_000).map(|n| doc! { "_id": n }).collect();
      let a = InMemoryStore::new().with_docs(&ns("d", "c"), docs);
      let (tx, mut h) = start(vec![("mongodb://a", a), ("mongodb://b", InMemoryStore::new())]);
      open(&mut h, &tx, 1).await;
      tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 1, pairs: vec![pair(1, ns("d", "c"), ns("d", "c"))] })).unwrap();
      tx.send(Command::Compare(CompareCommand::Close { session: 1 })).unwrap();
      tokio::time::sleep(Duration::from_millis(300)).await;
      while let Ok(event) = h.rx.try_recv() {
          assert!(
              !matches!(event, Event::Compare(CompareEvent::PairDone { .. } | CompareEvent::RunDone { .. })),
              "a closed session reports nothing more: {event:?}"
          );
      }
  }
  ```

- [ ] **Step 2: Run and watch them fail.**
  Run: `cargo test -p curlybrace-core --features test-support runtime::compare`
  Expected: the new tests FAIL ("not implemented yet").

- [ ] **Step 3: Implement the runner.** Replace the whole of `runner.rs`:

  ```rust
  //! Runs the pairs of one compare run and serves their stored results.
  //!
  //! Pairs run one after another; each pair's failure is its own (a panic in the engine is caught
  //! and reported as that pair's failure). A cancel (or a close) ends the remaining pairs as
  //! `Cancelled`. Every pair gets exactly one `PairDone` and the run exactly one `RunDone`, sent
  //! after the last `PairDone`, unless the session was closed (then nothing more is sent).

  use std::sync::Arc;

  use crate::compare::diff::diff_normalized;
  use crate::compare::engine::{CancelFlag, EngineOutcome, PairInput, Progress, Summary, run_pair};
  use crate::compare::key::KeyTuple;
  use crate::compare::normalize::normalize;
  use crate::compare::protocol::{CompareEvent, PairId, PairOutcome, PairSpec, RowView, RunId, SessionId};
  use crate::compare::results::Category;
  use crate::error::{CoreError, Result};

  use super::session::{ActiveRun, Session, StoredPair};
  use super::{Events, Repaint, Shared, connections, emit};

  pub(super) fn cancel(shared: &Shared, session: SessionId, run: RunId) {
      let Some(session) = shared.compare.get(session) else {
          return;
      };
      let guard = session.run.lock().unwrap_or_else(|e| e.into_inner());
      if let Some(active) = guard.as_ref().filter(|a| a.id == run) {
          active.cancel.cancel();
      }
  }

  pub(super) fn discard(shared: &Shared, session: SessionId, pair: PairId) {
      if let Some(session) = shared.compare.get(session) {
          // Dropping the stored set returns its share of the budget.
          session.pairs.lock().unwrap_or_else(|e| e.into_inner()).remove(&pair);
      }
  }

  pub(super) fn start(
      shared: Arc<Shared>,
      events: Events,
      repaint: Repaint,
      session: Arc<Session>,
      session_id: SessionId,
      run: RunId,
      pairs: Vec<PairSpec>,
  ) -> Result<()> {
      connections(&session)?;
      let cancel = CancelFlag::default();
      {
          let mut slot = session.run.lock().unwrap_or_else(|e| e.into_inner());
          if slot.is_some() {
              return Err(CoreError::Unsupported("a comparison is already running".into()));
          }
          *slot = Some(ActiveRun { id: run, cancel: cancel.clone() });
      }
      tokio::spawn(async move {
          for spec in pairs {
              let outcome = run_one(&session, &events, &repaint, session_id, run, spec.clone(), &cancel).await;
              if !session.is_open() {
                  break;
              }
              emit(&events, &repaint, CompareEvent::PairDone { session: session_id, run, pair: spec.id, outcome });
          }
          *session.run.lock().unwrap_or_else(|e| e.into_inner()) = None;
          if session.is_open() {
              emit(&events, &repaint, CompareEvent::RunDone { session: session_id, run });
          }
          drop(shared);
      });
      Ok(())
  }

  async fn run_one(
      session: &Arc<Session>,
      events: &Events,
      repaint: &Repaint,
      session_id: SessionId,
      run: RunId,
      spec: PairSpec,
      cancel: &CancelFlag,
  ) -> PairOutcome {
      if cancel.is_cancelled() {
          return PairOutcome::Cancelled(Summary::default());
      }
      let Ok(conns) = connections(session) else {
          return PairOutcome::Failed(CoreError::Unsupported("the comparison is closed".into()));
      };
      let input = PairInput {
          source: conns.source.clone(),
          source_ns: spec.source.clone(),
          source_filter: spec.source_filter.clone(),
          target: conns.target.clone(),
          target_ns: spec.target.clone(),
          target_filter: spec.target_filter.clone(),
          key: spec.key.clone(),
          rules: spec.rules.clone(),
          budget: session.budget.clone(),
      };
      let (events_c, repaint_c, pair_id) = (events.clone(), repaint.clone(), spec.id);
      let cancel_c = cancel.clone();
      // The engine runs in its own task so a panic fails this pair only.
      let joined = tokio::spawn(async move {
          let mut report = |p: Progress| {
              emit(&events_c, &repaint_c, CompareEvent::Progress {
                  session: session_id,
                  run,
                  pair: pair_id,
                  scanned_source: p.scanned_source,
                  scanned_target: p.scanned_target,
                  differences: p.differences,
              });
          };
          run_pair(input, &cancel_c, &mut report).await
      })
      .await;
      match joined {
          Ok(Ok((EngineOutcome::Completed, summary, results))) => {
              session.pairs.lock().unwrap_or_else(|e| e.into_inner()).insert(spec.id, StoredPair { spec, results });
              PairOutcome::Completed(summary)
          }
          Ok(Ok((EngineOutcome::Cancelled, summary, _discarded))) => PairOutcome::Cancelled(summary),
          Ok(Err(error)) => PairOutcome::Failed(error),
          Err(_) => PairOutcome::Failed(CoreError::Database("internal error".into())),
      }
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) fn rows(
      session: &Session,
      events: &Events,
      repaint: &Repaint,
      session_id: SessionId,
      pair: PairId,
      category: Category,
      offset: usize,
      limit: usize,
  ) -> Result<()> {
      let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
      let stored = pairs.get(&pair).ok_or_else(|| CoreError::Unsupported("no results for this pair".into()))?;
      let rows = stored
          .results
          .page(category, offset, limit)
          .iter()
          .map(|e| RowView { key: e.key.clone(), has_source: e.source.is_some(), has_target: e.target.is_some() })
          .collect();
      let total = stored.results.count(category);
      drop(pairs);
      emit(events, repaint, CompareEvent::Rows { session: session_id, pair, category, offset, total, rows });
      Ok(())
  }

  /// Re-reads both documents for `key` and diffs them under the pair's rules.
  pub(super) async fn detail(
      session: &Session,
      session_id: SessionId,
      pair: PairId,
      key: KeyTuple,
  ) -> Result<CompareEvent> {
      let spec = {
          let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
          pairs.get(&pair).ok_or_else(|| CoreError::Unsupported("no results for this pair".into()))?.spec.clone()
      };
      let conns = connections(session)?;
      let (source, target) = tokio::try_join!(
          fetch_one(&conns.source, &spec.source, &spec.source_filter, &spec.key, &key),
          fetch_one(&conns.target, &spec.target, &spec.target_filter, &spec.key, &key)
      )?;
      let changes = match (&source, &target) {
          (Some(s), Some(t)) => diff_normalized(&normalize(s, &spec.rules)?, &normalize(t, &spec.rules)?, spec.rules.ignore_array_order)?,
          _ => Vec::new(),
      };
      Ok(CompareEvent::Detail { session: session_id, pair, key, source, target, changes })
  }

  async fn fetch_one(
      store: &Arc<dyn crate::store::DocumentStore>,
      ns: &crate::store::Namespace,
      filter: &bson::Document,
      spec: &crate::compare::key::KeySpec,
      key: &KeyTuple,
  ) -> Result<Option<bson::Document>> {
      let found = store.scan_sorted(ns, filter, spec, Some(key), 1).await?;
      Ok(found.into_iter().next().filter(|d| {
          crate::compare::key::extract_key(d, spec).is_ok_and(|k| crate::compare::key::compare_tuples(&k, key).is_eq())
      }))
  }
  ```
  `session.pairs` and `session.run` are `pub(super)` in `session.rs`, which is visible to `runner.rs` (both children of `runtime::compare`).

- [ ] **Step 4: Run the runtime tests.**
  Run: `cargo test -p curlybrace-core --features test-support runtime::`
  Expected: PASS. If `closing_mid_run...` is flaky because the engine finishes before `Close` is processed, raise the document count; the assertion (nothing after the close) must hold regardless, because `run_one` is followed by an `is_open()` check before every emit.

- [ ] **Step 5: Write the tier-1 session test.** Create `crates/core/tests/local_compare_session.rs` (uses the real driver against the local server, two throwaway databases, through the real `MongoConnector`; follow `tests/local_mongo_crud.rs` for how it obtains the URI and builds a `ConnectionConfig`):

  ```rust
  //! Tier 1 (local server): a compare of two real databases through the runtime. Writes only to
  //! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

  mod support;

  use bson::{Document, doc};
  use curlybrace_core::compare::key::KeySpec;
  use curlybrace_core::compare::protocol::{CompareCommand, CompareEvent, PairOutcome, PairSpec};
  use curlybrace_core::compare::rules::CompareRules;
  use curlybrace_core::runtime::{Command, Event};
  use support::{local_enabled, local_runtime};

  #[tokio::test]
  #[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
  async fn two_real_databases_compare_end_to_end() {
      if !local_enabled("two_real_databases_compare_end_to_end") {
          return;
      }
      let (dbs, client, mut runtime, config) = local_runtime().await;
      let (a, b) = (dbs.ns("left"), dbs.ns("right"));
      let seed = |ns: &curlybrace_core::store::Namespace, docs: Vec<Document>| {
          let c = client.database(&ns.db).collection::<Document>(&ns.collection);
          async move { c.insert_many(docs).await.unwrap() }
      };
      seed(&a, vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 2 }, doc! { "_id": 3, "v": 3 }]).await;
      seed(&b, vec![doc! { "_id": 2, "v": 2 }, doc! { "_id": 3, "v": 30 }, doc! { "_id": 4, "v": 4 }]).await;
      runtime.tx.send(Command::Compare(CompareCommand::Open { session: 1, source: config.clone(), target: config })).unwrap();
      assert_eq!(support::next_compare(&mut runtime).await, CompareEvent::Opened { session: 1 });
      let key = KeySpec::id();
      let spec = PairSpec { id: 1, source: a, target: b, source_filter: doc! {}, target_filter: doc! {}, rules: CompareRules::for_key(&key), key };
      runtime.tx.send(Command::Compare(CompareCommand::Start { session: 1, run: 1, pairs: vec![spec] })).unwrap();
      loop {
          match support::next_compare(&mut runtime).await {
              CompareEvent::PairDone { outcome: PairOutcome::Completed(s), .. } => {
                  assert_eq!((s.equal, s.only_in_source, s.only_in_target, s.different), (1, 1, 1, 1));
              }
              CompareEvent::RunDone { .. } => break,
              CompareEvent::Progress { .. } => {}
              other => panic!("{other:?}"),
          }
      }
      runtime.tx.send(Command::Compare(CompareCommand::Close { session: 1 })).unwrap();
      let _ = Event::Disconnected { epoch: 0 };
  }
  ```
  Add the two helpers it names to `crates/core/tests/support/mod.rs`: `local_runtime()` returns `(LocalDbs, mongodb::Client, RuntimeHandle, ConnectionConfig)` built exactly like the existing `local_mongo()` helper plus `spawn_runtime_with_connector(None, None, Arc::new(MongoConnector), no_repaint(), RuntimeConfig::default())` and `ConnectionConfig::new("local", &uri)`; `next_compare(&mut RuntimeHandle)` is the same loop as in the unit tests (skip non-compare events, 10-second timeout). Run it against your OWN throwaway container exactly as in Task 4 Step 10 (never against `127.0.0.1:27017` or an existing container).

- [ ] **Step 6: Format, lint, commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): run pairs with isolated failures, cancel, rows, detail and discard

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 9: UI state, view-models and reducer integration

**Files:**
- Create: `crates/ui/src/viewmodel/compare/mod.rs`, `setup.rs`, `results.rs`, `detail.rs`; `crates/ui/src/app/compare.rs` (reducer for compare inputs), `crates/ui/src/app/compare_tests.rs`
- Modify: `crates/ui/src/viewmodel/mod.rs`, `crates/ui/src/app/mod.rs`, `crates/ui/src/app/state.rs`, `crates/ui/src/app/reducer.rs`, `crates/ui/src/app/banner.rs` (failure text for the new contexts), `crates/ui/locales/en.json` (`failures.compare_*`), `crates/ui/src/i18n.rs` (completeness list)

**Interfaces:**
- Consumes: Task 7/8 protocol types, `AppState`, reducer `Input`/`update`/`on_event`, `Banner::Failure`.
- Produces:
  ```rust
  // state.rs
  pub enum Workspace { #[default] Browse, Compare }                         // Clone, Copy, Debug, Default, PartialEq, Eq
  AppState { pub workspace: Workspace, pub compare: CompareState, /* existing fields */ }
  // viewmodel/compare/mod.rs
  pub struct CompareState { pub session: Option<SessionPhase>, pub next_session: u64, pub next_run: u64,
                            pub setup: SetupForm, pub preflight: Preflight, pub run: RunView, pub results: Option<ResultsView>,
                            pub detail: Option<DetailView> }
  pub enum SessionPhase { Opening(SessionId), Ready(SessionId) }
  // reducer: Input::Compare(CompareAction) with
  pub enum CompareAction { Show, Hide, Open { source: Uuid, target: Uuid }, CloseSession, PickSide(Side, Pick), SetKey(String), SetFilter(Side, String),
        SetRules(RulesForm), RunCheck, Start, Cancel, SelectCategory(Category), Page(i64), Select(KeyTuple), Discard, Back }
  ```
  (The exact enum members above are the contract the views in Task 10 use. Keep them as written.)

- [ ] **Step 1: Failing view-model tests.** Write unit tests (one file per view-model, in the same style as `viewmodel/findbar.rs`) that pin:

  1. `SetupForm` — `key_text` parses `"_id"` or comma-separated dotted paths into a `KeySpec` (errors surface as `Err(CoreError::InvalidDocument)` mapped to the key `compare.setup.key_invalid`, not English); `filter_text` empty → `{}`; `{"a": 1}` → document; invalid JSON → error key `compare.setup.filter_invalid`; `RulesForm` defaults equal `CompareRules::for_key`; the form is `ready()` only when both sides are picked, both are not views, and key and filters parse.
  2. `PreflightView::from(Preflight)` → states: `Ok`, `NeedsIndex { side }` (maps to a message key `compare.preflight.needs_index` with params), `EmptyBoth`.
  3. `ResultsView` — three category counts, current category, `offset`, page size 100; `Page(+1)`/`Page(-1)` clamp to `[0, total)`; `Select(key)` returns a `CompareCommand::Detail`; rows beyond `total` are ignored; stale `Rows` events (different `pair`, `category` or `offset` than requested) are dropped.
  4. `DetailView` — `changes` rendered as `ChangeLine { path, kind, source_text, target_text }` with values formatted by the existing `json` formatter (`crate::viewmodel::json_cache` or `cell`) and `Added/Removed/Changed` as i18n keys `compare.detail.added|removed|changed`.

  Write each as concrete `#[test]`s using real `bson`/`CompareRules` values (no mocks), then implement until green.

- [ ] **Step 2: Failing reducer tests** in `crates/ui/src/app/compare_tests.rs` (declared from `app/compare.rs` via `#[cfg(test)] #[path = "compare_tests.rs"] mod tests;` like `reducer.rs` does):

  ```rust
  use super::*;
  use crate::app::reducer::{Input, update};
  use crate::app::state::{AppState, ConnStatus, Workspace};
  use crate::app::state::Banner;
  use crate::viewmodel::compare::{RunView, SessionPhase};
  use curlybrace_core::compare::engine::Summary;
  use curlybrace_core::compare::protocol::{CompareCommand, CompareEvent, PairOutcome};
  use curlybrace_core::connection::ConnectionConfig;
  use curlybrace_core::error::CoreError;
  use curlybrace_core::runtime::{Command, Event};

  fn two_connections() -> (AppState, uuid::Uuid, uuid::Uuid) {
      let (a, b) = (ConnectionConfig::new("a", "mongodb://a"), ConnectionConfig::new("b", "mongodb://b"));
      let ids = (a.id, b.id);
      (AppState::new(vec![a, b]), ids.0, ids.1)
  }

  #[test]
  fn open_sends_open_with_a_fresh_session_id_and_shows_the_workspace() {
      let (state, a, b) = two_connections();
      let (state, cmds) = update(state, Input::Compare(CompareAction::Open { source: a, target: b }));
      assert_eq!(state.workspace, Workspace::Compare);
      assert!(matches!(&cmds[..], [Command::Compare(CompareCommand::Open { session: 1, .. })]));
      assert!(matches!(state.compare.session, Some(SessionPhase::Opening(1))));
  }

  #[test]
  fn opening_the_same_pair_twice_ignores_the_second_request() {
      let (state, a, b) = two_connections();
      let (state, _) = update(state, Input::Compare(CompareAction::Open { source: a, target: b }));
      let (_, cmds) = update(state, Input::Compare(CompareAction::Open { source: a, target: b }));
      assert!(cmds.is_empty());
  }

  fn opened() -> (AppState, uuid::Uuid, uuid::Uuid) {
      let (state, a, b) = two_connections();
      let (state, _) = update(state, Input::Compare(CompareAction::Open { source: a, target: b }));
      let (state, cmds) = update(state, Input::Event(Event::Compare(CompareEvent::Opened { session: 1 })));
      assert!(matches!(state.compare.session, Some(SessionPhase::Ready(1))));
      assert_eq!(cmds.len(), 2, "one database listing per side");
      (state, a, b)
  }

  #[test]
  fn opened_moves_to_ready_and_lists_databases_for_both_sides() {
      let (_, _, _) = opened();
  }

  #[test]
  fn open_failed_clears_the_session_and_shows_a_banner() {
      let (state, a, b) = two_connections();
      let (state, _) = update(state, Input::Compare(CompareAction::Open { source: a, target: b }));
      let ev = Event::Compare(CompareEvent::OpenFailed { session: 1, error: CoreError::Database("x".into()) });
      let (state, cmds) = update(state, Input::Event(ev));
      assert!(state.compare.session.is_none());
      assert!(cmds.is_empty());
      assert!(matches!(state.banner, Some(Banner::Failure { context: "compare_open", .. })));
  }

  #[test]
  fn start_is_refused_until_the_form_is_ready_and_the_preflight_allows_it() {
      let (state, _, _) = opened();
      let (state, cmds) = update(state, Input::Compare(CompareAction::Start));
      assert!(cmds.is_empty(), "nothing is picked yet");
      assert_eq!(state.compare.run, RunView::Idle);
  }

  #[test]
  fn one_terminal_event_per_pair_and_the_run_terminal_update_the_run_view() {
      let state = running_state();
      let done = Event::Compare(CompareEvent::PairDone { session: 1, run: 1, pair: 1, outcome: PairOutcome::Completed(Summary::default()) });
      let (state, _) = update(state, Input::Event(done));
      assert!(matches!(state.compare.run, RunView::Running { finished_pairs: 1, .. }));
      let (state, _) = update(state, Input::Event(Event::Compare(CompareEvent::RunDone { session: 1, run: 1 })));
      assert!(matches!(state.compare.run, RunView::Finished { .. }));
  }

  #[test]
  fn a_late_pair_done_for_an_older_run_is_ignored() {
      let state = running_state();
      let late = Event::Compare(CompareEvent::PairDone { session: 1, run: 0, pair: 1, outcome: PairOutcome::Completed(Summary::default()) });
      let (after, cmds) = update(state, Input::Event(late));
      assert!(matches!(after.compare.run, RunView::Running { finished_pairs: 0, .. }));
      assert!(cmds.is_empty());
  }

  #[test]
  fn leaving_the_workspace_keeps_the_session_and_closing_it_sends_close() {
      let (state, _, _) = opened();
      let (state, cmds) = update(state, Input::Compare(CompareAction::Hide));
      assert_eq!(state.workspace, Workspace::Browse);
      assert!(cmds.is_empty() && state.compare.session.is_some());
      let (state, cmds) = update(state, Input::Compare(CompareAction::CloseSession));
      assert!(matches!(&cmds[..], [Command::Compare(CompareCommand::Close { session: 1 })]));
      assert!(state.compare.session.is_none());
  }

  #[test]
  fn disconnecting_the_main_connection_does_not_close_the_compare_session() {
      let (state, _, _) = opened();
      let (state, cmds) = update(state, Input::Disconnect);
      assert!(state.compare.session.is_some(), "the compare session has its own connections");
      assert!(!cmds.iter().any(|c| matches!(c, Command::Compare(_))));
  }
  ```
  `running_state()` is a helper in the same test file that builds an opened state, fills the setup form with a source and target collection (using the `CompareAction::PickSide` and `SetKey` actions), applies a `Checked` event with an indexed `Preflight`, and sends `CompareAction::Start`; it asserts the resulting command is `Command::Compare(CompareCommand::Start { session: 1, run: 1, .. })` and returns the state. Keep the pair id fixed at 1 for the single-pair workspace.

- [ ] **Step 3: Implement.** Keep `reducer.rs` under 800 lines by putting all compare logic in `app/compare.rs`: `Input::Compare(a) => compare::on_action(state, a)`; in `on_event`, add as the FIRST arm `Event::Compare(e) => compare::on_event(state, e)` (so compare events are handled before the "not connected" catch-all and the main-connection epoch logic, which they must not depend on). Session ids come from `state.compare.next_session` (starting at 1, incremented per `Open`), run ids from `next_run`. The pure functions (`on_action`, `on_event`) take and return `AppState` like the rest of the reducer. `failure_log_line` (the existing function that turns a banner failure into a log line) must learn the new contexts without logging keys or values; add a test beside the existing ones.

- [ ] **Step 4: i18n for failures.** Add `failures.compare_open`, `compare_databases`, `compare_collections`, `compare_check`, `compare_start`, `compare_rows`, `compare_detail` to `en.json` (alphabetical, nested like the existing `failures` block, plain-language text), and the completeness test in `i18n.rs` (the one that walks every `Command::context()`-derived key) must include them.

- [ ] **Step 5: Run and commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add compare state, view-models and reducer integration

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 10: Compare workspace views, entry points and i18n

**Files:**
- Create: `crates/ui/src/views/compare/mod.rs`, `setup.rs`, `preflight.rs`, `running.rs`, `results.rs`, `detail.rs`
- Modify: `crates/ui/src/views/mod.rs`, `crates/ui/src/app/panels.rs`, `crates/ui/src/app/mod.rs` (frame routing and `UiEvent` handling), `crates/ui/src/views/sidebar.rs` and `crates/ui/src/viewmodel/sidebar.rs` (the "Compare to…" context-menu entry), `crates/ui/locales/en.json` (`compare.*`), `README.md`

**Interfaces:**
- Consumes: Task 9's `CompareState`, `CompareAction`, `Workspace`.
- Produces: `show_compare(ui, &mut AppState.compare, &ConnectionsState, i18n) -> Option<CompareAction>` and one function per sub-view, each taking its view-model and returning `Option<CompareAction>`; no view reads `AppState` directly or contains a decision (the same split as `views/findbar.rs`).

- [ ] **Step 1: Layout.** The workspace is a `CentralPanel` with, top to bottom: the workspace switcher (Browse | Compare) next to the connection button in `panels.rs`; a single-pair setup form (source connection → database → collection; target the same; key field; two filter fields; a collapsible "Rules" section with the five rule controls); the preflight strip (counts, index status, a blocking message when `unindexed_too_large`); a Start/Cancel button with a progress bar and counters while running; then the results area: three category tabs with counts, a table of keys (100 per page, Prev/Next), and, when a row is selected, the detail pane listing `FieldChange` lines. A sidebar context menu entry "Compare to…" on a collection pre-fills the source side and switches the workspace.

- [ ] **Step 2: Strings.** Every label, tooltip, placeholder, button, tab title, empty state, preflight message and rule name is an i18n key under `compare.*` in `en.json` (nested, alphabetical). The preflight and error text gives next steps ("Create an index on the key fields, then check again."). Count labels use parameters (`{count}`), never string concatenation. Add the keys in the same step as the view that uses them; the existing `i18n.rs` completeness test fails for any `t("…")` key missing from `en.json`, and a second test added here fails for any `compare.*` key in `en.json` that no view uses.

- [ ] **Step 3: Build the views** one function per file, following `views/findbar.rs` for structure (a `show_*` function, no logic, returns the action). Add `ui.push_id` with stable ids derived from the pair/category/key so kittest can find widgets, and `widget_info`/accessible labels on every control (the existing `accessibility.rs` test walks the tree; extend it to open the Compare workspace and assert every interactive node has a label).

- [ ] **Step 4: Frame tests** (in `app/mod.rs` next to the existing headless frame tests): the workspace renders in each phase — no session, opening, ready with an empty form, preflight blocked, running, results with all three categories empty, results with rows, detail open — without panicking and without drawing any literal English (assert that rendering with the pseudo-locale test catalogue used by the existing i18n tests produces no unmatched keys).

- [ ] **Step 5: Run and commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates README.md && git commit -m "feat(compare): add the Compare workspace views and sidebar entry

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 11: End-to-end flows, snapshots, docs and the spec addenda

**Files:**
- Create: `crates/ui/tests/local_compare_flows.rs`
- Modify: `crates/ui/tests/snapshots.rs`, `crates/ui/tests/support/*` (helpers), `crates/ui/tests/snapshots/` (new baselines), `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md`, `README.md`, `docs/` site page for the feature, `CHANGELOG` is NOT edited by hand (release-please owns it)

- [ ] **Step 1: Click-driven flows (tier 1, `#[ignore]`, env-gated, throwaway databases).** Using the existing kittest harness (`tests/support/`), drive: connect two saved connections to the local server → open Compare → pick source and target collections → Check → Start → wait for results → select the Different tab → select a row → assert the detail pane lists the changed field's path → Discard → assert the results are gone. A second flow: a duplicate key on one side shows the localized duplicate-key message and no partial results. A third: Cancel mid-run on a large collection leaves the workspace usable (a new Check/Start works). These run only in `scripts/integration-tests.sh` and never touch the user's containers.

- [ ] **Step 2: Snapshots.** Add snapshot tests (same mechanism as the existing 11, `ui-snapshots` job, macOS only) for: setup form empty; preflight blocked by a missing index; running with progress; results with rows; detail pane. Generate baselines locally with the project's existing update command (`scripts/…`/`UPDATE_SNAPSHOTS`, see `tests/snapshots.rs` header) and review each image by eye before committing.

- [ ] **Step 3: Check the spec still matches the code.** The addenda (inclusive `from`, type-aware keyset filter, `CompareKeyInvalid`) are already in the spec. Re-read the spec sections "`DocumentStore` additions" and "Errors" against the final code and correct any drift; also record that Double-versus-Decimal128 comparison is by exact value and is pinned by the tier-1 conformance test.

- [ ] **Step 4: Docs.** README gets a short "Compare two collections" section (what it compares, that nothing is written in slice 1, the index requirement, the limits). The site page documents the same. State plainly that Apply and export arrive in later slices (#8).

- [ ] **Step 5: Full verification and the review loop.** Run the whole default suite, then tier 1 against your own throwaway container (never an existing one), then the snapshot job locally:
  ```bash
  cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  scripts/check-no-db-in-ci.sh
  ```
  Run a whole-branch review on the most capable model (security-sensitive: tunnel gate generalization, key handling, memory bounds), fix every CRITICAL/HIGH finding, and re-review each fix until a full pass finds nothing new. Re-walk every call site of the changed `open_tunnel_if_configured` and of the `DocumentStore` additions (default `Unsupported` impls: which test doubles now silently lack them?) and state what the change means for each.

- [ ] **Step 6: Commit and open the PR** (target `main`; body ends with the Claude Code line; `Refs #8`, not `Closes`, because slices 1b, 1c, 2 and 3 remain).
  ```bash
  git add -A && git commit -m "feat(compare): add compare end-to-end flows, snapshots and docs

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```
