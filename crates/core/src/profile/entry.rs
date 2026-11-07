use std::fmt;

use bson::{Bson, Document};

use crate::error::{CoreError, Result};
use crate::explain::count;

/// The `appName` this application connects with; its profiler entries are our own traffic.
pub const OWN_APP_NAME: &str = "curlybrace";

#[derive(Clone, PartialEq)]
pub struct ProfileEntry {
    pub ts_secs: i64,
    pub ns: String,
    pub op: String,
    pub millis: u64,
    pub plan_summary: String,
    pub keys_examined: u64,
    pub docs_examined: u64,
    pub returned: u64,
    pub has_sort_stage: bool,
    pub app_name: Option<String>,
    pub command: Document,
}

impl ProfileEntry {
    /// True for a cursor continuation (`getMore`): it re-reads an earlier query's cursor, so it
    /// is neither a new execution nor judged by its own plan. The poller skips continuations:
    /// the originating find entry carries the plan and the shape.
    pub fn is_continuation(&self) -> bool {
        self.op.eq_ignore_ascii_case("getmore") || first_key_is(&self.command, "getMore")
    }
}

/// `command` holds the query's literal values, so Debug (logs, panics, test output) shows only
/// how many keys it has.
impl fmt::Debug for ProfileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileEntry")
            .field("ts_secs", &self.ts_secs)
            .field("ns", &self.ns)
            .field("op", &self.op)
            .field("millis", &self.millis)
            .field("plan_summary", &self.plan_summary)
            .field("keys_examined", &self.keys_examined)
            .field("docs_examined", &self.docs_examined)
            .field("returned", &self.returned)
            .field("has_sort_stage", &self.has_sort_stage)
            .field("app_name", &self.app_name)
            .field(
                "command",
                &format_args!("<redacted {} keys>", self.command.len()),
            )
            .finish()
    }
}

const PROFILE_NS_SUFFIX: &str = ".system.profile";
const MILLIS_PER_SEC: i64 = 1000;

/// Reads one `system.profile` document. Numbers may be any width; every field but `ts` and `ns`
/// is optional and defaults to empty/zero. Errors with `CoreError::Database` without them.
pub fn parse_entry(doc: &Document) -> Result<ProfileEntry> {
    let ts_secs = match doc.get("ts") {
        Some(Bson::DateTime(ts)) => ts.timestamp_millis().div_euclid(MILLIS_PER_SEC),
        _ => return Err(CoreError::Database("profile entry has no ts".into())),
    };
    let ns = doc
        .get_str("ns")
        .map_err(|_| CoreError::Database("profile entry has no ns".into()))?
        .to_owned();
    let text = |key: &str| doc.get_str(key).unwrap_or_default().to_owned();
    Ok(ProfileEntry {
        ts_secs,
        ns,
        op: text("op"),
        millis: count(Some(doc), "millis"),
        plan_summary: text("planSummary"),
        keys_examined: count(Some(doc), "keysExamined"),
        docs_examined: count(Some(doc), "docsExamined"),
        returned: count(Some(doc), "nreturned"),
        has_sort_stage: doc.get_bool("hasSortStage").unwrap_or(false),
        app_name: doc.get_str("appName").ok().map(str::to_owned),
        command: doc.get_document("command").cloned().unwrap_or_default(),
    })
}

/// True for traffic this application generated itself, which must not be recorded as a miss:
/// an `appName` that is [`OWN_APP_NAME`] or ends with it after a space (the driver appends its
/// own name to a user's, e.g. `"myapp curlybrace"`; `"mycurlybrace"` is someone else's), a read
/// of a `system.profile` collection, an `$indexStats` aggregation, or an `explain` command.
pub fn is_own_traffic(entry: &ProfileEntry) -> bool {
    app_name_is_ours(entry.app_name.as_deref())
        || entry.ns.ends_with(PROFILE_NS_SUFFIX)
        || first_key_is(&entry.command, "explain")
        || is_index_stats(&entry.command)
}

fn app_name_is_ours(app_name: Option<&str>) -> bool {
    app_name.is_some_and(|name| {
        name.strip_suffix(OWN_APP_NAME)
            .is_some_and(|rest| rest.is_empty() || rest.ends_with(' '))
    })
}

fn first_key_is(command: &Document, verb: &str) -> bool {
    command.keys().next().is_some_and(|k| k == verb)
}

fn is_index_stats(command: &Document) -> bool {
    first_key_is(command, "aggregate")
        && command.get_array("pipeline").is_ok_and(|stages| {
            stages
                .iter()
                .filter_map(Bson::as_document)
                .any(|stage| stage.contains_key("$indexStats"))
        })
}
