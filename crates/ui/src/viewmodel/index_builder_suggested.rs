//! The index builder pre-filled with the index the Plan tab suggests.
use super::index_builder::{IndexBuilderState, default_name};
use curlybrace_core::ejson::doc_to_view_json;
use curlybrace_core::explain::SuggestedIndex;
use curlybrace_core::indexes::{IndexKey, KeyKind};
use curlybrace_core::pipeline_options::CollationOption;
use curlybrace_core::store::Namespace;

impl IndexBuilderState {
    /// The builder pre-filled with `index`'s keys (1 Asc, -1 Desc), the default name following
    /// them, and the collation as its JSON text when there is one (an index serves a query's
    /// string comparisons only with the query's collation). Nothing is built until the person
    /// presses Create.
    pub fn from_suggested(
        ns: Namespace,
        index: &SuggestedIndex,
        collation: Option<&CollationOption>,
    ) -> Self {
        let keys: Vec<IndexKey> = index
            .keys
            .iter()
            .map(|(field, direction)| IndexKey {
                field: field.clone(),
                kind: if *direction < 0 {
                    KeyKind::Desc
                } else {
                    KeyKind::Asc
                },
            })
            .collect();
        Self {
            name: default_name(&keys),
            keys,
            collation_text: collation
                .map(|c| doc_to_view_json(&c.to_document()))
                .unwrap_or_default(),
            ..Self::new(ns)
        }
    }
}

#[cfg(test)]
#[path = "index_builder_suggested_tests.rs"]
mod tests;
