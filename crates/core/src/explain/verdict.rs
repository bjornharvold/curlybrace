//! The typed result of judging a query's explain output.

use crate::store::Namespace;

/// How the winning plan serves the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictKind {
    IndexUsed,
    CoveredByIndex,
    CollectionScan,
    SortNotCovered,
    InefficientIndex,
    NoFilter,
    EmptyCollection,
}

/// Why a verdict was reached. `key` is an i18n key suffix under `query.verdict.`;
/// core never carries English text.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub key: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl Reason {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            params: Vec::new(),
        }
    }

    pub fn with_param(self, name: &'static str, value: impl ToString) -> Self {
        let params = self
            .params
            .into_iter()
            .chain([(name, value.to_string())])
            .collect();
        Self { params, ..self }
    }
}

/// An index the query would benefit from: ordered `(field, direction)` keys.
#[derive(Debug, Clone, PartialEq)]
pub struct SuggestedIndex {
    pub keys: Vec<(String, i32)>,
}

impl SuggestedIndex {
    /// A ready-to-copy `db.getSiblingDB("<db>").getCollection("<collection>").createIndex({...})`
    /// command. Advice only. The database is named so a paste into a session that is in another
    /// database cannot index a stray collection there. Every name is rendered as an escaped
    /// string literal, so a name can never change what the pasted command does.
    pub fn command(&self, ns: &Namespace) -> String {
        let keys = self
            .keys
            .iter()
            .map(|(field, dir)| format!("{}: {dir}", js_string(field)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "db.getSiblingDB({}).getCollection({}).createIndex({{ {keys} }})",
            js_string(&ns.db),
            js_string(&ns.collection)
        )
    }
}

/// A JavaScript string literal for `text`. JSON escaping covers quotes, backslashes and
/// control characters; U+2028/U+2029 are valid in JSON but end a line in JavaScript.
fn js_string(text: &str) -> String {
    serde_json::Value::from(text)
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub index_name: Option<String>,
    pub keys_examined: u64,
    pub docs_examined: u64,
    pub returned: u64,
    /// True when the explain carried no `executionStats` (counts are then 0).
    pub plan_only: bool,
    pub reasons: Vec<Reason>,
    pub suggestion: Option<SuggestedIndex>,
    /// The `<db>.<collection>` the server planned the query on (`queryPlanner.namespace`). It
    /// differs from the namespace queried for a view, which runs on its source collection.
    pub namespace: Option<String>,
}
