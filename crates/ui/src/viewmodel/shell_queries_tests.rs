use super::*;
use bson::{Document, doc};

fn one(text: &str, start_db: &str) -> ShellCall {
    let reading = read_calls(text, start_db);
    assert_eq!(reading.unchecked, 0, "unchecked in {text:?}");
    assert_eq!(
        reading.calls.len(),
        1,
        "calls in {text:?}: {:?}",
        reading.calls
    );
    reading.calls.into_iter().next().expect("one call")
}

fn nothing(text: &str) {
    assert_eq!(read_calls(text, "test"), Reading::default(), "in {text:?}");
}

#[test]
fn a_find_with_a_chained_sort_and_limit_is_one_call() {
    let text = "db.users.find({age: {$gt: 30}}).sort({age: 1}).limit(5)";
    let call = one(text, "test");
    assert_eq!(call.db, "test");
    assert_eq!(call.collection, "users");
    assert_eq!(call.method, "find");
    assert_eq!(call.query.filter, doc! { "age": { "$gt": 30 } });
    assert_eq!(call.query.sort, Some(doc! { "age": 1 }));
    assert_eq!(call.query.projection, None);
    assert_eq!(call.range, 0..text.len());
}

#[test]
fn every_listed_method_is_read_and_others_are_not() {
    for method in METHODS {
        let call = one(&format!("db.c.{method}({{a: 1}})"), "test");
        assert_eq!(call.method, method);
        assert_eq!(call.query.filter, doc! { "a": 1 });
    }
    nothing("db.c.aggregate([{$match: {a: 1}}])");
    nothing("db.c.insertOne({a: 1})");
    nothing("db.getCollectionNames()");
    nothing("db.c.find");
}

#[test]
fn a_filter_that_is_not_a_literal_is_unchecked() {
    let reading = read_calls("db.users.find(filter)", "test");
    assert!(reading.calls.is_empty());
    assert_eq!(reading.unchecked, 1);
    assert_eq!(read_calls("db.users.find({a: x})", "t").unchecked, 1);
    assert_eq!(read_calls("db.users.find({a: /x/})", "t").unchecked, 1);
}

#[test]
fn an_empty_argument_list_is_an_empty_filter() {
    let call = one("db.users.find( /* all */ )", "test");
    assert_eq!(call.query.filter, Document::new());
}

#[test]
fn the_second_argument_of_find_is_the_projection() {
    let call = one("db.u.findOne({a: 1}, {_id: 0, name: 1})", "t");
    assert_eq!(call.query.projection, Some(doc! { "_id": 0, "name": 1 }));
    assert_eq!(read_calls("db.u.find({a: 1}, fields)", "t").unchecked, 1);
    // The second argument of a write is the update, not a projection.
    let update = one("db.u.updateOne({a: 1}, {$set: {b: x}})", "t");
    assert_eq!(update.query.projection, None);
}

#[test]
fn a_sort_that_is_not_a_literal_makes_the_call_unchecked() {
    let reading = read_calls("db.u.find({a: 1}).sort(order)", "t");
    assert_eq!((reading.calls.len(), reading.unchecked), (0, 1));
}

#[test]
fn other_chained_calls_are_skipped_and_limit_and_skip_are_read() {
    let text =
        "db.u.find({a: 1})\n  // newest first\n  .skip(10).sort({at: -1}).limit(3).toArray()";
    let call = one(text, "t");
    assert_eq!(call.query.sort, Some(doc! { "at": -1 }));
    assert_eq!((call.query.skip, call.query.limit), (10, 3));
    assert_eq!(call.range, 0..text.len());
    let pretty = one("db.u.find({}).pretty(); x", "t");
    assert_eq!(pretty.range, 0.."db.u.find({}).pretty()".len());
}

#[test]
fn calls_in_comments_strings_templates_and_regex_literals_are_ignored() {
    nothing("// db.a.find({})");
    nothing("/* db.a.find({}) */");
    nothing("x = \"db.a.find({})\"");
    nothing("x = 'db.a.find({})'");
    nothing("x = `db.a.find({})`");
    nothing("x = /db.a.find({})/");
}

#[test]
fn use_changes_the_database_for_later_calls() {
    let text = "db.a.find({})\nuse shop\ndb.orders.deleteMany({status: 'x'})";
    let reading = read_calls(text, "test");
    let dbs: Vec<&str> = reading.calls.iter().map(|c| c.db.as_str()).collect();
    assert_eq!(dbs, ["test", "shop"]);
    assert_eq!(reading.calls[1].collection, "orders");
    assert_eq!(reading.calls[1].method, "deleteMany");
    assert_eq!(one("use my-db_2;\ndb.a.find({})", "t").db, "my-db_2");
}

#[test]
fn a_use_whose_name_is_not_plain_makes_later_calls_unchecked_until_a_plain_one() {
    let text = "use 'x'\ndb.a.find({})\nuse a.b\ndb.a.find({})\nuse ok\ndb.a.find({})";
    let reading = read_calls(text, "t");
    assert_eq!(reading.unchecked, 2);
    assert_eq!(reading.calls.len(), 1);
    assert_eq!(reading.calls[0].db, "ok");
}

#[test]
fn use_as_a_variable_or_mid_line_is_not_a_database_switch() {
    for text in [
        "use = 1\ndb.a.find({})",
        "x({\n use: 1 })\ndb.a.find({})",
        "f(); use shop\ndb.a.find({})",
    ] {
        assert_eq!(one(text, "t").db, "t", "in {text:?}");
    }
}

#[test]
fn get_sibling_db_with_a_literal_names_the_database() {
    let call = one("db.getSiblingDB(\"a\").users.find({})", "test");
    assert_eq!((call.db.as_str(), call.collection.as_str()), ("a", "users"));
    // As decided for this guard, the literal also applies to later calls.
    let reading = read_calls("db.getSiblingDB('a')\ndb.b.find({})", "test");
    assert_eq!(reading.calls[0].db, "a");
}

#[test]
fn get_sibling_db_with_a_variable_makes_it_and_later_calls_unchecked() {
    let text = "db.getSiblingDB(name).users.find({})\ndb.b.find({})\nuse back\ndb.c.find({})";
    let reading = read_calls(text, "test");
    assert_eq!(reading.unchecked, 2);
    assert_eq!(reading.calls.len(), 1);
    assert_eq!(reading.calls[0].db, "back");
}

#[test]
fn get_collection_receivers_are_read() {
    let call = one("db.getCollection(\"my coll\").find({a: 1})", "t");
    assert_eq!(call.collection, "my coll");
    let sibling = one(
        "db.getSiblingDB('s').getCollection('c').countDocuments({})",
        "t",
    );
    assert_eq!(
        (sibling.db.as_str(), sibling.collection.as_str()),
        ("s", "c")
    );
    assert_eq!(read_calls("db.getCollection(n).find({})", "t").unchecked, 1);
}

#[test]
fn dotted_collection_names_are_joined() {
    assert_eq!(
        one("db.system.profile.find({})", "t").collection,
        "system.profile"
    );
}

#[test]
fn a_db_that_is_a_property_of_something_else_is_not_read() {
    nothing("x.db.a.find({})");
    nothing("mydb.a.find({})");
}

#[test]
fn a_call_nested_in_a_callback_is_read_on_its_own() {
    let reading = read_calls(
        "db.a.find({}).forEach(d => db.b.deleteOne({_id: d._id}))",
        "t",
    );
    assert_eq!(reading.calls.len(), 1);
    assert_eq!(reading.calls[0].collection, "a");
    assert_eq!(reading.unchecked, 1);
}

#[test]
fn thirty_calls_are_all_returned() {
    let text: String = (0..30)
        .map(|n| format!("db.c{n}.find({{n: {n}}})\n"))
        .collect();
    let reading = read_calls(&text, "t");
    assert_eq!(reading.calls.len(), 30);
    assert_eq!(reading.calls[29].collection, "c29");
}

#[test]
fn ranges_are_byte_ranges_of_the_original_text_around_multibyte_text() {
    let text = "// 😀 é\nconst s = '日本'; db.ü.find({n: \"😀\"}).sort({é: 1}) // ß";
    let call = one(text, "t");
    assert_eq!(
        &text[call.range.clone()],
        "db.ü.find({n: \"😀\"}).sort({é: 1})"
    );
    assert_eq!(call.collection, "ü");
    assert_eq!(call.query.sort, Some(doc! { "é": 1 }));
}

#[test]
fn broken_scripts_neither_panic_nor_loop() {
    let broken = [
        "db.a.find(",
        "db.a.find({",
        "db.a.find({a: 'x",
        "db.a.find({a: `x",
        "db.a.find({a: 1 /* x",
        "db.a.find({)}",
        "db.a.find(}{)",
        "db.",
        "db",
        "db.a.",
        "db.a.find",
        "db.getSiblingDB(",
        "db.getSiblingDB('a').",
        "db.getCollection(",
        "use",
        "use\n",
        ")))]]]}}}",
        "db.a.find({}).sort(",
        "db.a.find({}).",
        "😀db.😀.find({😀})",
        "\\",
        "db.a.find('\\",
    ];
    for text in broken {
        let reading = read_calls(text, "t");
        assert!(reading.calls.len() <= 1, "in {text:?}");
    }
    assert_eq!(read_calls("db.a.find(", "t").unchecked, 1);
    assert_eq!(read_calls("db.getSiblingDB(", "t").calls.len(), 0);
}

#[test]
fn every_prefix_of_a_script_is_read_without_panicking() {
    let text = "use shop\ndb.getSiblingDB('é').getCollection(\"😀\").find({a: [1, {b: ISODate('2024-05-01')}]}, {x: 1}).sort({a: -1}) // ü\n";
    for (at, _) in text.char_indices() {
        let _ = read_calls(&text[..at], "t");
    }
}

#[test]
fn many_unterminated_calls_are_read_in_linear_time() {
    let text = "db.a.find(".repeat(100_000);
    let reading = read_calls(&text, "t");
    assert!(reading.calls.is_empty());
    assert_eq!(reading.unchecked, 100_000);
}

#[test]
fn many_calls_are_read_in_linear_time() {
    let text = "db.a.find({a: 1}).sort({b: 1});\n".repeat(50_000);
    let started = std::time::Instant::now();
    let reading = read_calls(&text, "t");
    assert_eq!(reading.calls.len(), 50_000);
    assert_eq!(reading.unchecked, 0);
    // Generous: linear work here is milliseconds; a quadratic reader takes minutes.
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
}

#[test]
fn long_lines_of_braces_and_deep_nesting_are_unchecked_without_overflow() {
    let braces = format!("db.a.find({})", "{".repeat(200_000));
    assert_eq!(read_calls(&braces, "t").unchecked, 1);
    let deep = format!(
        "db.a.find({}1{})",
        "{a: ".repeat(50_000),
        "}".repeat(50_000)
    );
    assert_eq!(read_calls(&deep, "t").unchecked, 1);
    let nested_calls = "db.a.find({a: ".repeat(20_000);
    assert_eq!(read_calls(&nested_calls, "t").unchecked, 20_000);
}
