use bson::doc;

use super::{from_query, suggest_for_shape};
use crate::explain::suggest;
use crate::store::Query;

#[test]
fn suggest_for_shape_matches_release_a_rule() {
    let q = Query {
        filter: doc! { "age": { "$gt": 18 }, "status": "open", "region": { "$in": ["eu", "us"] } },
        sort: Some(doc! { "createdAt": -1 }),
        ..Query::default()
    };
    let from_shape = suggest_for_shape(&from_query(&q)).unwrap();
    let from_query = suggest(&q).unwrap();
    // Same equality, sort, range layering; the shape orders equality fields alphabetically.
    assert_eq!(
        from_shape.keys,
        vec![
            ("region".to_string(), 1),
            ("status".to_string(), 1),
            ("createdAt".to_string(), -1),
            ("age".to_string(), 1),
        ]
    );
    let mut a = from_shape.keys.clone();
    let mut b = from_query.keys.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert!(suggest_for_shape(&from_query_empty()).is_none());
}

fn from_query_empty() -> super::Shape {
    from_query(&Query::default())
}
