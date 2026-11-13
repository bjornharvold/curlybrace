use super::*;
use crate::viewmodel::aggregate::STAGE_MENU;

fn labels(text: &str, forced: bool) -> Vec<String> {
    complete_stage(text, text.len(), forced)
        .map(|c| c.items.into_iter().map(|s| s.label).collect())
        .unwrap_or_default()
}

#[test]
fn a_top_level_key_offers_stage_names() {
    assert_eq!(labels("{ \"$m", false), ["$match", "$merge"]);
    assert_eq!(labels("{ $m", false), ["$match", "$merge"]);
    let c = complete_stage("{ \"$m", 5, false).unwrap();
    assert_eq!(c.replace, 3..5, "the quote stays");
}

#[test]
fn an_empty_stage_text_offers_stage_names_on_demand() {
    assert_eq!(labels("", false), Vec::<String>::new());
    assert_eq!(labels("  \n", true).len(), MAX_ITEMS);
    assert_eq!(
        labels("$s", false),
        [
            "$sort",
            "$skip",
            "$sample",
            "$set",
            "$sortByCount",
            "$setWindowFields"
        ]
    );
}

#[test]
fn a_body_key_offers_operators() {
    let text = "{ \"$match\": { \"$g";
    let found = labels(text, false);
    assert!(found.contains(&"$gt".to_string()) && !found.contains(&"$merge".to_string()));
    assert!(labels("{ \"$match\": { \"a\": 1, $e", false).contains(&"$eq".to_string()));
    assert!(labels("{ \"$match\": { \"$and\": [ { \"$o", false).contains(&"$or".to_string()));
}

#[test]
fn nothing_inside_other_strings_or_values() {
    assert!(labels("{ \"$group\": { \"_id\": \"$m", false).is_empty());
    assert!(labels("{ \"$match\": { \"a\": \"x $m", false).is_empty());
    assert!(labels("{ \"$match\": { \"a\": $m", false).is_empty());
    assert!(labels("{ \"$match\": {} } $m", false).is_empty());
    assert!(labels("{ \"a $m", false).is_empty());
    assert!(labels("{ \"$limit\": [ $m", false).is_empty());
    assert!(labels("{ \"x\": \"\\\" $m", false).is_empty());
}

#[test]
fn the_stage_names_are_real_operators_and_cover_the_menu() {
    for name in STAGE_MENU {
        assert!(STAGE_NAMES.contains(&name), "{name}");
    }
    for name in STAGE_NAMES {
        assert!(OPERATORS.contains(name), "{name} is not an operator");
    }
    let mut sorted = STAGE_NAMES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), STAGE_NAMES.len());
}
