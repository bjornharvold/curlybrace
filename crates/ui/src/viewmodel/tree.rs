//! Pure flattening of a document into tree rows (collapsed containers hide children).
use crate::I18n;
use bson::{Bson, Document};
use mongostudio_core::ejson::bson_type_name;
use std::collections::HashSet;

/// Most rows emitted for one document/array level; the rest collapse into a summary row.
pub const MAX_ROWS_PER_LEVEL: usize = 200;

/// Which document view is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Tree,
    Table,
    Json,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Tree, ViewMode::Table, ViewMode::Json];

    /// i18n key of the mode's switcher label.
    pub fn label_key(self) -> &'static str {
        match self {
            ViewMode::Tree => "view.tree",
            ViewMode::Table => "view.table",
            ViewMode::Json => "view.json",
        }
    }
}

/// How the view renders a row's value; keeps translatable parts out of the pure data.
#[derive(Debug, Clone, PartialEq)]
pub enum RowDetail {
    /// `TreeRow::value` is the literal text.
    Scalar,
    /// A translated keyword, given as its `cell.*` key.
    Keyword(&'static str),
    Object(usize),
    Array(usize),
    Binary(usize),
    Timestamp {
        time: u32,
        increment: u32,
    },
    /// Summary row: this many items were not listed.
    More(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub key: String,
    /// Internal label from `bson_type_name`; see [`type_label_key`] for display.
    pub type_name: &'static str,
    pub value: String,
    pub path: Vec<String>,
    pub detail: RowDetail,
}

impl TreeRow {
    /// True for a non-empty document or array (it can be expanded).
    pub fn is_expandable(&self) -> bool {
        matches!(self.detail, RowDetail::Object(n) | RowDetail::Array(n) if n > 0)
    }

    pub fn is_summary(&self) -> bool {
        matches!(self.detail, RowDetail::More(_))
    }
}

/// i18n key under which the view displays an internal type label.
pub fn type_label_key(type_name: &str) -> String {
    format!("bsontype.{type_name}")
}

/// Rows for the top-level fields of `doc`, plus the children of every expanded path.
pub fn flatten_doc(doc: &Document, expanded: &HashSet<Vec<String>>) -> Vec<TreeRow> {
    let mut out = Vec::new();
    let entries = doc.iter().map(|(k, v)| (k.clone(), v));
    push_level(entries, doc.len(), &[], expanded, &mut out);
    out
}

fn push_level<'a>(
    entries: impl Iterator<Item = (String, &'a Bson)>,
    total: usize,
    parent: &[String],
    expanded: &HashSet<Vec<String>>,
    out: &mut Vec<TreeRow>,
) {
    for (key, value) in entries.take(MAX_ROWS_PER_LEVEL) {
        let mut path = parent.to_vec();
        path.push(key.clone());
        let row = make_row(parent.len(), key, value, path);
        let open = row.is_expandable() && expanded.contains(&row.path);
        let path = row.path.clone();
        out.push(row);
        if open {
            push_children(value, &path, expanded, out);
        }
    }
    if total > MAX_ROWS_PER_LEVEL {
        out.push(summary_row(parent, total - MAX_ROWS_PER_LEVEL));
    }
}

fn push_children(
    value: &Bson,
    path: &[String],
    expanded: &HashSet<Vec<String>>,
    out: &mut Vec<TreeRow>,
) {
    match value {
        Bson::Document(d) => {
            let entries = d.iter().map(|(k, v)| (k.clone(), v));
            push_level(entries, d.len(), path, expanded, out);
        }
        Bson::Array(a) => {
            let entries = a.iter().enumerate().map(|(i, v)| (i.to_string(), v));
            push_level(entries, a.len(), path, expanded, out);
        }
        _ => {}
    }
}

fn make_row(depth: usize, key: String, value: &Bson, path: Vec<String>) -> TreeRow {
    let (detail, text) = detail_of(value);
    TreeRow {
        depth,
        key,
        type_name: bson_type_name(value),
        value: text,
        path,
        detail,
    }
}

fn detail_of(value: &Bson) -> (RowDetail, String) {
    let scalar = |s: String| (RowDetail::Scalar, s);
    let keyword = |k: &'static str| (RowDetail::Keyword(k), String::new());
    match value {
        Bson::Document(d) => (RowDetail::Object(d.len()), String::new()),
        Bson::Array(a) => (RowDetail::Array(a.len()), String::new()),
        Bson::Binary(b) => (RowDetail::Binary(b.bytes.len()), String::new()),
        Bson::Timestamp(t) => (
            RowDetail::Timestamp {
                time: t.time,
                increment: t.increment,
            },
            String::new(),
        ),
        Bson::Null => keyword("cell.null"),
        Bson::Undefined => keyword("cell.undefined"),
        Bson::MinKey => keyword("cell.min_key"),
        Bson::MaxKey => keyword("cell.max_key"),
        Bson::String(s) | Bson::Symbol(s) | Bson::JavaScriptCode(s) => scalar(s.clone()),
        Bson::ObjectId(o) => scalar(o.to_hex()),
        Bson::DateTime(dt) => scalar(
            dt.try_to_rfc3339_string()
                .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        ),
        Bson::RegularExpression(re) => scalar(format!("/{}/{}", re.pattern, re.options)),
        other => scalar(other.to_string()),
    }
}

fn summary_row(parent: &[String], remaining: usize) -> TreeRow {
    let mut path = parent.to_vec();
    path.push(String::new());
    TreeRow {
        depth: parent.len(),
        key: String::new(),
        type_name: "",
        value: String::new(),
        path,
        detail: RowDetail::More(remaining),
    }
}

/// Display text of a row's value, translated where needed.
pub fn row_value_text(row: &TreeRow, i18n: &I18n) -> String {
    let count = |key: &str, n: usize| i18n.t_with(key, &[("n", &n.to_string())]);
    match &row.detail {
        RowDetail::Scalar => row.value.clone(),
        RowDetail::Keyword(key) => i18n.t(key),
        RowDetail::Object(n) => count("cell.document", *n),
        RowDetail::Array(n) => count("cell.array", *n),
        RowDetail::Binary(n) => count("cell.binary", *n),
        RowDetail::More(n) => count("tree.more_items", *n),
        RowDetail::Timestamp { time, increment } => i18n.t_with(
            "cell.timestamp",
            &[
                ("time", &time.to_string()),
                ("increment", &increment.to_string()),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use mongostudio_core::test_support::mock_objects::exotic_doc;

    fn none() -> HashSet<Vec<String>> {
        HashSet::new()
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exotic_doc_has_one_row_per_field_with_right_type() {
        let d = exotic_doc();
        let rows = flatten_doc(&d, &none());
        assert_eq!(rows.len(), d.len());
        for (row, (key, value)) in rows.iter().zip(d.iter()) {
            assert_eq!(&row.key, key);
            assert_eq!(row.type_name, bson_type_name(value));
            assert_eq!(row.depth, 0);
            assert_eq!(row.path, vec![key.clone()]);
        }
    }

    #[test]
    fn nested_hidden_until_expanded_then_children_at_next_depth() {
        let d = exotic_doc();
        assert!(flatten_doc(&d, &none()).iter().all(|r| r.path.len() == 1));
        let open: HashSet<_> = [path(&["nested"])].into();
        let rows = flatten_doc(&d, &open);
        assert_eq!(rows.len(), d.len() + 1);
        let at = rows.iter().position(|r| r.key == "nested").unwrap();
        let child = &rows[at + 1];
        assert_eq!((child.key.as_str(), child.depth), ("a", 1));
        assert_eq!(child.path, path(&["nested", "a"]));
    }

    #[test]
    fn expanded_array_uses_index_keys_and_deep_paths_need_every_ancestor() {
        let d = doc! { "arr": [1_i32, { "x": 1_i32 }] };
        let open: HashSet<_> = [path(&["arr"])].into();
        let rows = flatten_doc(&d, &open);
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["arr", "0", "1"]);
        let deep: HashSet<_> = [path(&["arr", "1"])].into();
        assert_eq!(flatten_doc(&d, &deep).len(), 1, "parent collapsed hides it");
        let both: HashSet<_> = [path(&["arr"]), path(&["arr", "1"])].into();
        assert_eq!(flatten_doc(&d, &both).len(), 4);
    }

    #[test]
    fn huge_array_is_capped_with_one_summary_row() {
        let big: Vec<Bson> = (0..10_000).map(Bson::Int32).collect();
        let d = doc! { "big": big };
        let open: HashSet<_> = [path(&["big"])].into();
        let rows = flatten_doc(&d, &open);
        assert_eq!(rows.len(), 1 + MAX_ROWS_PER_LEVEL + 1);
        let last = rows.last().unwrap();
        assert_eq!(last.detail, RowDetail::More(10_000 - MAX_ROWS_PER_LEVEL));
        assert!(last.is_summary());
        assert_eq!(last.depth, 1);
    }

    #[test]
    fn wide_top_level_is_capped_too() {
        let mut d = Document::new();
        for i in 0..(MAX_ROWS_PER_LEVEL + 5) {
            d.insert(format!("k{i}"), i as i32);
        }
        let rows = flatten_doc(&d, &none());
        assert_eq!(rows.len(), MAX_ROWS_PER_LEVEL + 1);
        assert_eq!(rows.last().unwrap().detail, RowDetail::More(5));
    }

    #[test]
    fn exactly_200_children_have_no_summary_and_201_have_one_with_remaining_1() {
        for (count, summary) in [
            (MAX_ROWS_PER_LEVEL, None),
            (MAX_ROWS_PER_LEVEL + 1, Some(1)),
        ] {
            let arr: Vec<Bson> = (0..count as i32).map(Bson::Int32).collect();
            let d = doc! { "a": arr };
            let open: HashSet<_> = [path(&["a"])].into();
            let rows = flatten_doc(&d, &open);
            match summary {
                None => {
                    assert_eq!(rows.len(), 1 + MAX_ROWS_PER_LEVEL);
                    assert!(rows.iter().all(|r| !r.is_summary()));
                }
                Some(n) => {
                    assert_eq!(rows.len(), 1 + MAX_ROWS_PER_LEVEL + 1);
                    assert_eq!(rows.iter().filter(|r| r.is_summary()).count(), 1);
                    assert_eq!(rows.last().unwrap().detail, RowDetail::More(n));
                }
            }
        }
    }

    #[test]
    fn dots_spaces_and_unicode_keys_are_single_path_segments() {
        let d = doc! { "a.b c": { "ключ ✓": 1_i32 } };
        let open: HashSet<_> = [path(&["a.b c"])].into();
        let rows = flatten_doc(&d, &open);
        assert_eq!(rows[1].path, path(&["a.b c", "ключ ✓"]));
    }

    #[test]
    fn empty_containers_are_not_expandable_and_order_is_preserved() {
        let d = doc! { "z": {}, "a": [], "m": 1_i32 };
        let rows = flatten_doc(&d, &none());
        assert_eq!(
            rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["z", "a", "m"]
        );
        assert!(!rows[0].is_expandable() && !rows[1].is_expandable());
        assert_eq!(flatten_doc(&d, &[path(&["z"])].into()).len(), 3);
    }

    #[test]
    fn value_text_translates_summaries_and_keywords() {
        let i18n = I18n::english();
        let d = doc! { "n": Bson::Null, "arr": [1_i32, 2_i32], "s": "hi" };
        let rows = flatten_doc(&d, &none());
        assert_eq!(row_value_text(&rows[0], &i18n), i18n.t("cell.null"));
        assert_eq!(row_value_text(&rows[1], &i18n), "2 items");
        assert_eq!(row_value_text(&rows[2], &i18n), "hi");
        let more = summary_row(&[], 7);
        assert_eq!(
            row_value_text(&more, &i18n),
            i18n.t_with("tree.more_items", &[("n", "7")])
        );
    }

    #[test]
    fn type_label_key_namespaces_the_internal_name() {
        assert_eq!(type_label_key("objectId"), "bsontype.objectId");
    }
}
