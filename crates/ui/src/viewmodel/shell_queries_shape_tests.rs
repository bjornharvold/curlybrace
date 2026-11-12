//! The database rules for an unknown `db`, and the call shapes that are read or counted
//! unchecked rather than dropped.
use super::*;
use bson::doc;

fn counts(text: &str) -> (usize, usize) {
    let reading = read_calls(text, "t");
    (reading.calls.len(), reading.unchecked)
}

#[test]
fn an_unknown_db_stays_unknown_through_an_assigned_sibling() {
    assert_eq!(
        read_calls("db = x\ndb = db.getSiblingDB('b')\ndb.z.find({})", "t").unchecked,
        1
    );
    let reading = read_calls(
        "db = connect('otherhost/x'); db = db.getSiblingDB('b'); db.users.find({a: 1})",
        "t",
    );
    assert_eq!((reading.calls.len(), reading.unchecked), (0, 1));
}

#[test]
fn an_unknown_db_stays_unknown_through_a_sibling_receiver() {
    assert_eq!(
        read_calls("db = x\ndb.getSiblingDB('a').z.find({})", "t").unchecked,
        1
    );
}

#[test]
fn a_use_line_makes_an_unknown_db_known_again() {
    let text =
        "db = x\ndb = db.getSiblingDB('b')\nuse b\ndb.z.find({})\ndb.getSiblingDB('a').z.find({})";
    let reading = read_calls(text, "t");
    let dbs: Vec<&str> = reading.calls.iter().map(|c| c.db.as_str()).collect();
    assert_eq!((dbs, reading.unchecked), (vec!["b", "a"], 0));
}

#[test]
fn a_string_index_on_db_is_read_like_get_collection() {
    let reading = read_calls("db['my-coll'].find({a: 1})", "t");
    assert_eq!(reading.unchecked, 0);
    let call = &reading.calls[0];
    assert_eq!((call.collection.as_str(), call.method), ("my-coll", "find"));
    assert_eq!(call.query.filter, doc! { "a": 1 });
    let sibling = read_calls("db.getSiblingDB('s')[\"c\"].deleteOne({})", "t");
    assert_eq!(sibling.calls[0].db, "s");
    assert_eq!(counts("db[name].find({})"), (0, 1));
}

#[test]
fn other_receiver_shapes_with_a_listed_method_count_as_unchecked() {
    for text in [
        "db.getSiblingDB('a').getSiblingDB('b').x.find()",
        "db.getCollection('a').b.find({})",
        "db['a'].b.find({})",
        "db.a.withOptions().find({})",
    ] {
        assert_eq!(counts(text), (0, 1), "in {text:?}");
    }
    // Shapes with no listed method are still not calls of interest.
    assert_eq!(counts("db.a.aggregate([]).toArray()"), (0, 0));
    assert_eq!(counts("db.getCollectionNames()"), (0, 0));
}

#[test]
fn plan_changing_chained_calls_make_the_call_unchecked() {
    for chained in [
        ".hint({a: 1})",
        ".collation({locale: 'fr'})",
        ".min({a: 1})",
        ".max({a: 9})",
    ] {
        let text = format!("db.a.find({{a: 1}}){chained}.limit(1)");
        assert_eq!(counts(&text), (0, 1), "in {text:?}");
    }
}

#[test]
fn an_options_argument_to_find_makes_the_call_unchecked() {
    assert_eq!(counts("db.a.find({a: 1}, {b: 1}, {hint: 'a_1'})"), (0, 1));
    assert_eq!(
        counts("db.a.findOne({a: 1}, {}, {collation: {locale: 'fr'}})"),
        (0, 1)
    );
    // A blank third argument is no options at all.
    assert_eq!(counts("db.a.find({a: 1}, {b: 1}, )"), (1, 0));
}

#[test]
fn an_options_argument_to_a_count_update_or_delete_makes_the_call_unchecked() {
    for (call, args) in [
        ("countDocuments", "{a: 1}"),
        ("updateOne", "{a: 1}, {$set: {b: 2}}"),
        ("updateMany", "{a: 1}, {$set: {b: 2}}"),
        ("deleteOne", "{a: 1}"),
        ("deleteMany", "{a: 1}"),
    ] {
        let plain = format!("db.a.{call}({args})");
        assert_eq!(counts(&plain), (1, 0), "in {plain:?}");
        let blank = format!("db.a.{call}({args}, )");
        assert_eq!(counts(&blank), (1, 0), "in {blank:?}");
        for options in ["{hint: 'a_1'}", "{collation: {locale: 'fr'}}"] {
            let text = format!("db.a.{call}({args}, {options})");
            assert_eq!(counts(&text), (0, 1), "in {text:?}");
        }
    }
}
