# Data Compare, Slice 1b: Find Duplicate Keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a compare stops because documents share a key, let the user press **Find duplicate keys** and see which key values collide and how often, on each side, so they can pick a better key.

**Architecture:** One new read-only `DocumentStore` method (`duplicate_keys`, a server-side `$group` on the key) for the in-memory and Mongo stores; one new compare command and event (`DuplicateKeys`) that asks both sides of a pair; a small UI state machine and result panel under the pair's failure message. Nothing is written to any database, and the keys are shown in the UI only, never logged.

**Tech Stack:** Rust, `bson` 2.15, `mongodb` 3.6, tokio, egui/eframe 0.32 (same as slice 1).

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` (the "Errors" section that introduces **Find duplicate keys**, the cross-check table row "Multiple Matches view", and testing item 9). Issue #8. Slice 1 shipped in PR #29.

## Decisions made while planning

1. **Both sides are asked.** A failed pair reports `CompareDuplicateKey` without saying which side collided, so the command asks the source and the target concurrently and the panel shows a group per side (a side with no duplicates says so).
2. **`duplicate_keys(ns, filter, key, limit)` returns at most `limit` groups, worst first** (count descending, ties by key order). The runtime asks for `limit + 1` and reports `more: true` when it got more than `limit`, so the UI can say "and more".
3. **Documents that lack a key field are excluded** from the grouping (they are a different error, `CompareKeyMissing`, which stops the compare earlier). Every other value is grouped as the server's `$group` groups it: numbers equal across types (`1`, `1.0`, `NumberLong(1)`) fall in one group, which is exactly how the scanner treats them. The in-memory store reproduces this with the slice 1 comparator.
4. **The aggregation runs server-side with `allowDiskUse` and simple collation**, so memory on the client is bounded by `limit`. It cannot use an index for the grouping, so it can be slow on large collections: it gets its own timeout (`duplicate_timeout`, default 120 s) instead of the 30 s data-command timeout. Closing the session while it runs silences the answer at once (`emit_if_open`) but the connection is released only when the call returns or times out, the same as a `Check` in flight.
5. **One request at a time, and edits invalidate it**, the same rule as the preflight check: asking again while a request is in flight does nothing, and editing the setup while it is in flight marks the answer stale so it is dropped.
6. **Slice 1 workspace has one pair**, so the button appears under the failure of that pair. Slice 1c (multi-pair) will reuse the command and event unchanged and show the button per failed pair.

## Global Constraints

- Strictly **read-only** against databases. No document data or keys on disk, in logs, or in `Debug` output of anything that is logged. The log line for a failed command carries only the context and the error's i18n key (slice 1 rule, `failure_log_line`).
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical). Errors are typed `CoreError` variants. No literal English in views, view-models or the reducer.
- Immutability; files < 800 lines (aim 200-400); functions < 50 lines; nesting <= 4.
- TDD for every task. Default `cargo test --workspace` stays DB-, docker- and GPU-free. Tier-1 tests (local server) are `#[ignore]` + env-gated, write only to throwaway `cbit_<id>` databases, and run ONLY against a container named `cb-it-dev-*` that the test run starts itself, never against the user's own containers (`local7979`, `local265`) or `127.0.0.1:27017`. GitHub CI must never need MongoDB (`scripts/check-no-db-in-ci.sh` stays green).
- Protocol rules from slice 1 still hold: a command that names an unknown or closed session is answered with `Failed { context }`; every session-tied emission goes through `Session::emit_if_open`; nothing is emitted after `Close`; panics in compare tasks are contained and answered.
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet; security-sensitive and final reviews use a stronger model.

## Review Focus

1. **Grouping semantics disagree between stores.** `1`, `1.0` and `NumberLong(1)` must collide on both; `null` and a missing field must not (missing is excluded); `NaN` collides with `NaN`; compound keys group on the whole tuple. Pinned in Task 1 (unit + real server).
2. **Keys leaking out of the UI.** Keys are data: they must not appear in logs, banner text built from `Debug`, or a `Debug`-formatted `Command`/`Event` that is logged. Pinned in Tasks 2 and 3 (log-line tests with a recognisable secret-like key value).
3. **Stale answers.** An answer for an edited form, a restarted run, a closed or replaced session, or an older request must not populate the panel. Pinned in Task 3.
4. **Silent truncation.** Exactly `limit` groups must not look like "all"; `limit + 1` must say "more". Pinned in Tasks 1 and 2.
5. **Slow or hung aggregation.** The call is bounded by its own timeout and the answer after `Close` is silent. Pinned in Task 2.

## File Structure

```
crates/core/src/
  store.rs                         + DuplicateGroup, DocumentStore::duplicate_keys (default Unsupported)   (Task 1)
  compare/key.rs                   + key_values (None when a key field is missing)                          (Task 1)
  test_support/memory_store.rs     + duplicate_keys                                                          (Task 1)
  mongo_store.rs                   + duplicate_keys ($group pipeline)                                        (Task 1)
  compare/protocol.rs              + CompareCommand::DuplicateKeys, CompareEvent::DuplicateKeys, DuplicateReport, DUPLICATE_KEYS_LIMIT   (Task 2)
  runtime/mod.rs                   + RuntimeConfig.duplicate_timeout                                         (Task 2)
  runtime/compare/mod.rs           + handler, context "compare_duplicates"                                  (Task 2)
  runtime/compare/duplicates_tests.rs   offline runtime tests                                               (Task 2)
crates/core/tests/local_compare_store.rs   + duplicate_keys against a real server                           (Task 1)
crates/ui/src/
  viewmodel/compare/duplicates.rs  DuplicatesView + row view-models                                         (Task 3)
  viewmodel/compare/mod.rs         CompareState.duplicates, resets                                         (Task 3)
  app/compare.rs                   CompareAction::FindDuplicates, event handling                          (Task 3)
  app/compare_tests/duplicates.rs  reducer tests                                                            (Task 3)
  error_text.rs, banner.rs         context compare_duplicates                                              (Task 3)
  views/compare/duplicates.rs      the panel                                                                 (Task 4)
  views/compare/running.rs         button under the failure                                                 (Task 4)
  locales/en.json                  compare.duplicates.*, failures.compare_duplicates                        (Tasks 3, 4)
crates/ui/tests/
  local_compare_flows.rs           + duplicate-key flow lists the colliding keys                            (Task 4)
  snapshots.rs                     + compare_duplicates_panel                                                (Task 4)
docs: README.md, spec "as built" note                                                                      (Task 4)
```

---

### Task 1: `duplicate_keys` in the store contract

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/compare/key.rs`, `crates/core/src/test_support/memory_store.rs`, `crates/core/src/mongo_store.rs`, `crates/core/tests/local_compare_store.rs`

**Interfaces:**
- Consumes: `KeySpec`, `KeyTuple`, `compare_tuples`, `Namespace`, existing `check_filter`/`matches` (in-memory), `db_err` (Mongo).
- Produces:
  ```rust
  // store.rs
  #[derive(Clone, Debug, PartialEq)]
  pub struct DuplicateGroup { pub key: KeyTuple, pub count: u64 }
  // DocumentStore, default body returns CoreError::Unsupported:
  /// Key tuples shared by two or more documents matching `filter`, at most `limit`, most frequent
  /// first (ties in key order). Documents lacking a key field are not counted. A missing
  /// collection has no duplicates.
  async fn duplicate_keys(&self, ns: &Namespace, filter: &Document, key: &KeySpec, limit: u32)
      -> Result<Vec<DuplicateGroup>>;
  // compare/key.rs
  /// The key tuple of `doc`, or None when a key field is missing. Unlike `extract_key` it does
  /// not refuse arrays or deprecated types (the duplicate finder reports whatever collides).
  pub fn key_values(doc: &Document, spec: &KeySpec) -> Option<KeyTuple>;
  ```

- [ ] **Step 1: Failing unit tests for `key_values`.** Add to the test module of `crates/core/src/compare/key.rs`:

  ```rust
      #[test]
      fn key_values_returns_none_for_a_missing_field_and_keeps_unusual_values() {
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          assert_eq!(key_values(&doc! { "x": 1 }, &spec), None);
          assert_eq!(
              key_values(&doc! { "k": [1, 2] }, &spec),
              Some(KeyTuple(vec![Bson::Array(vec![Bson::Int32(1), Bson::Int32(2)])]))
          );
          let nested = KeySpec::new(vec!["a.b".into()]).unwrap();
          assert_eq!(key_values(&doc! { "a": { "b": 3 } }, &nested), Some(KeyTuple(vec![Bson::Int32(3)])));
          assert_eq!(key_values(&doc! { "a": [{ "b": 3 }] }, &nested), None, "a path never crosses an array");
      }
  ```

  Run `cargo test -p curlybrace-core compare::key::tests::key_values` and watch it fail to compile.

- [ ] **Step 2: Implement `key_values`.** In `compare/key.rs`, reuse the private `lookup`:

  ```rust
  /// The key tuple of `doc`, or `None` when a key field is missing or unreachable.
  pub fn key_values(doc: &Document, spec: &KeySpec) -> Option<KeyTuple> {
      spec.fields
          .iter()
          .map(|path| lookup(doc, path).cloned())
          .collect::<Option<Vec<_>>>()
          .map(KeyTuple)
  }
  ```

  Run the test again: PASS.

- [ ] **Step 3: Failing in-memory tests.** In the test module of `crates/core/src/test_support/memory_store.rs`:

  ```rust
      fn dup_store() -> InMemoryStore {
          let ns = Namespace::new("db", "c");
          InMemoryStore::new().with_docs(
              &ns,
              vec![
                  doc! { "_id": 1, "k": 1_i32, "t": "x" },
                  doc! { "_id": 2, "k": 1_i64, "t": "x" },
                  doc! { "_id": 3, "k": 1.0_f64, "t": "y" },
                  doc! { "_id": 4, "k": "a" },
                  doc! { "_id": 5, "k": "a", "t": "x" },
                  doc! { "_id": 6, "k": "b" },
                  doc! { "_id": 7, "t": "x" },
                  doc! { "_id": 8, "t": "x" },
              ],
          )
      }

      #[tokio::test]
      async fn duplicate_keys_groups_numbers_equal_across_types_and_orders_worst_first() {
          let ns = Namespace::new("db", "c");
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          let groups = dup_store().duplicate_keys(&ns, &doc! {}, &spec, 10).await.unwrap();
          assert_eq!(
              groups,
              vec![
                  DuplicateGroup { key: KeyTuple(vec![Bson::Int32(1)]), count: 3 },
                  DuplicateGroup { key: KeyTuple(vec![Bson::String("a".into())]), count: 2 },
              ],
              "Int32 1, Int64 1 and Double 1.0 are one key; 'b' is unique; documents without k are not counted"
          );
      }

      #[tokio::test]
      async fn duplicate_keys_respects_the_filter_the_limit_and_compound_keys() {
          let ns = Namespace::new("db", "c");
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          let store = dup_store();
          let only_x = store.duplicate_keys(&ns, &doc! { "t": "x" }, &spec, 10).await.unwrap();
          assert_eq!(
              only_x,
              vec![DuplicateGroup { key: KeyTuple(vec![Bson::Int32(1)]), count: 2 }],
              "under t == 'x' only _id 1 and 2 share a key; the filter is applied before grouping"
          );
          let limited = store.duplicate_keys(&ns, &doc! {}, &spec, 1).await.unwrap();
          assert_eq!(limited.len(), 1);
          assert_eq!(limited[0].count, 3);
          let compound = KeySpec::new(vec!["k".into(), "t".into()]).unwrap();
          let groups = store.duplicate_keys(&ns, &doc! {}, &compound, 10).await.unwrap();
          assert_eq!(groups.len(), 1, "only (1, 'x') repeats: _id 1 and 2");
          assert_eq!(groups[0].count, 2);
      }

      #[tokio::test]
      async fn duplicate_keys_on_a_missing_collection_is_empty_and_nan_collides_with_nan() {
          let spec = KeySpec::new(vec!["k".into()]).unwrap();
          let none = InMemoryStore::new();
          assert!(none.duplicate_keys(&Namespace::new("x", "y"), &doc! {}, &spec, 5).await.unwrap().is_empty());
          let ns = Namespace::new("db", "c");
          let nans = InMemoryStore::new().with_docs(&ns, vec![doc! { "k": f64::NAN }, doc! { "k": f64::NAN }]);
          assert_eq!(nans.duplicate_keys(&ns, &doc! {}, &spec, 5).await.unwrap()[0].count, 2);
      }
  ```

- [ ] **Step 4: Run and watch fail.** `cargo test -p curlybrace-core --features test-support memory_store::tests::duplicate_keys` → FAIL to compile (`DuplicateGroup`, `duplicate_keys` not found).

- [ ] **Step 5: Add the type and trait method** to `crates/core/src/store.rs` (after `list_collection_kinds`), with the doc comment from the Interfaces block and the default `Err(CoreError::Unsupported("this store cannot find duplicate keys".into()))`. Import `KeyTuple` where needed.

- [ ] **Step 6: Implement it in `InMemoryStore`.**

  ```rust
      async fn duplicate_keys(
          &self,
          ns: &Namespace,
          filter: &Document,
          key_spec: &KeySpec,
          limit: u32,
      ) -> Result<Vec<DuplicateGroup>> {
          check_filter(filter)?;
          let guard = self.lock()?;
          let docs = guard.get(&key(ns)).map(Vec::as_slice).unwrap_or_default();
          let mut keyed: Vec<KeyTuple> = docs
              .iter()
              .filter(|d| matches(d, filter))
              .filter_map(|d| key_values(d, key_spec))
              .collect();
          keyed.sort_by(compare_tuples);
          let mut groups: Vec<DuplicateGroup> = Vec::new();
          for tuple in keyed {
              match groups.last_mut() {
                  Some(last) if compare_tuples(&last.key, &tuple).is_eq() => last.count += 1,
                  _ => groups.push(DuplicateGroup { key: tuple, count: 1 }),
              }
          }
          groups.retain(|g| g.count > 1);
          // Stable sort: equal counts stay in key order.
          groups.sort_by(|a, b| b.count.cmp(&a.count));
          groups.truncate(limit as usize);
          Ok(groups)
      }
  ```

  Run the in-memory tests: PASS.

- [ ] **Step 7: Implement it in `MongoStore`.** The pipeline groups on aliased fields (`k0`, `k1`, ...) so any path is valid, excludes documents missing a key field, and keeps the group key's original values:

  ```rust
      async fn duplicate_keys(
          &self,
          ns: &Namespace,
          filter: &Document,
          key: &KeySpec,
          limit: u32,
      ) -> Result<Vec<DuplicateGroup>> {
          let mut group_id = Document::new();
          let mut present = Document::new();
          for (i, path) in key.fields.iter().enumerate() {
              group_id.insert(format!("k{i}"), format!("${path}"));
              present.insert(path.clone(), doc! { "$exists": true });
          }
          let mut matcher = filter.clone();
          for (path, cond) in present {
              matcher = if matcher.is_empty() {
                  doc! { path: cond }
              } else {
                  doc! { "$and": [matcher, { path: cond }] }
              };
          }
          let pipeline = vec![
              doc! { "$match": matcher },
              doc! { "$group": { "_id": group_id, "n": { "$sum": 1_i64 } } },
              doc! { "$match": { "n": { "$gt": 1_i64 } } },
              doc! { "$sort": { "n": -1_i32 } },
              doc! { "$limit": i64::from(limit) },
          ];
          let cursor = self
              .collection(ns)
              .aggregate(pipeline)
              .collation(Collation::builder().locale("simple").build())
              .allow_disk_use(true)
              .await
              .map_err(db_err)?;
          let rows: Vec<Document> = cursor.try_collect().await.map_err(db_err)?;
          rows.into_iter()
              .map(|row| group_from_row(&row, key.fields.len()))
              .collect()
      }
  ```

  and a private helper next to it that turns a row into a `DuplicateGroup` (reads `_id.k0..kN` into the tuple in order, `n` as `Int32`/`Int64` into `u64`, returning `CoreError::Database("unexpected aggregation result".into())` for a malformed row). `$sort` on `n` alone leaves ties in server order, so after collecting, apply the same stable key-order tie-break as the in-memory store: sort the vector by `(count desc, compare_tuples)`. Note for the report: `$exists: true` matches a field present with value `null`, which groups with other nulls; the in-memory store does the same because `key_values` keeps `Null`.

  **Edge to verify on the real server and pin with a test:** a path crossing an array (`a.b` where `a` is an array) is `$exists`-true on the server when any element has `b`, but `key_values` (and the scanner) treat it as missing. Decide by test: if the two disagree, make the aggregation exclude array-crossing paths with an additional `$expr` guard (`{$not: {$isArray: "$a"}}` for every proper prefix of the path) so both stores agree, and say so in a comment.

- [ ] **Step 8: Tier-1 tests against a real server.** Add to `crates/core/tests/local_compare_store.rs` (`#[ignore]`, `local_enabled`-gated, `dbs.ns(..)` databases, like the neighbouring tests):

  1. `duplicate_keys_matches_the_in_memory_semantics`: seed the same documents as `dup_store()` through the driver and assert the same groups from `MongoStore::duplicate_keys` (numbers collide across types, missing excluded, worst first).
  2. `duplicate_keys_honours_filter_limit_and_compound_keys`.
  3. `duplicate_keys_with_a_dotted_key_and_array_crossing_paths` (the edge in Step 7).
  4. `duplicate_keys_on_a_missing_collection_is_empty`.
  5. `duplicate_keys_returns_more_than_the_limit_plus_one_when_asked`: 30 groups of 2; `limit = 21` returns 21.

  Run them ONLY on your own throwaway container: `docker run -d --name cb-it-dev-$RANDOM -p 127.0.0.1::27017 mongo:7`, find the port with `docker port`, wait until it accepts connections, `CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:<port> CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_compare_store -- --include-ignored --nocapture`, then `docker rm -f -v` only that container. Never touch `local7979`, `local265` or `127.0.0.1:27017`. If a server result disagrees with the in-memory store, stop and report the disagreement; do not weaken the test.

- [ ] **Step 9: Gates and commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && scripts/check-no-db-in-ci.sh
  git add -A crates && git commit -m "feat(compare): add duplicate_keys to the store contract

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 2: Protocol and runtime: the `DuplicateKeys` command

**Files:**
- Modify: `crates/core/src/compare/protocol.rs`, `crates/core/src/runtime/mod.rs`, `crates/core/src/runtime/compare/mod.rs`
- Create: `crates/core/src/runtime/compare/duplicates_tests.rs` (declared `#[cfg(test)] mod duplicates_tests;` in `runtime/compare/mod.rs`)

**Interfaces:**
- Consumes: Task 1 (`DuplicateGroup`, `duplicate_keys`), the slice 1 session/handler machinery (`spawn_command`, `handle`, `validate_spec`, `connections`, `emit_if_open`, `answer_closed`, `command_context`, `command_session`).
- Produces:
  ```rust
  // compare/protocol.rs
  /// Most groups shown per side; the runtime asks the store for one more to learn whether there are more.
  pub const DUPLICATE_KEYS_LIMIT: u32 = 20;
  #[derive(Clone, Debug, PartialEq)]
  pub struct DuplicateReport { pub groups: Vec<DuplicateGroup>, pub more: bool }
  // CompareCommand:
  DuplicateKeys { session: SessionId, pair: PairSpec },
  // CompareEvent:
  DuplicateKeys { session: SessionId, pair: PairId, source: DuplicateReport, target: DuplicateReport },
  // runtime/mod.rs RuntimeConfig:
  pub duplicate_timeout: Duration,   // default DUPLICATE_TIMEOUT = 120 s
  ```
  Answer rule (add to the module doc in `runtime/compare/mod.rs`): `DuplicateKeys` → `DuplicateKeys` | `Failed { context: "compare_duplicates" }`.

- [ ] **Step 1: Failing offline runtime tests** in `duplicates_tests.rs`, using the existing helpers from `tests.rs` (`start`, `open`, `next_compare`, `pair`, `ns`, `seeded`-style stores built with `InMemoryStore::with_docs`). Tests:

  ```rust
  #[tokio::test]
  async fn both_sides_are_asked_and_each_reports_its_own_duplicates() {
      // source has key 1 twice (Int32 and Int64), target has 'a' three times, plus unique keys.
      // Start a session, send DuplicateKeys with key ["k"], expect one DuplicateKeys event whose
      // source.groups == [(1, 2)] and target.groups == [("a", 3)], more == false on both.
  }

  #[tokio::test]
  async fn more_is_set_only_when_there_are_more_groups_than_the_limit() {
      // 21 groups on the source => groups.len() == 20 && more; exactly 20 groups => 20 && !more.
  }

  #[tokio::test]
  async fn a_side_without_duplicates_reports_an_empty_report() { /* groups empty, more false */ }

  #[tokio::test]
  async fn an_invalid_key_or_rules_is_refused_before_any_store_call() {
      // PairSpec with key fields [""] built by struct literal (bypassing KeySpec::new): expect
      // Failed { context: "compare_duplicates", error: CoreError::CompareSetupInvalid }.
  }

  #[tokio::test]
  async fn unknown_or_closed_sessions_get_a_failed_answer() { /* session 99 and a closed session */ }

  #[tokio::test]
  async fn a_closed_session_stays_silent_for_an_in_flight_request() {
      // gated store blocking in duplicate_keys; send, Close, use a synchronous-answer barrier
      // (a Rows command for the closed session answers Failed at once), release the gate, then
      // assert nothing more arrives for the session.
  }

  #[tokio::test]
  async fn a_panicking_store_ends_in_the_commands_failed_event() { /* Failed compare_duplicates, Database("internal error") */ }

  #[tokio::test]
  async fn a_slow_aggregation_times_out_with_its_own_timeout() {
      // RuntimeConfig { duplicate_timeout: 50 ms, command_timeout: 5 s }, a store that never returns:
      // Failed { compare_duplicates, Database("timeout") } within ~1 s.
  }
  ```

  Fill every body with real assertions (follow `closed_tests.rs` and `abort_tests.rs` for the gated/panicking store doubles and the barrier technique). Run: `cargo test -p curlybrace-core --features test-support runtime::compare::duplicates` → FAIL to compile.

- [ ] **Step 2: Protocol additions.** Add the constant, `DuplicateReport`, the command and the event to `compare/protocol.rs` exactly as in the Interfaces block (import `DuplicateGroup` from `crate::store`). `CompareEvent` derives `PartialEq`, so `DuplicateReport` must too (it does).

- [ ] **Step 3: Runtime config.** In `runtime/mod.rs` add `pub const DUPLICATE_TIMEOUT: Duration = Duration::from_secs(120);`, the `duplicate_timeout` field on `RuntimeConfig` (with `Default`), and include it in the `Debug` impl; fix every `RuntimeConfig { .. }` literal the compiler flags.

- [ ] **Step 4: Handler.** In `runtime/compare/mod.rs`:
  - `command_session`: add the new variant.
  - `command_context`: `CompareCommand::DuplicateKeys { .. } => "compare_duplicates"`.
  - `handle`: a new arm delegating to a small function (keep `handle` under 50 lines):

  ```rust
  async fn find_duplicates(
      session: &Session,
      id: SessionId,
      pair: &PairSpec,
      timeout: Duration,
  ) -> Result<CompareEvent> {
      validate_spec(pair)?;
      let conns = connections(session)?;
      let ask = |store: Arc<dyn DocumentStore>, ns: &Namespace, filter: &Document| {
          let (ns, filter, key) = (ns.clone(), filter.clone(), pair.key.clone());
          async move {
              let mut groups = store
                  .duplicate_keys(&ns, &filter, &key, DUPLICATE_KEYS_LIMIT + 1)
                  .await?;
              let more = groups.len() > DUPLICATE_KEYS_LIMIT as usize;
              groups.truncate(DUPLICATE_KEYS_LIMIT as usize);
              Ok::<_, CoreError>(DuplicateReport { groups, more })
          }
      };
      let both = async {
          tokio::try_join!(
              ask(conns.source.clone(), &pair.source, &pair.source_filter),
              ask(conns.target.clone(), &pair.target, &pair.target_filter),
          )
      };
      let (source, target) = tokio::time::timeout(timeout, both)
          .await
          .map_err(|_| CoreError::Database("timeout".into()))??;
      Ok(CompareEvent::DuplicateKeys { session: id, pair: pair.id, source, target })
  }
  ```

  `handle`'s arm passes `shared.config.duplicate_timeout`. The event goes out through the existing single emit point (`spawn_command` → `emit_if_open`); panic containment and the closed-session `Failed` answer come for free.

- [ ] **Step 5: Keys never reach a log.** Check with a test in `crates/ui/src/error_text.rs` territory in Task 3; here add a core-side doc comment on `CompareEvent::DuplicateKeys`: "carries key values: show them, never log them".

- [ ] **Step 6: Run the tests and the gates; commit.**
  ```bash
  cargo test -p curlybrace-core --features test-support runtime::compare
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add the DuplicateKeys command to the compare protocol

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 3: UI state, reducer and view-model

**Files:**
- Create: `crates/ui/src/viewmodel/compare/duplicates.rs`, `crates/ui/src/app/compare_tests/duplicates.rs`
- Modify: `crates/ui/src/viewmodel/compare/mod.rs`, `crates/ui/src/app/compare.rs`, `crates/ui/src/app/compare_tests.rs` (module declaration), `crates/ui/src/error_text.rs`, `crates/ui/src/app/banner.rs`, `crates/ui/locales/en.json`, `crates/ui/src/i18n.rs` (usage/completeness lists)

**Interfaces:**
- Consumes: Task 2 protocol, `CompareState` (`check_in_flight`/`check_stale` pattern, `session_dropped`, `edited`), `RunView::Finished { failure }`, `Banner::Failure`.
- Produces:
  ```rust
  // viewmodel/compare/duplicates.rs
  #[derive(Clone, Debug, Default, PartialEq)]
  pub enum DuplicatesView {
      #[default] Idle,
      Loading { stale: bool },
      Loaded { source: DuplicateReport, target: DuplicateReport },
      Failed,                       // the banner carries the error; the panel offers Try again
  }
  impl DuplicatesView {
      pub fn can_find(&self, run_failed_on_duplicate_key: bool) -> bool;   // only after a CompareDuplicateKey failure, and not while loading
      pub fn rows(report: &DuplicateReport) -> Vec<DuplicateRow>;          // key text via the relaxed-JSON formatter, count
  }
  pub struct DuplicateRow { pub key_text: String, pub count: u64 }
  // CompareState gains: pub duplicates: DuplicatesView
  // CompareAction gains: FindDuplicates
  ```

- [ ] **Step 1: Failing view-model tests** (`duplicates.rs` test module): `can_find` is true only when the finished run's failure is `CompareDuplicateKey` and the view is not `Loading`; `rows` renders `Int32(1)` and a string key with the same formatter the key table uses and keeps count order; a compound key renders all values in field order; `DuplicatesView::Loaded` with empty reports is representable (the panel says "no duplicates found").

- [ ] **Step 2: Failing reducer tests** (`compare_tests/duplicates.rs`, helpers from the existing fixtures: `opened()`, `running_state()`, finish a run with `PairDone { outcome: Failed(CompareDuplicateKey) }` + `RunDone`):

  1. `find_duplicates_sends_the_command_for_the_current_pair` → exactly one `Command::Compare(CompareCommand::DuplicateKeys { session: 1, pair })` whose pair id is the workspace pair (1) and whose key/filters/rules are the form's; state is `Loading { stale: false }`.
  2. `it_is_refused_unless_the_run_failed_on_a_duplicate_key` (idle, completed run, other failure) → no command.
  3. `a_second_request_while_loading_sends_nothing`.
  4. `the_answer_populates_the_view` (matching session and pair) → `Loaded` with both reports.
  5. `an_edit_while_loading_marks_the_answer_stale_and_it_is_dropped` → view returns to `Idle`, no panel, user can ask again; mirror the one-Check-in-flight rule.
  6. `answers_for_another_session_or_pair_are_ignored`, `an_answer_with_nothing_requested_is_ignored`.
  7. `a_failed_request_clears_loading_and_shows_the_banner` (`Failed { context: "compare_duplicates" }` → `Banner::Failure { context: "compare_duplicates", .. }`, view `Failed`; a stale failure shows no banner).
  8. `restart_discard_close_and_session_drop_reset_the_panel` (Start, Discard, CloseSession, OpenFailed, opening another pair → `Idle`).
  9. `the_failure_log_line_never_contains_key_values`: build `Event::Compare(CompareEvent::DuplicateKeys {..})` with a key value like `"hunter2-secret"` and a `Failed { compare_duplicates, Database("E11000 dup key: hunter2-secret") }`; assert `failure_log_line`/`compare_log_line` output for both is only `compare_duplicates failed: <i18n key>` (and nothing for the success event) and that no `Debug` of state is used.

  Run: `cargo test -p curlybrace-ui --lib compare` → FAIL to compile.

- [ ] **Step 3: Implement.** `DuplicatesView` and `DuplicateRow` in the new file; `CompareState.duplicates` (reset by `session_dropped`, `edited` marks `Loading { stale: true }` or resets to `Idle`, `start`/`discard`); `CompareAction::FindDuplicates`; in `app/compare.rs` add `find_duplicates(state)` (guard with `can_find`, build the pair via `CompareState::pair()` exactly like Check does, send the command) and the two event arms (`DuplicateKeys` answer, `Failed { context: "compare_duplicates" }`) in the existing event dispatch, keeping `on_event` helpers under 50 lines. Add `compare_duplicates` to the failure contexts known by `banner.rs`/`error_text.rs` (`failure_log_line` logs context + error i18n key only). i18n keys needed by the view-model now: `failures.compare_duplicates`; the `compare.duplicates.*` keys for the view land in Task 4 together with the view.

- [ ] **Step 4: Gates and commit.**
  ```bash
  cargo fmt --all && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
  git add -A crates && git commit -m "feat(compare): add the duplicate-keys state and reducer

  Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
  ```

---

### Task 4: The panel, flows, snapshot and docs

**Files:**
- Create: `crates/ui/src/views/compare/duplicates.rs`
- Modify: `crates/ui/src/views/compare/mod.rs`, `crates/ui/src/views/compare/running.rs`, `crates/ui/locales/en.json`, `crates/ui/tests/local_compare_flows.rs`, `crates/ui/tests/snapshots.rs`, `crates/ui/tests/support/*`, `crates/ui/tests/accessibility.rs`, `README.md`, `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md`

**Interfaces:**
- Consumes: Task 3 (`DuplicatesView`, `DuplicateRow`, `CompareAction::FindDuplicates`), slice 1 views (`show_failure`, `show_run`).
- Produces: `show_duplicates(ui, &CompareState, &I18n) -> Option<CompareAction>`; no logic in views (the gate is `DuplicatesView::can_find`).

- [ ] **Step 1: Failing frame tests** (in `app/compare_tests/frames.rs`, pseudo-locale style used there) for: a run that failed on a duplicate key shows the **Find duplicate keys** button; `Loading` shows the loading text and a disabled button; `Loaded` lists a group per side with the key text and `count`, the "and more" note when `more`, and the "no duplicates on this side" text for an empty side; `Failed` shows Try again; a failed run with another error does NOT show the button. Every drawn string is an i18n key (the existing no-literal-English frame check).

- [ ] **Step 2: Implement the view.** The panel sits directly under the failure message in the finished state. Per side: heading `compare.duplicates.source|target`, then rows rendered as `"{key}"  ×{count}` via `i18n.t_with("compare.duplicates.row", ...)`; below the sides a hint with the next step ("Choose key fields that are unique, such as an email address or a combination of fields, then check again."). Stable ids via `ui.push_id`, accessible labels on the button, and extend `tests/accessibility.rs` so the finished-with-duplicate-failure state has every interactive node labelled.

- [ ] **Step 3: i18n keys** (all in `en.json`, nested, alphabetical; the usage test enforces both directions): `compare.duplicates.find`, `.loading`, `.retry`, `.source`, `.target`, `.row` (`{key}` `{count}`), `.more`, `.none_on_side`, `.hint`, `.title`.

- [ ] **Step 4: Tier-1 UI flow.** Add `finding_duplicate_keys_lists_the_colliding_values` to `crates/ui/tests/local_compare_flows.rs` (`#[ignore]`, env-gated, `cbit_*` only): two collections where the source has two documents with `email: "dup@example.com"`; compare keyed on `email` fails with the localized duplicate-key message; click **Find duplicate keys**; assert the panel lists `dup@example.com` with count 2 under Source and the "no duplicates" text under Target; then change the key to `_id`, check, start, and the compare completes. Run it ONLY against your own `cb-it-dev-*` container (same rules as Task 1) and remove the container afterwards.

- [ ] **Step 5: Snapshot.** Add `compare_duplicates_panel` to `crates/ui/tests/snapshots.rs` using the existing `Shot`/`shot!` machinery and a fixed in-memory state (two groups on the source, one "more", empty target). Generate with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored`, LOOK at the PNG (clipping, overlap, contrast) and fix layout problems before committing; confirm the 16 existing snapshots pass unchanged; never commit `*.new.png`.

- [ ] **Step 6: Docs.** README "Compare two collections": one sentence ("If a key is not unique the compare stops and **Find duplicate keys** shows which values repeat"). Spec: in the Errors section's slice 1b paragraph add an "as built" note (both sides are asked; `limit + 1` and `more`; missing-key documents excluded; own `duplicate_timeout`; the in-flight rule). Landing page `site/index.html`: extend the compare entry with "If the key is not unique, it shows which values repeat." Do not edit `CHANGELOG.md`.

- [ ] **Step 7: Whole-slice review and PR.** Run the full gates (`cargo fmt --all --check`, clippy, `cargo test --workspace --locked`, `scripts/check-no-db-in-ci.sh`, the tier-1 binaries `local_compare_store` and `local_compare_flows` on your own container, the snapshot suite). Then a whole-branch review on a stronger model focused on the Review Focus list above, fix findings, and open the PR against `main` (title `feat(compare): find duplicate keys (slice 1b)`, body ends with the Claude Code line, `Refs #8`).
