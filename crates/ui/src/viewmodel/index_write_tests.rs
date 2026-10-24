use super::*;
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;

fn named(names: &[&str]) -> Vec<CollectionIndex> {
    names
        .iter()
        .map(|name| CollectionIndex {
            spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
            size_bytes: None,
        })
        .collect()
}

#[test]
fn copy_name_never_collides() {
    // The first free version, starting at 2.
    assert_eq!(copy_name(&named(&["a_1"]), "a_1"), "a_1_v2");
    assert_eq!(copy_name(&named(&["a_1", "a_1_v2"]), "a_1"), "a_1_v3");
    // A taken version in the middle is skipped, never reused.
    assert_eq!(
        copy_name(&named(&["a_1", "a_1_v2", "a_1_v4"]), "a_1"),
        "a_1_v3"
    );
    // A base that already ends in _vN continues the sequence after N instead of stacking suffixes.
    assert_eq!(copy_name(&named(&["a_1_v2"]), "a_1_v2"), "a_1_v3");
    assert_eq!(copy_name(&named(&["a_1_v3", "a_1_v4"]), "a_1_v3"), "a_1_v5");
    // Only a real version suffix counts: digits after "_v", and something in front of it.
    assert_eq!(copy_name(&named(&["x_vip"]), "x_vip"), "x_vip_v2");
    assert_eq!(copy_name(&named(&["_v2"]), "_v2"), "_v2_v2");
    assert_eq!(copy_name(&named(&["a_v"]), "a_v"), "a_v_v2");
    // Dots, spaces and unicode are kept; the result is not in the list whatever the list holds.
    assert_eq!(copy_name(&named(&["é.x y"]), "é.x y"), "é.x y_v2");
    let crowded: Vec<String> = (2..60).map(|n| format!("k_v{n}")).collect();
    let refs: Vec<&str> = crowded.iter().map(String::as_str).collect();
    let existing = named(&refs);
    for base in ["k", "k_v5", "k_v70", "k_v99999999999999999999999"] {
        let picked = copy_name(&existing, base);
        assert!(existing.iter().all(|i| i.spec.name != picked), "{base}");
        assert_ne!(picked, base);
    }
}

#[test]
fn only_the_exact_name_confirms_a_drop() {
    let confirm = DropConfirm::new("My.Idx é".into());
    assert!(!confirm.confirmed(), "nothing typed");
    for wrong in ["my.idx é", "My.Idx e", " My.Idx é", "My.Idx é ", "My.Idx"] {
        assert!(
            !confirm.clone().with_typed(wrong.into()).confirmed(),
            "{wrong:?}"
        );
    }
    assert!(confirm.with_typed("My.Idx é".into()).confirmed());
}

#[test]
fn the_ttl_prompt_needs_a_whole_number_of_zero_or_more_of_seconds() {
    let prompt = TtlPrompt::new("t".into(), Some(100));
    assert_eq!(prompt.seconds(), Ok(100), "starts at the current expiry");
    let key = BuilderProblem::Index(IndexProblem::TtlNegative);
    for bad in ["", "abc", "1.5", "1e3", "+-4"] {
        assert_eq!(
            prompt.clone().with_text(bad.into()).seconds(),
            Err(BuilderProblem::TtlNotANumber),
            "{bad:?}"
        );
    }
    assert_eq!(
        prompt.clone().with_text("-1".into()).seconds(),
        Err(key.clone())
    );
    assert_eq!(
        BuilderProblem::TtlNotANumber.message_key(),
        "indexes.builder.problem_ttl_not_a_number"
    );
    assert_eq!(
        BuilderProblem::TtlTooLarge.message_key(),
        "indexes.builder.problem_ttl_too_large"
    );
    assert_eq!(prompt.clone().with_text(" 60 ".into()).seconds(), Ok(60));
    assert_eq!(prompt.with_text("0".into()).seconds(), Ok(0));
    assert_eq!(ttl_problem(0), None);
    assert_eq!(ttl_problem(-3), Some(key));
    assert_eq!(ttl_problem(1), None);
}

#[test]
fn a_whole_number_that_does_not_fit_is_too_large_not_a_non_number() {
    let prompt = TtlPrompt::new("t".into(), None);
    let max = MAX_TTL_SECONDS.to_string();
    let over = (MAX_TTL_SECONDS + 1).to_string();
    assert_eq!(MAX_TTL_SECONDS, i64::from(i32::MAX));
    for big in ["99999999999999999999", over.as_str(), "  9999999999  "] {
        assert_eq!(
            prompt.clone().with_text(big.into()).seconds(),
            Err(BuilderProblem::TtlTooLarge),
            "{big:?}"
        );
    }
    assert_eq!(
        prompt.clone().with_text(max).seconds(),
        Ok(MAX_TTL_SECONDS),
        "the largest the server accepts"
    );
    assert_eq!(
        prompt.with_text("-99999999999999999999".into()).seconds(),
        Err(BuilderProblem::Index(IndexProblem::TtlNegative)),
        "a huge negative is still negative"
    );
    assert_eq!(ttl_problem(MAX_TTL_SECONDS), None);
    assert_eq!(
        ttl_problem(MAX_TTL_SECONDS + 1),
        Some(BuilderProblem::TtlTooLarge)
    );
}

#[test]
fn only_the_id_index_is_special() {
    assert!(is_id_index("_id_"));
    assert!(!is_id_index("_id"));
    assert!(!is_id_index("x_id_"));
}
