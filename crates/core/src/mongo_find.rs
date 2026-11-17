//! The page find of [`crate::mongo_store::MongoStore`]: skip, limit, sort and projection of the
//! query plus the options the person set, applied through the one policy
//! (`PipelineOptions::find_applied`). Driver errors are classified knowing which options were
//! sent, so a missing hint, a refused collation and an unsatisfiable read preference have their
//! own error. Option values are never logged and server text is never kept.

use bson::{Bson, Document};
use futures::TryStreamExt;
use mongodb::Collection;
use mongodb::options::FindOptions;

use crate::error::Result;
use crate::mongo_pipeline_options::{
    classify_with, driver_collation, driver_hint, driver_max_time, read_criteria,
};
use crate::mongo_store::{build_page, fetch_limit, skip_as_i64};
use crate::query_options::{AppliedFindOptions, FindOperation};
use crate::store::{Page, Query};

/// The driver options of a page find: skip, limit (one over the page), sort, projection and what
/// `o` sends of the person's options. With no option set these are exactly the options the find
/// sent before options existed: no time limit, collation, hint, comment or read preference.
pub(crate) fn find_options(q: &Query, o: &AppliedFindOptions) -> Result<FindOptions> {
    let skip = skip_as_i64(q)?;
    let applied = &o.options;
    Ok(FindOptions::builder()
        .skip(skip)
        .limit(fetch_limit(q))
        .sort(q.sort.clone())
        .projection(q.projection.clone())
        .collation(applied.collation.as_ref().map(driver_collation))
        .hint(applied.hint.as_ref().map(driver_hint))
        .allow_disk_use(applied.allow_disk_use)
        .max_time(o.send_max_time.then(|| driver_max_time(applied)))
        .comment(applied.comment.clone().map(Bson::String))
        .selection_criteria(read_criteria(applied))
        .build())
}

/// One page of `q` from `collection`, with the query's options.
pub(crate) async fn find(collection: &Collection<Document>, q: &Query) -> Result<Page> {
    let applied = q.options.find_applied(FindOperation::Find);
    let options = find_options(q, &applied)?;
    let fail = |e: mongodb::error::Error| classify_with(&e, &applied.options);
    let cursor = collection
        .find(q.filter.clone())
        .with_options(options)
        .await
        .map_err(fail)?;
    let docs: Vec<Document> = cursor.try_collect().await.map_err(fail)?;
    Ok(build_page(docs, q))
}

#[cfg(test)]
#[path = "mongo_find_tests.rs"]
mod tests;
