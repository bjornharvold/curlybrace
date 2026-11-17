use bson::doc;

use super::*;
use crate::pipeline_options::{CollationOption, Strength};
use crate::store::Namespace;

fn keys(index: Option<SuggestedIndex>) -> Vec<(String, i32)> {
    index.expect("a suggestion").keys
}

fn pairs(list: &[(&str, i32)]) -> Vec<(String, i32)> {
    list.iter().map(|(f, d)| (f.to_string(), *d)).collect()
}

#[test]
fn a_leading_match_gives_equality_then_range() {
    let stages = [doc! { "$match": { "a": 1_i32, "b": { "$gt": 2_i32 } } }];
    assert_eq!(
        keys(suggest_for_pipeline(&stages)),
        pairs(&[("a", 1), ("b", 1)])
    );
}

#[test]
fn a_sort_after_the_match_goes_between() {
    let stages = [
        doc! { "$match": { "a": 1_i32, "c": { "$lt": 5_i32 } } },
        doc! { "$sort": { "b": -1_i32 } },
    ];
    assert_eq!(
        keys(suggest_for_pipeline(&stages)),
        pairs(&[("a", 1), ("b", -1), ("c", 1)])
    );
}

#[test]
fn a_leading_sort_alone_suggests_its_keys() {
    let stages = [doc! { "$sort": { "total": -1_i32, "name": 1_i32 } }];
    assert_eq!(
        keys(suggest_for_pipeline(&stages)),
        pairs(&[("total", -1), ("name", 1)])
    );
}

#[test]
fn a_sort_not_directly_after_the_match_is_ignored() {
    let stages = [
        doc! { "$match": { "a": 1_i32 } },
        doc! { "$addFields": { "x": 1_i32 } },
        doc! { "$sort": { "b": -1_i32 } },
    ];
    assert_eq!(keys(suggest_for_pipeline(&stages)), pairs(&[("a", 1)]));
}

#[test]
fn a_first_stage_that_is_neither_suggests_nothing() {
    for first in [
        doc! { "$group": { "_id": "$a" } },
        doc! { "$lookup": { "from": "x", "localField": "a", "foreignField": "b", "as": "c" } },
        doc! { "$geoNear": { "near": [0, 0], "distanceField": "d" } },
    ] {
        let stages = [first, doc! { "$sort": { "b": 1_i32 } }];
        assert_eq!(suggest_for_pipeline(&stages), None);
    }
    assert_eq!(suggest_for_pipeline(&[]), None);
}

#[test]
fn text_search_suggests_nothing() {
    let stages = [
        doc! { "$match": { "$text": { "$search": "x" }, "a": 1_i32 } },
        doc! { "$sort": { "b": 1_i32 } },
    ];
    assert_eq!(suggest_for_pipeline(&stages), None);
}

#[test]
fn an_empty_match_suggests_nothing() {
    assert_eq!(suggest_for_pipeline(&[doc! { "$match": {} }]), None);
    // A sort that is not a field list (`$natural`, a text score) gives nothing to index either.
    let stages = [
        doc! { "$match": {} },
        doc! { "$sort": { "$natural": 1_i32 } },
    ];
    assert_eq!(suggest_for_pipeline(&stages), None);
}

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn index() -> SuggestedIndex {
    SuggestedIndex {
        keys: pairs(&[("status", 1), ("total", -1)]),
    }
}

#[test]
fn the_command_without_collation_is_suggested_index_command() {
    assert_eq!(
        create_index_command(&index(), &ns(), None),
        index().command(&ns())
    );
}

fn collation(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.into(),
        strength: Some(Strength::Secondary),
        case_level: Some(true),
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

#[test]
fn the_command_carries_the_collation_escaped() {
    let text = create_index_command(&index(), &ns(), Some(&collation("en")));
    assert_eq!(
        text,
        "db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1, \"total\": -1 }, { collation: { locale: \"en\", strength: 2, caseLevel: true } })"
    );
    let hostile = create_index_command(&index(), &ns(), Some(&collation("e\"}); drop(); //")));
    assert!(
        hostile.contains(r#"locale: "e\"}); drop(); //""#),
        "{hostile}"
    );
    assert_eq!(hostile.matches("createIndex").count(), 1);
}
