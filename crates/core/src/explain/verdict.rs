//! The typed result of judging a query's explain output.

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
    /// A ready-to-copy `db.<collection>.createIndex({...})` command. Advice only.
    pub fn command(&self, collection: &str) -> String {
        let keys = self
            .keys
            .iter()
            .map(|(field, dir)| format!("{}: {dir}", serde_json::Value::from(field.as_str())))
            .collect::<Vec<_>>()
            .join(", ");
        format!("db.{collection}.createIndex({{ {keys} }})")
    }
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
}
