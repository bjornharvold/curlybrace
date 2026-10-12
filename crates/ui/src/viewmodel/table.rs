//! Column derivation for the table view.
use bson::Document;

/// Union of all keys in first-seen order, with `_id` first when present.
pub fn table_columns(docs: &[Document]) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for key in docs.iter().flat_map(|d| d.keys()) {
        if !cols.iter().any(|c| c == key) {
            cols.push(key.clone());
        }
    }
    if let Some(pos) = cols.iter().position(|c| c == "_id") {
        let id = cols.remove(pos);
        cols.insert(0, id);
    }
    cols
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn unions_keys_in_first_seen_order_with_id_first() {
        let docs = vec![
            doc! { "b": 1, "_id": 1, "a": 2 },
            doc! { "c": 3, "a": 4, "_id": 2 },
        ];
        assert_eq!(table_columns(&docs), ["_id", "b", "a", "c"]);
    }

    #[test]
    fn no_id_keeps_order_and_empty_is_empty() {
        assert_eq!(table_columns(&[doc! { "x": 1, "y": 2 }]), ["x", "y"]);
        assert!(table_columns(&[]).is_empty());
    }
}
