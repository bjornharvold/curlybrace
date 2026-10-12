use super::*;
use bson::doc;
use curlybrace_core::compare::rules::FieldScope;
use curlybrace_core::store::CollectionKind;

fn picked(db: &str, coll: &str) -> SideForm {
    SideForm::default()
        .with_database(db)
        .with_collections(
            db,
            vec![
                (coll.to_string(), CollectionKind::Collection),
                ("v".to_string(), CollectionKind::View),
            ],
        )
        .with_collection(coll)
}

fn ready_form() -> SetupForm {
    SetupForm::default()
        .with_side(Side::Source, picked("a", "users"))
        .with_side(Side::Target, picked("b", "users"))
}

#[test]
fn default_key_is_id_and_comma_separated_paths_parse() {
    let form = SetupForm::default();
    assert_eq!(form.key().unwrap(), KeySpec::id());
    let form = form.with_key_text("tenant, address.zip");
    assert_eq!(
        form.key().unwrap().fields,
        vec!["tenant".to_string(), "address.zip".to_string()]
    );
}

#[test]
fn bad_keys_map_to_an_i18n_key_not_english() {
    for text in ["", "a,,b", "a..b", "a, a", " , "] {
        let err = SetupForm::default().with_key_text(text).key().unwrap_err();
        assert_eq!(err, SetupError::Key, "{text:?}");
        assert_eq!(err.i18n_key(), "compare.setup.key_invalid");
    }
}

#[test]
fn filter_text_empty_is_an_empty_document_and_json_objects_parse() {
    let form = SetupForm::default();
    assert_eq!(form.filter(Side::Source).unwrap(), doc! {});
    let form = form.with_side(Side::Source, SideForm::default().with_filter("  "));
    assert_eq!(form.filter(Side::Source).unwrap(), doc! {});
    let form = form.with_side(Side::Target, SideForm::default().with_filter(r#"{"a": 1}"#));
    assert_eq!(form.filter(Side::Target).unwrap(), doc! {"a": 1});
}

#[test]
fn invalid_filters_report_the_side_and_an_i18n_key() {
    for text in ["{not json", "[1]", "3"] {
        let form =
            SetupForm::default().with_side(Side::Target, SideForm::default().with_filter(text));
        let err = form.filter(Side::Target).unwrap_err();
        assert_eq!(err, SetupError::Filter(Side::Target), "{text:?}");
        assert_eq!(err.i18n_key(), "compare.setup.filter_invalid");
    }
}

#[test]
fn rules_defaults_equal_the_core_defaults_and_follow_the_key() {
    let form = SetupForm::default();
    assert_eq!(form.rules().unwrap(), CompareRules::for_key(&KeySpec::id()));
    let form = form.with_key_text("sku");
    let rules = form.rules().unwrap();
    assert_eq!(
        rules,
        CompareRules::for_key(&KeySpec::new(vec!["sku".into()]).unwrap())
    );
    assert!(!rules.include_id);
}

#[test]
fn explicit_include_id_survives_a_key_change() {
    let rules = RulesForm {
        include_id: Some(true),
        ..RulesForm::default()
    };
    let form = SetupForm::default().with_rules(rules).with_key_text("sku");
    assert!(form.rules().unwrap().include_id);
}

#[test]
fn field_scope_paths_come_from_comma_separated_text() {
    let rules = RulesForm {
        scope: ScopeKind::AllExcept,
        paths_text: "updatedAt, meta.v".into(),
        ..RulesForm::default()
    };
    let form = SetupForm::default().with_rules(rules);
    assert_eq!(
        form.rules().unwrap().field_scope,
        FieldScope::AllExcept(vec!["updatedAt".into(), "meta.v".into()])
    );
}

#[test]
fn invalid_rules_map_to_an_i18n_key() {
    for scope in [ScopeKind::Only, ScopeKind::AllExcept] {
        let rules = RulesForm {
            scope,
            paths_text: "a..b".into(),
            ..RulesForm::default()
        };
        let err = SetupForm::default().with_rules(rules).rules().unwrap_err();
        assert_eq!(err, SetupError::Rules);
        assert_eq!(err.i18n_key(), "compare.setup.rules_invalid");
    }
    let empty_only = RulesForm {
        scope: ScopeKind::Only,
        ..RulesForm::default()
    };
    assert!(SetupForm::default().with_rules(empty_only).rules().is_err());
}

#[test]
fn ready_needs_both_sides_picked_and_no_views_and_valid_inputs() {
    assert!(ready_form().ready());
    assert!(!SetupForm::default().ready());
    let half = SetupForm::default().with_side(Side::Source, picked("a", "users"));
    assert_eq!(half.issues(), vec![SetupError::Incomplete]);

    let view = ready_form().with_side(Side::Target, picked("b", "users").with_collection("v"));
    assert_eq!(view.issues(), vec![SetupError::ViewSelected(Side::Target)]);
    assert!(!view.ready());

    assert!(!ready_form().with_key_text("").ready());
    let bad_filter =
        ready_form().with_side(Side::Source, picked("a", "users").with_filter("{oops"));
    assert!(!bad_filter.ready());
}

#[test]
fn pair_carries_namespaces_filters_key_and_rules_with_the_fixed_id() {
    let form = ready_form().with_key_text("sku").with_side(
        Side::Source,
        picked("a", "users").with_filter(r#"{"x": 2}"#),
    );
    let pair = form.pair().unwrap();
    assert_eq!(pair.id, PAIR_ID);
    assert_eq!(pair.source, Namespace::new("a", "users"));
    assert_eq!(pair.target, Namespace::new("b", "users"));
    assert_eq!(pair.source_filter, doc! {"x": 2});
    assert_eq!(pair.target_filter, doc! {});
    assert_eq!(pair.key.fields, vec!["sku".to_string()]);
}

#[test]
fn picking_a_database_clears_the_collection_and_its_listing() {
    let side = picked("a", "users").with_database("other");
    assert_eq!(side.picked(), None);
    assert!(side.collections.is_empty());
    assert_eq!(side.collection, None);
}

#[test]
fn collection_listings_for_another_database_are_dropped() {
    let side = SideForm::default()
        .with_database("a")
        .with_collections("zzz", vec![("c".into(), CollectionKind::Collection)]);
    assert!(side.collections.is_empty());
}
