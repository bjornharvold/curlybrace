use super::*;

fn colls(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// Completes with the caret at the end of `text`.
fn at_end(text: &str, collections: &[&str]) -> Option<Completion> {
    complete(text, text.len(), &colls(collections))
}

fn labels(c: &Option<Completion>) -> Vec<&str> {
    c.as_ref()
        .map(|c| c.items.iter().map(|s| s.label.as_str()).collect())
        .unwrap_or_default()
}

// --- the five places of the spec table ---------------------------------------------------

#[test]
fn a_word_at_the_start_of_a_statement_offers_statement_words() {
    let c = at_end("pri", &[]);
    assert_eq!(labels(&c), ["print", "printjson"]);
    assert_eq!(c.as_ref().unwrap().replace, 0..3);
    assert_eq!(c.unwrap().items[0].kind, SuggestionKind::Word);
    assert!(labels(&at_end("d", &[])).contains(&"db"));
    assert!(labels(&at_end("x = 1;\n  Obj", &[])).contains(&"ObjectId"));
    assert!(labels(&at_end("f();Iso", &[])).contains(&"ISODate"));
    assert!(labels(&at_end("if (a) {\n  pri", &[])).contains(&"print"));
}

#[test]
fn a_word_inside_an_expression_offers_nothing() {
    assert_eq!(at_end("x = pri", &[]), None);
    assert_eq!(at_end("f(pri", &[]), None);
    assert_eq!(at_end("{ pri", &[]), None);
    assert_eq!(at_end("a +\n pri", &[]), None);
}

#[test]
fn after_show_the_arguments_are_offered() {
    let c = at_end("show d", &[]);
    assert_eq!(labels(&c), ["dbs", "databases"]);
    assert_eq!(c.unwrap().replace, 5..6);
    assert_eq!(labels(&at_end("x;\nshow co", &[])), ["collections"]);
    assert_eq!(
        at_end("x = show d", &[]),
        None,
        "show must start a statement"
    );
    assert_eq!(
        at_end("show\nd", &[]).map(|c| c.items[0].kind),
        Some(SuggestionKind::Word)
    );
}

#[test]
fn after_db_dot_collections_come_before_database_methods() {
    let c = at_end("db.", &["orders", "users"]);
    let got = labels(&c);
    assert_eq!(&got[..2], ["orders", "users"]);
    assert!(got.contains(&"getCollection"));
    let items = c.unwrap().items;
    assert_eq!(items[0].kind, SuggestionKind::Collection);
    assert_eq!(items[2].kind, SuggestionKind::DbMethod);
}

#[test]
fn the_replace_range_covers_only_the_typed_prefix() {
    let c = at_end("x;\ndb.us", &["users", "orders"]).unwrap();
    assert_eq!(c.replace, 6..8);
    assert_eq!(labels(&Some(c.clone())), ["users"]);
    assert_eq!(c.items[0].insert, "users");
    assert_eq!(at_end("db.", &["users"]).unwrap().replace, 3..3);
}

#[test]
fn after_db_name_dot_collection_methods_are_offered() {
    let c = at_end("db.users.fi", &["users"]);
    assert_eq!(&labels(&c)[..2], ["find", "findOne"]);
    assert_eq!(
        c.as_ref().unwrap().items[0].kind,
        SuggestionKind::CollectionMethod
    );
    assert_eq!(c.unwrap().replace, 9..11);
    assert!(labels(&at_end("db.users.", &[])).contains(&"insertOne"));
    assert!(labels(&at_end("db.users.cou", &[])).contains(&"countDocuments"));
}

#[test]
fn a_chain_continued_on_the_next_line_is_still_a_collection() {
    let text = "db.users\n  .agg";
    assert_eq!(labels(&at_end(text, &[])), ["aggregate"]);
    assert_eq!(at_end(text, &[]).unwrap().replace, 12..15);
}

#[test]
fn other_receivers_offer_nothing() {
    assert_eq!(at_end("sh.", &["users"]), None);
    assert_eq!(at_end("foo.users.fi", &[]), None);
    assert_eq!(at_end("x.db.", &["users"]), None);
    assert_eq!(at_end("db.users.find().", &[]), None);
    assert_eq!(at_end("a.b.c.fi", &[]), None);
}

#[test]
fn a_dollar_inside_an_object_or_array_offers_operators() {
    let c = at_end("db.a.find({ $ma", &[]);
    assert_eq!(labels(&c)[0], "$match");
    assert!(
        labels(&c)
            .iter()
            .all(|l| l.to_lowercase().starts_with("$ma"))
    );
    assert_eq!(c.as_ref().unwrap().items[0].kind, SuggestionKind::Operator);
    assert_eq!(c.unwrap().replace, 12..15);
    assert!(labels(&at_end("db.a.aggregate([{ $gr", &[])).contains(&"$group"));
    assert!(labels(&at_end("x.update({}, { $se", &[])).contains(&"$set"));
    assert!(labels(&at_end("f({ a: { $i", &[])).contains(&"$in"));
    assert!(
        labels(&at_end("f({ a: {$", &[])).len() == 12,
        "a bare $ lists the first 12"
    );
}

#[test]
fn a_dollar_outside_an_object_or_array_offers_nothing() {
    assert_eq!(at_end("$ma", &[]), None);
    assert_eq!(at_end("f($ma", &[]), None);
    assert_eq!(at_end("{ a.$ma", &[]), None);
}

// --- filtering ----------------------------------------------------------------------------

#[test]
fn the_prefix_filters_case_insensitively() {
    assert!(labels(&at_end("OBJ", &[])).contains(&"ObjectId"));
    assert_eq!(
        labels(&at_end("db.US", &["users", "Usage", "orders"])),
        ["users", "Usage"]
    );
    assert!(labels(&at_end("db.users.FINDo", &[])).contains(&"findOne"));
}

#[test]
fn at_most_twelve_items_are_offered() {
    let many: Vec<String> = (0..20).map(|i| format!("c{i:02}")).collect();
    let c = complete("db.", 3, &many).unwrap();
    assert_eq!(c.items.len(), 12);
    assert_eq!(c.items[0].label, "c00");
    assert_eq!(
        c.items[11].label, "c11",
        "collections first, then cut at the cap"
    );
}

#[test]
fn nothing_matching_is_none() {
    assert_eq!(at_end("db.zzzz", &["users"]), None);
    assert_eq!(at_end("qqq", &[]), None);
}

// --- code only ----------------------------------------------------------------------------

#[test]
fn nothing_inside_strings_comments_templates_or_regexes() {
    assert_eq!(at_end("'db.us", &["users"]), None);
    assert_eq!(at_end("\"db.us", &["users"]), None);
    assert_eq!(at_end("// db.us", &["users"]), None);
    assert_eq!(at_end("/* db.us", &["users"]), None);
    assert_eq!(at_end("`abc db.us", &["users"]), None);
    assert_eq!(at_end("x = {a: '$ma", &[]), None);
    assert_eq!(complete("x = /db.us/", 9, &colls(&["users"])), None);
    assert_eq!(at_end("db.a.find('x').", &["users"]), None);
}

#[test]
fn a_template_placeholder_is_code_again() {
    assert!(labels(&at_end("`a ${ db.us", &["users"])).contains(&"users"));
}

#[test]
fn an_empty_word_needs_a_dot_or_a_dollar() {
    assert_eq!(at_end("", &["users"]), None);
    assert_eq!(at_end("x = 1; ", &["users"]), None);
    assert_eq!(at_end("show ", &[]), None);
    assert_eq!(at_end("db.users.find(", &[]), None);
    assert_eq!(at_end("x;\n", &[]), None);
}

// --- collection names that are not plain identifiers -----------------------------------------

#[test]
fn a_name_that_is_not_an_identifier_uses_get_collection() {
    let c = at_end("x;\ndb.m", &["my-coll", "mine"]).unwrap();
    assert_eq!(c.items[0].label, "my-coll");
    assert_eq!(c.items[0].insert, "db.getCollection(\"my-coll\")");
    // The replaced range now includes `db.`, so every other insert carries it too.
    assert_eq!(c.items[1].insert, "db.mine");
    assert_eq!(c.replace, 3..7, "db. plus the prefix");
    let c = at_end("db.", &["1st", "a b", "ok_1$"]).unwrap();
    assert_eq!(c.items[0].insert, "db.getCollection(\"1st\")");
    assert_eq!(c.items[1].insert, "db.getCollection(\"a b\")");
    assert_eq!(c.items[2].insert, "db.ok_1$");
    assert!(c.items.iter().all(|s| s.insert.starts_with("db.")));
    assert_eq!(c.replace, 0..3);
}

#[test]
fn plain_names_alone_keep_the_short_range() {
    let c = at_end("db.o", &["ok", "my-coll"]).unwrap();
    assert_eq!(labels(&Some(c.clone())), ["ok"]);
    assert_eq!(c.items[0].insert, "ok");
    assert_eq!(c.replace, 3..4);
}

#[test]
fn the_name_lists_have_no_duplicates() {
    use super::super::shell_complete_data::*;
    use super::super::shell_complete_ops::OPERATORS;
    for list in [
        STATEMENT_WORDS,
        SHOW_ARGS,
        DB_METHODS,
        COLLECTION_METHODS,
        OPERATORS,
    ] {
        let mut sorted = list.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len());
    }
    assert!(OPERATORS.iter().all(|o| o.starts_with('$')));
}

#[test]
fn quotes_and_backslashes_in_a_name_are_escaped() {
    let c = at_end("db.a", &["a\"b", "a\\c", "a\nd"]).unwrap();
    assert_eq!(c.items[0].insert, "db.getCollection(\"a\\\"b\")");
    assert_eq!(c.items[1].insert, "db.getCollection(\"a\\\\c\")");
    assert_eq!(c.items[2].insert, "db.getCollection(\"a\\nd\")");
}

#[test]
fn a_collection_named_like_a_database_method_uses_get_collection() {
    let c = at_end("db.st", &["stats"]).unwrap();
    assert_eq!(c.items[0].insert, "db.getCollection(\"stats\")");
}

#[test]
fn a_get_collection_rewrite_keeps_the_text_before_db() {
    let c = at_end("const a = 1;\n  db.my", &["my-coll"]).unwrap();
    assert_eq!(c.replace, 15..20);
}

// --- offsets ---------------------------------------------------------------------------------

#[test]
fn offsets_are_bytes_after_multi_byte_text() {
    let text = "// é\nx = 'ü';\ndb.us";
    let c = at_end(text, &["users"]).unwrap();
    assert!(text.is_char_boundary(c.replace.start) && text.is_char_boundary(c.replace.end));
    assert_eq!(&text[c.replace.clone()], "us");
    let c = at_end("é;db.", &["users"]).unwrap();
    assert_eq!(c.replace, 6..6);
}

#[test]
fn a_caret_off_a_char_boundary_or_past_the_end_does_not_panic() {
    let text = "é db.us";
    assert_eq!(complete(text, 1, &colls(&["users"])), None);
    assert_eq!(complete(text, 99, &colls(&["users"])), None);
    assert!(complete(text, text.len(), &colls(&["users"])).is_some());
}

#[test]
fn the_caret_can_be_in_the_middle_of_the_text() {
    let text = "db.us.find({})";
    let c = complete(text, 5, &colls(&["users"])).unwrap();
    assert_eq!(c.replace, 3..5);
}

// --- place ------------------------------------------------------------------------------------

#[test]
fn place_at_names_the_context() {
    assert_eq!(place_at("pri", 3), Some(Place::StatementWord));
    assert_eq!(place_at("show d", 6), Some(Place::ShowArg));
    assert_eq!(place_at("db.", 3), Some(Place::DbMember));
    assert_eq!(place_at("db.a.", 5), Some(Place::CollectionMember));
    assert_eq!(place_at("f({ $a", 6), Some(Place::Operator));
    assert_eq!(place_at("x = 1", 5), None);
    assert_eq!(place_at("'db.", 4), None);
}
