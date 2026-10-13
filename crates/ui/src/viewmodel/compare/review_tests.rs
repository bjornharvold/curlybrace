use super::*;
use bson::Bson;
use curlybrace_core::compare::diff::PathSegment;
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
            delete: 1,
            fields: 0
        }
    );
    assert_eq!(
        r.pairs[1].counts,
        KindCounts {
            insert: 7,
            replace: 2,
            delete: 0,
            fields: 0
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
                    ApplyKind::Fields => p.counts.fields,
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
        Warning::FieldCopyKeepsRest,
    ] {
        let pointer = format!("/{}", w.i18n_key().replace('.', "/"));
        assert!(
            root.pointer(&pointer).is_some_and(|v| v.is_string()),
            "{} missing",
            w.i18n_key()
        );
    }
}

fn fpaths(names: &[&str]) -> Vec<Vec<PathSegment>> {
    names
        .iter()
        .map(|n| vec![PathSegment::Field((*n).into())])
        .collect()
}

fn with_fields(list: &PendingList, pair: PairId, key: i32, names: &[&str]) -> PendingList {
    list.with_fields(pair, &k(key), A::CopyFieldsToTarget, fpaths(names))
        .0
}

#[test]
fn fields_items_count_documents_and_fields_separately() {
    let l = with_fields(&PendingList::default(), 1, 1, &["a", "b"]);
    let l = with_fields(&l, 1, 2, &["c"]);
    let r = review_with(&entries(), &l, IdPolicy::Keep).unwrap();
    assert_eq!(
        r.pairs[0].counts,
        KindCounts {
            fields: 2,
            ..KindCounts::default()
        }
    );
    assert_eq!(r.pairs[0].field_count, 3);
    assert_eq!(r.field_total(), 3);
    assert_eq!(r.op_total(), 2);
    assert_eq!(r.op_total(), l.op_count(&totals));
    assert_eq!(
        r.pairs[0].destinations,
        vec![(A::CopyFieldsToTarget, dest("tgt-conn", "b", "x"))]
    );
}

#[test]
fn fields_toward_the_source_write_the_source() {
    let l = PendingList::default()
        .with_fields(2, &k(1), A::CopyFieldsToSource, fpaths(&["a"]))
        .0;
    let r = review_with(&entries(), &l, IdPolicy::Keep).unwrap();
    assert_eq!(
        r.pairs[0].destinations,
        vec![(A::CopyFieldsToSource, dest("src-conn", "c", "y"))]
    );
}

#[test]
fn the_field_warning_is_present_exactly_when_a_fields_item_is() {
    let whole = review(&[(1, DIFF, A::CopyToTarget, keys(&[1]))]);
    assert!(!whole.warnings.contains(&Warning::FieldCopyKeepsRest));
    let only = with_fields(&PendingList::default(), 1, 1, &["a"]);
    let r = review_with(&entries(), &only, IdPolicy::Keep).unwrap();
    assert_eq!(r.warnings, vec![Warning::FieldCopyKeepsRest]);
    let (mixed, _) = only.with_action(1, SRC, A::CopyToTarget, keys(&[5]));
    let r = review_with(&entries(), &mixed, IdPolicy::Generate).unwrap();
    assert!(r.warnings.contains(&Warning::FieldCopyKeepsRest));
    assert!(r.warnings.contains(&Warning::GeneratedIds));
    // removing the fields item removes the warning
    let r = review_with(&entries(), &mixed.without(0), IdPolicy::Keep).unwrap();
    assert!(!r.warnings.contains(&Warning::FieldCopyKeepsRest));
}

#[test]
fn field_only_reviews_never_need_typing() {
    let l = with_fields(&PendingList::default(), 1, 1, &["a"]);
    let r = review_with(&entries(), &l, IdPolicy::Keep).unwrap();
    assert_eq!(r.required, None);
    assert!(r.confirm_enabled("DELETE"));
    // deletes elsewhere decide the text exactly as before
    let (both, _) = l.with_action(1, TGT, A::DeleteFromTarget, keys(&[1]));
    let r = review_with(&entries(), &both, IdPolicy::Keep).unwrap();
    assert_eq!(r.required, Some(RequiredText::Literal("x".into())));
}

#[test]
fn reviews_with_other_paths_do_not_stand_for_the_same_writes() {
    let a = review_with(
        &entries(),
        &with_fields(&PendingList::default(), 1, 1, &["a"]),
        IdPolicy::Keep,
    )
    .unwrap();
    let b = review_with(
        &entries(),
        &with_fields(&PendingList::default(), 1, 1, &["b"]),
        IdPolicy::Keep,
    )
    .unwrap();
    assert!(a.same_writes(&a.clone()));
    assert!(!a.same_writes(&b));
}

fn random_mixed_list(rng: &mut Lcg) -> PendingList {
    let names = ["a", "b", "c", "d"];
    (0..rng.next(8) + 1).fold(PendingList::default(), |list, _| {
        let pair = rng.next(3) as PairId + 1;
        if rng.next(2) == 0 {
            let key = rng.next(4) as i32;
            let action = if rng.next(2) == 0 {
                A::CopyFieldsToTarget
            } else {
                A::CopyFieldsToSource
            };
            let paths = fpaths(&names[..rng.next(4) as usize + 1]);
            return list.with_fields(pair, &k(key), action, paths).0;
        }
        let cat = [SRC, TGT, DIFF][rng.next(3) as usize];
        let offered = A::offered_for(cat);
        let action = offered[rng.next(offered.len() as u64) as usize];
        let scope = match rng.next(3) {
            0 => Scope::Category,
            _ => keys(&(0..rng.next(4) as i32 + 1).collect::<Vec<_>>()),
        };
        list.with_action(pair, cat, action, scope).0
    })
}

#[test]
fn counts_match_the_command_ops_with_field_items_mixed_in() {
    let mut e = entries();
    e.insert(3, entry(30, ns("e", "p"), ns("f", "q")));
    let mut rng = Lcg(7);
    let mut saw_fields = 0;
    for case in 0..200 {
        let pending = random_mixed_list(&mut rng);
        let r = review_with(&e, &pending, IdPolicy::Keep).expect("a review");
        let (ops, bulk) = pending.to_command_parts();
        let counted =
            |f: &dyn Fn(&KindCounts) -> u64| -> u64 { r.pairs.iter().map(|p| f(&p.counts)).sum() };
        let from_ops = |kind: ApplyKind| ops.iter().filter(|o| o.kind == kind).count() as u64;
        let from_bulk = |kind: ApplyKind| -> u64 {
            bulk.iter()
                .filter(|b| b.kind == kind)
                .map(|b| totals(b.pair, b.category) as u64)
                .sum()
        };
        let want = |kind| from_ops(kind) + from_bulk(kind);
        assert_eq!(counted(&|c| c.insert), want(ApplyKind::Insert), "{case}");
        assert_eq!(counted(&|c| c.replace), want(ApplyKind::Replace), "{case}");
        assert_eq!(counted(&|c| c.delete), want(ApplyKind::Delete), "{case}");
        assert_eq!(counted(&|c| c.fields), want(ApplyKind::Fields), "{case}");
        let sent_fields: u64 = ops.iter().map(|o| o.fields.len() as u64).sum();
        assert_eq!(r.field_total(), sent_fields, "case {case}");
        assert!(bulk.iter().all(|b| b.kind != ApplyKind::Fields));
        assert_eq!(r.op_total(), pending.op_count(&totals), "case {case}");
        assert_eq!(
            r.op_total(),
            ops.len() as u64
                + bulk
                    .iter()
                    .map(|b| totals(b.pair, b.category) as u64)
                    .sum::<u64>(),
            "case {case}"
        );
        let has_fields = pending
            .items()
            .iter()
            .any(|c| matches!(c.scope, Scope::Fields { .. }));
        assert_eq!(
            r.warnings.contains(&Warning::FieldCopyKeepsRest),
            has_fields,
            "case {case}"
        );
        saw_fields += usize::from(has_fields);
    }
    assert!(saw_fields > 20, "the generator must exercise fields items");
}
