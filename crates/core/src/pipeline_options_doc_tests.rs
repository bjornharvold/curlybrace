use bson::{Bson, doc};

use crate::pipeline_options::{Alternate, CaseFirst, CollationOption, HintOption, Strength};

fn only_locale(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.into(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

#[test]
fn a_collation_document_has_only_the_set_fields_in_mongodb_spelling() {
    assert_eq!(only_locale("en").to_document(), doc! { "locale": "en" });
    let full = CollationOption {
        strength: Some(Strength::Secondary),
        case_level: Some(false),
        case_first: Some(CaseFirst::Upper),
        numeric_ordering: Some(true),
        alternate: Some(Alternate::NonIgnorable),
        ..only_locale("fr_CA")
    };
    assert_eq!(
        full.to_document(),
        doc! {
            "locale": "fr_CA",
            "strength": 2_i32,
            "caseLevel": false,
            "caseFirst": "upper",
            "numericOrdering": true,
            "alternate": "non-ignorable",
        }
    );
    let partial = CollationOption {
        case_first: Some(CaseFirst::Off),
        alternate: Some(Alternate::Shifted),
        ..only_locale("simple")
    };
    assert_eq!(
        partial.to_document(),
        doc! { "locale": "simple", "caseFirst": "off", "alternate": "shifted" }
    );
}

#[test]
fn strength_is_an_int32() {
    for (strength, n) in [
        (Strength::Primary, 1),
        (Strength::Tertiary, 3),
        (Strength::Identical, 5),
    ] {
        let doc = CollationOption {
            strength: Some(strength),
            ..only_locale("en")
        }
        .to_document();
        assert_eq!(doc.get("strength"), Some(&Bson::Int32(n)));
    }
}

#[test]
fn a_hint_is_a_string_or_its_keys() {
    assert_eq!(
        HintOption::Name("status_1".into()).to_bson(),
        Bson::String("status_1".into())
    );
    let keys = doc! { "status": 1_i32, "total": -1_i32 };
    assert_eq!(
        HintOption::Keys(keys.clone()).to_bson(),
        Bson::Document(keys)
    );
}
