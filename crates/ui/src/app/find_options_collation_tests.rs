//! A page found with a collation: bulk writes and the copy of the whole result are off (they
//! match by the filter alone), while hand-picked documents and other options are not affected.
use super::super::explain_tests::browsing;
use super::super::reducer::{Input, update};
use super::super::state::{AppState, Banner};
use crate::viewmodel::{BulkEntry, DocAction, DocCopy};
use bson::doc;
use curlybrace_core::pipeline::{CollationOption, HintOption, PipelineOptions};
use curlybrace_core::store::{Page, Query};

fn hinted() -> PipelineOptions {
    PipelineOptions {
        hint: Some(HintOption::Name("a_1".into())),
        ..PipelineOptions::default()
    }
}

fn collated() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: None,
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        ..PipelineOptions::default()
    }
}

fn query_with(options: PipelineOptions) -> Query {
    Query {
        filter: doc! { "age": 1 },
        options,
        ..Query::default()
    }
}

/// A page found with `query` is showing.
fn showing(s: AppState, query: Query) -> AppState {
    AppState {
        page: Some(Page {
            docs: vec![doc! { "_id": 1 }],
            skip: 0,
            has_more: true,
        }),
        issued_query: query.clone(),
        page_query: query,
        ..s
    }
}

#[test]
fn bulk_is_off_while_the_page_was_found_with_a_collation() {
    let s = showing(browsing(), query_with(collated()));
    assert_eq!(s.bulk_entry(), BulkEntry::Collated);
    let (s, commands) = update(
        s,
        Input::Doc(DocAction::Bulk(crate::viewmodel::BulkKind::Delete)),
    );
    assert!(commands.is_empty());
    assert!(s.bulk.is_none());
    let hint_only = showing(browsing(), query_with(hinted()));
    assert_eq!(hint_only.bulk_entry(), BulkEntry::Available);
    let default = showing(browsing(), Query::default());
    assert_eq!(default.bulk_entry(), BulkEntry::Available);
}

#[test]
fn read_only_and_projected_come_before_the_collation_reason() {
    let mut s = showing(browsing(), query_with(collated()));
    s.read_only = true;
    assert_eq!(s.bulk_entry(), BulkEntry::ReadOnly);
    let mut projected = showing(browsing(), query_with(collated()));
    projected.page_query.projection = Some(doc! { "a": 1 });
    assert_eq!(projected.bulk_entry(), BulkEntry::Projected);
}

#[test]
fn copying_the_whole_result_is_refused_under_a_collation_but_picked_documents_copy() {
    let s = || showing(browsing(), query_with(collated()));
    let (refused, _) = update(s(), Input::Doc(DocAction::Copy(DocCopy::Result)));
    assert!(matches!(
        refused.banner,
        Some(Banner::PasteBlocked(
            crate::viewmodel::PasteBlock::CollatedResult
        ))
    ));
    assert_eq!(refused.clipboard, None);
    let (picked, _) = update(
        s(),
        Input::Doc(DocAction::Copy(DocCopy::Ids(vec![bson::Bson::Int32(1)]))),
    );
    assert!(picked.clipboard.is_some());
    let plain = showing(browsing(), query_with(hinted()));
    let (copied, _) = update(plain, Input::Doc(DocAction::Copy(DocCopy::Result)));
    assert!(
        copied.clipboard.is_some(),
        "a hint does not change the matches"
    );
}
