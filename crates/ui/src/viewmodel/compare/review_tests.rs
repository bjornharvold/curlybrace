use super::*;
use bson::Bson;
use curlybrace_core::compare::key::KeyTuple;
use std::collections::BTreeMap;

use super::super::selection::RowAction as A;
use curlybrace_core::compare::results::Category;

const SRC: Category = Category::OnlyInSource;
const TGT: Category = Category::OnlyInTarget;
const DIFF: Category = Category::Different;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn keys(ns: &[i32]) -> Scope {
    Scope::Keys(ns.iter().map(|n| k(*n)).collect())
}

fn ns(db: &str, c: &str) -> Namespace {
    Namespace {
        db: db.into(),
        collection: c.into(),
    }
}

fn conns() -> ReviewConnections {
    ReviewConnections {
        source: (Uuid::from_u128(1), "src-conn".into()),
        target: (Uuid::from_u128(2), "tgt-conn".into()),
    }
}

fn entry(id: EntryId, source: Namespace, target: Namespace) -> ReviewEntry {
    ReviewEntry {
        entry: id,
        facts: RunFacts {
            source,
            target,
            key_has_id: true,
            include_id: true,
            scope_is_all: true,
        },
    }
}

/// Pair 1: a.x -> b.x; pair 2: c.y -> d.z.
fn entries() -> BTreeMap<PairId, ReviewEntry> {
    BTreeMap::from([
        (1, entry(10, ns("a", "x"), ns("b", "x"))),
        (2, entry(20, ns("c", "y"), ns("d", "z"))),
    ])
}

fn totals(_: PairId, cat: Category) -> usize {
    match cat {
        SRC => 5,
        TGT => 7,
        DIFF => 11,
    }
}

fn list(items: &[(PairId, Category, A, Scope)]) -> PendingList {
    items
        .iter()
        .fold(PendingList::default(), |l, (p, c, a, s)| {
            l.with_action(*p, *c, *a, s.clone()).0
        })
}

fn review_with(
    entries: &BTreeMap<PairId, ReviewEntry>,
    pending: &PendingList,
    policy: IdPolicy,
) -> Option<Review> {
    build_review(
        pending,
        policy,
        &conns(),
        &|p| entries.get(&p).cloned(),
        &totals,
    )
}

fn review(items: &[(PairId, Category, A, Scope)]) -> Review {
    review_with(&entries(), &list(items), IdPolicy::Keep).expect("a review")
}

fn dest(conn: &str, db: &str, c: &str) -> Destination {
    Destination {
        connection: conn.into(),
        database: db.into(),
        collection: c.into(),
    }
}

#[test]
fn destinations_follow_the_direction_in_both_pairs() {
    let r = review(&[
        (1, SRC, A::CopyToTarget, keys(&[1])),
        (1, TGT, A::CopyToSource, keys(&[2])),
        (2, SRC, A::CopyToTarget, keys(&[3])),
        (2, TGT, A::DeleteFromTarget, keys(&[4])),
    ]);
    assert_eq!(r.pairs.len(), 2);
    assert_eq!(r.pairs[0].pair, 1);
    assert_eq!(r.pairs[0].entry, 10);
    assert_eq!(
        r.pairs[0].destinations,
        vec![
            (A::CopyToTarget, dest("tgt-conn", "b", "x")),
            (A::CopyToSource, dest("src-conn", "a", "x")),
        ]
    );
    assert_eq!(
        r.pairs[1].destinations,
        vec![
            (A::CopyToTarget, dest("tgt-conn", "d", "z")),
            (A::DeleteFromTarget, dest("tgt-conn", "d", "z")),
        ]
    );
}

#[test]
fn delete_from_source_writes_the_source() {
    let r = review(&[(2, SRC, A::DeleteFromSource, keys(&[1]))]);
    assert_eq!(
        r.pairs[0].destinations,
        vec![(A::DeleteFromSource, dest("src-conn", "c", "y"))]
    );
}

#[test]
fn counts_equal_the_ops_the_command_would_send() {
    let pending = list(&[
        (1, SRC, A::CopyToTarget, keys(&[1, 2, 3])),
        (1, DIFF, A::CopyToTarget, Scope::Category),
        (1, TGT, A::DeleteFromTarget, keys(&[9])),
        (2, TGT, A::CopyToSource, Scope::Category),
        (2, DIFF, A::CopyToSource, keys(&[5, 6])),
    ]);
    let r = review_with(&entries(), &pending, IdPolicy::Keep).unwrap();
    let (ops, bulk) = pending.to_command_parts();
    let bulk_n = |pair, kind| {
        bulk.iter()
            .filter(|b| b.pair == pair && b.kind == kind)
            .map(|b| totals(b.pair, b.category) as u64)
            .sum::<u64>()
    };
    for p in &r.pairs {
        let explicit = |kind| {
            ops.iter()
                .filter(|o| o.pair == p.pair && o.kind == kind)
                .count() as u64
        };
        let expected = |kind| explicit(kind) + bulk_n(p.pair, kind);
        assert_eq!(p.counts.insert, expected(ApplyKind::Insert));
        assert_eq!(p.counts.replace, expected(ApplyKind::Replace));
        assert_eq!(p.counts.delete, expected(ApplyKind::Delete));
    }
    assert_eq!(
        r.pairs[0].counts,
        KindCounts {
            insert: 3,
            replace: 11,
            delete: 1
        }
    );
    assert_eq!(
        r.pairs[1].counts,
        KindCounts {
            insert: 7,
            replace: 2,
            delete: 0
        }
    );
    assert_eq!(r.op_total(), 24);
    assert_eq!(r.op_total(), pending.op_count(&totals));
}

#[test]
fn empty_pending_or_unknown_pair_gives_no_review() {
    assert!(review_with(&entries(), &PendingList::default(), IdPolicy::Keep).is_none());
    let pending = list(&[(99, SRC, A::CopyToTarget, keys(&[1]))]);
    assert!(review_with(&entries(), &pending, IdPolicy::Keep).is_none());
}

fn required(items: &[(PairId, Category, A, Scope)]) -> Option<RequiredText> {
    review(items).required
}

#[test]
fn required_text_follows_the_deletes() {
    let lit = |s: &str| Some(RequiredText::Literal(s.into()));
    assert_eq!(required(&[(1, SRC, A::CopyToTarget, keys(&[1]))]), None);
    // one collection (two items, same destination)
    assert_eq!(
        required(&[
            (1, TGT, A::DeleteFromTarget, keys(&[1])),
            (1, SRC, A::CopyToTarget, keys(&[2])),
        ]),
        lit("x")
    );
}

#[test]
fn several_collections_of_one_database_need_the_database() {
    let mut e = entries();
    e.insert(3, entry(30, ns("a", "x"), ns("b", "other")));
    let pending = list(&[
        (1, TGT, A::DeleteFromTarget, keys(&[1])),
        (3, TGT, A::DeleteFromTarget, keys(&[2])),
    ]);
    let r = review_with(&e, &pending, IdPolicy::Keep).unwrap();
    assert_eq!(r.required, Some(RequiredText::Literal("b".into())));
}

#[test]
fn several_databases_or_both_connections_need_the_word() {
    let both_db = review(&[
        (1, TGT, A::DeleteFromTarget, keys(&[1])),
        (2, TGT, A::DeleteFromTarget, keys(&[2])),
    ]);
    assert_eq!(both_db.required, Some(RequiredText::Word));
    let both_conn = review(&[
        (1, TGT, A::DeleteFromTarget, keys(&[1])),
        (1, SRC, A::DeleteFromSource, keys(&[2])),
    ]);
    assert_eq!(both_conn.required, Some(RequiredText::Word));
}

#[test]
fn same_names_on_two_connections_are_still_two_places() {
    let mut e = entries();
    e.insert(3, entry(30, ns("b", "x"), ns("a", "x")));
    let pending = list(&[
        (1, TGT, A::DeleteFromTarget, keys(&[1])),
        (3, SRC, A::DeleteFromSource, keys(&[2])),
    ]);
    let r = review_with(&e, &pending, IdPolicy::Keep).unwrap();
    assert_eq!(r.required, Some(RequiredText::Word));
}

#[test]
fn confirmation_is_exact_and_trimmed() {
    let r = review(&[(1, TGT, A::DeleteFromTarget, keys(&[1]))]);
    let typed = |t: &str| r.clone().with_confirm_text(t).confirm_enabled("DELETE");
    assert!(typed("x"));
    assert!(typed("  x\n"));
    assert!(!typed("X"));
    assert!(!typed(""));
    assert!(!typed("xx"));
    assert_eq!(r.required_text("DELETE"), Some("x"));
    let word = review(&[
        (1, TGT, A::DeleteFromTarget, keys(&[1])),
        (2, TGT, A::DeleteFromTarget, keys(&[2])),
    ]);
    assert_eq!(word.required_text("DELETE"), Some("DELETE"));
    assert!(
        word.clone()
            .with_confirm_text("DELETE")
            .confirm_enabled("DELETE")
    );
    assert!(!word.with_confirm_text("delete").confirm_enabled("DELETE"));
}

#[test]
fn no_delete_needs_no_confirmation() {
    let r = review(&[(1, SRC, A::CopyToTarget, keys(&[1]))]);
    assert!(r.confirm_enabled("DELETE"));
    assert_eq!(r.required_text("DELETE"), None);
}

fn with_entry(mutate: impl Fn(&mut ReviewEntry)) -> BTreeMap<PairId, ReviewEntry> {
    let mut e = entries();
    e.values_mut().for_each(mutate);
    e
}

fn warnings(
    e: &BTreeMap<PairId, ReviewEntry>,
    items: &[(PairId, Category, A, Scope)],
    policy: IdPolicy,
) -> Vec<Warning> {
    review_with(e, &list(items), policy).unwrap().warnings
}

#[test]
fn a_plain_id_keyed_full_scope_replace_warns_about_nothing() {
    let w = warnings(
        &entries(),
        &[(1, DIFF, A::CopyToTarget, keys(&[1]))],
        IdPolicy::Keep,
    );
    assert_eq!(w, vec![]);
}

#[test]
fn generated_ids_warn_only_with_generate_and_an_insert() {
    let ins = [(1, SRC, A::CopyToTarget, keys(&[1]))];
    let del = [(1, TGT, A::DeleteFromTarget, keys(&[1]))];
    assert_eq!(
        warnings(&entries(), &ins, IdPolicy::Generate),
        vec![Warning::GeneratedIds]
    );
    assert_eq!(warnings(&entries(), &ins, IdPolicy::Keep), vec![]);
    assert_eq!(warnings(&entries(), &del, IdPolicy::Generate), vec![]);
}

#[test]
fn changing_the_policy_recomputes_the_warnings_and_keeps_the_typing() {
    let r = review(&[(1, SRC, A::CopyToTarget, keys(&[1]))]).with_confirm_text("abc");
    let g = r.with_id_policy(IdPolicy::Generate);
    assert_eq!(g.warnings, vec![Warning::GeneratedIds]);
    assert_eq!(g.confirm_text, "abc");
    assert_eq!(g.id_policy, IdPolicy::Generate);
}

#[test]
fn a_replace_with_include_id_and_a_business_key_warns() {
    let e = with_entry(|e| {
        e.facts.key_has_id = false;
        e.facts.include_id = true;
    });
    let w = warnings(
        &e,
        &[(1, DIFF, A::CopyToTarget, keys(&[1]))],
        IdPolicy::Keep,
    );
    assert_eq!(w, vec![Warning::KeepsDestinationId]);
    let e = with_entry(|e| {
        e.facts.key_has_id = false;
        e.facts.include_id = false;
    });
    let w = warnings(
        &e,
        &[(1, DIFF, A::CopyToTarget, keys(&[1]))],
        IdPolicy::Keep,
    );
    assert_eq!(w, vec![]);
}

#[test]
fn a_replace_under_a_narrow_field_scope_warns() {
    let e = with_entry(|e| e.facts.scope_is_all = false);
    let w = warnings(
        &e,
        &[(1, DIFF, A::CopyToTarget, keys(&[1]))],
        IdPolicy::Keep,
    );
    assert_eq!(w, vec![Warning::WholeDocumentOverwrite]);
    let w = warnings(&e, &[(1, SRC, A::CopyToTarget, keys(&[1]))], IdPolicy::Keep);
    assert_eq!(w, vec![], "an insert is not a replace");
}

#[test]
fn an_insert_under_a_business_key_may_duplicate_it() {
    let e = with_entry(|e| e.facts.key_has_id = false);
    let w = warnings(
        &e,
        &[(1, SRC, A::CopyToTarget, keys(&[1]))],
        IdPolicy::Generate,
    );
    assert_eq!(
        w,
        vec![Warning::GeneratedIds, Warning::MaybeDuplicateBusinessKey]
    );
}

#[test]
fn every_warning_key_is_a_compare_apply_key() {
    for w in [
        Warning::GeneratedIds,
        Warning::KeepsDestinationId,
        Warning::WholeDocumentOverwrite,
        Warning::MaybeDuplicateBusinessKey,
    ] {
        assert!(w.i18n_key().starts_with("compare.apply.warning."));
    }
}

#[test]
fn a_name_with_spaces_is_typed_trimmed_and_a_blank_name_needs_the_word() {
    let mut e = entries();
    e.insert(3, entry(30, ns("a", "x"), ns("b", " x ")));
    e.insert(4, entry(40, ns("a", "x"), ns("b", "  ")));
    let one = |pair| {
        review_with(
            &e,
            &list(&[(pair, TGT, A::DeleteFromTarget, keys(&[1]))]),
            IdPolicy::Keep,
        )
        .unwrap()
    };
    let spaced = one(3);
    assert_eq!(spaced.required, Some(RequiredText::Literal(" x ".into())));
    assert!(
        spaced
            .clone()
            .with_confirm_text("x")
            .confirm_enabled("DELETE")
    );
    assert!(
        spaced
            .clone()
            .with_confirm_text(" x ")
            .confirm_enabled("DELETE")
    );
    assert!(!spaced.with_confirm_text("").confirm_enabled("DELETE"));
    let blank = one(4);
    assert_eq!(blank.required, Some(RequiredText::Word));
    assert!(
        !blank
            .clone()
            .with_confirm_text("")
            .confirm_enabled("DELETE")
    );
    assert!(blank.with_confirm_text("DELETE").confirm_enabled("DELETE"));
}

/// A small deterministic generator (LCG) so the property test needs no dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

fn random_list(rng: &mut Lcg) -> PendingList {
    let cats = [SRC, TGT, DIFF];
    (0..rng.next(8) + 1).fold(PendingList::default(), |list, _| {
        let pair = rng.next(3) as PairId + 1;
        let cat = cats[rng.next(3) as usize];
        let offered = A::offered_for(cat);
        let action = offered[rng.next(offered.len() as u64) as usize];
        let scope = match rng.next(3) {
            0 => Scope::Category,
            _ => keys(&(0..rng.next(5) as i32 + 1).collect::<Vec<_>>()),
        };
        list.with_action(pair, cat, action, scope).0
    })
}

#[test]
fn counts_match_the_command_ops_for_random_pending_lists() {
    let mut e = entries();
    e.insert(3, entry(30, ns("e", "p"), ns("f", "q")));
    let mut rng = Lcg(42);
    for case in 0..200 {
        let pending = random_list(&mut rng);
        let r = review_with(&e, &pending, IdPolicy::Keep).expect("a review");
        let (ops, bulk) = pending.to_command_parts();
        for kind in [ApplyKind::Insert, ApplyKind::Replace, ApplyKind::Delete] {
            let from_ops = ops.iter().filter(|o| o.kind == kind).count() as u64;
            let from_bulk: u64 = bulk
                .iter()
                .filter(|b| b.kind == kind)
                .map(|b| totals(b.pair, b.category) as u64)
                .sum();
            let counted: u64 = r
                .pairs
                .iter()
                .map(|p| match kind {
                    ApplyKind::Insert => p.counts.insert,
                    ApplyKind::Replace => p.counts.replace,
                    ApplyKind::Delete => p.counts.delete,
                    ApplyKind::Fields => 0, // slice 2c
                })
                .sum();
            assert_eq!(counted, from_ops + from_bulk, "case {case} {kind:?}");
        }
        assert_eq!(r.op_total(), pending.op_count(&totals), "case {case}");
    }
}

#[test]
fn every_warning_key_exists_in_en_json() {
    let root: serde_json::Value =
        serde_json::from_str(include_str!("../../../locales/en.json")).unwrap();
    for w in [
        Warning::GeneratedIds,
        Warning::KeepsDestinationId,
        Warning::WholeDocumentOverwrite,
        Warning::MaybeDuplicateBusinessKey,
    ] {
        let pointer = format!("/{}", w.i18n_key().replace('.', "/"));
        assert!(
            root.pointer(&pointer).is_some_and(|v| v.is_string()),
            "{} missing",
            w.i18n_key()
        );
    }
}
