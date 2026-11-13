//! The Add stage menu's stage names and the text of a new stage.

/// The names the Add stage menu offers, in menu order.
pub const STAGE_MENU: [&str; 13] = [
    "$match",
    "$group",
    "$project",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$count",
    "$facet",
    "$out",
    "$merge",
];

/// The template body of a new `name` stage (JSON data, not a label); an unknown name gets `{}`.
fn template_body(name: &str) -> &'static str {
    match name {
        "$group" => r#"{ "_id": null }"#,
        "$project" => r#"{ "_id": 1 }"#,
        "$sort" => r#"{ "_id": 1 }"#,
        "$limit" => "10",
        "$skip" => "0",
        "$unwind" => r#"{ "path": "$field" }"#,
        "$lookup" => {
            r#"{ "from": "collection", "localField": "field", "foreignField": "field", "as": "joined" }"#
        }
        "$count" => r#""count""#,
        "$out" => r#""collection""#,
        "$merge" => r#"{ "into": "collection" }"#,
        _ => "{}",
    }
}

/// The text of a new `name` stage, e.g. `{ "$match": {} }` laid out on three lines.
pub fn stage_template(name: &str) -> String {
    let key = serde_json::Value::String(name.to_string());
    format!("{{\n  {key}: {}\n}}", template_body(name))
}
