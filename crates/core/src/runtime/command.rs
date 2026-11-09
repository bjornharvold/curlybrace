//! Helpers of [`Command`] and [`Event`]: write classification, failure context, session and
//! request lookup.

use super::{Command, Event, SessionId};

impl Command {
    /// Whether this command changes data: a lost answer may hide a change the server applied.
    pub fn is_write(&self) -> bool {
        match self {
            Self::Insert { .. }
            | Self::Replace { .. }
            | Self::Delete { .. }
            | Self::Clone { .. }
            | Self::CreateIndex { .. }
            | Self::DropIndex { .. }
            | Self::SetIndexHidden { .. }
            | Self::SetIndexTtl { .. }
            | Self::SetProfilingLevel { .. }
            | Self::BulkPreview { .. }
            | Self::BulkUpdate { .. }
            | Self::BulkDelete { .. } => true,
            Self::Connect(_)
            | Self::Disconnect { .. }
            | Self::ListDatabases { .. }
            | Self::ListCollections { .. }
            | Self::Find { .. }
            | Self::ExplainFind { .. }
            | Self::Compare(_)
            | Self::Copy(_)
            | Self::TestConnection { .. }
            | Self::LockReadOnly { .. }
            | Self::ListIndexes { .. }
            | Self::IndexHistory { .. }
            | Self::EstimatedCount { .. }
            | Self::ProfilingState { .. }
            | Self::QueryShapes { .. }
            | Self::SetSamplingExcluded { .. }
            | Self::PurgeHistory { .. }
            | Self::BulkCount { .. }
            | Self::BulkSample { .. }
            | Self::BulkCancel { .. } => false,
        }
    }

    /// The number a numbered read carries (echoed by its answer and its failure).
    pub fn request(&self) -> Option<u64> {
        match self {
            Self::ListIndexes { request, .. }
            | Self::IndexHistory { request, .. }
            | Self::QueryShapes { request, .. }
            | Self::BulkCount { request, .. }
            | Self::BulkSample { request, .. } => Some(*request),
            _ => None,
        }
    }

    /// i18n key suffix reported in [`Event::Failed`] for this command.
    pub fn context(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::Disconnect { .. } => "disconnect",
            Self::ListDatabases { .. } => "list_databases",
            Self::ListCollections { .. } => "list_collections",
            Self::Find { .. } => "find",
            Self::ExplainFind { .. } => "explain_find",
            Self::Insert { .. } => "insert",
            Self::Replace { .. } => "replace",
            Self::Delete { .. } => "delete",
            Self::Clone { .. } => "clone",
            Self::Compare(_) => "compare",
            Self::Copy(_) => "copy",
            Self::TestConnection { .. } => "test_connection",
            Self::LockReadOnly { .. } => "lock_read_only",
            Self::ListIndexes { .. } => "list_indexes",
            Self::IndexHistory { .. } => "index_history",
            Self::EstimatedCount { .. } => "estimated_count",
            Self::ProfilingState { .. } => "profiling_state",
            Self::SetProfilingLevel { .. } => "set_profiling_level",
            Self::QueryShapes { .. } => "query_shapes",
            Self::CreateIndex { .. } => "create_index",
            Self::DropIndex { .. } => "drop_index",
            Self::SetIndexHidden { .. } => "set_index_hidden",
            Self::SetIndexTtl { .. } => "set_index_ttl",
            Self::SetSamplingExcluded { .. } => "set_sampling_excluded",
            Self::PurgeHistory { .. } => "purge_history",
            Self::BulkCount { .. } => "bulk_count",
            Self::BulkSample { .. } => "bulk_sample",
            Self::BulkPreview { .. } => "bulk_preview",
            Self::BulkUpdate { .. } => "bulk_update",
            Self::BulkDelete { .. } => "bulk_delete",
            Self::BulkCancel { .. } => "bulk_cancel",
        }
    }

    /// The session this command addresses; `None` for compare, connection tests, read-only
    /// locks and the history commands, which are independent of sessions.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Connect(cfg) => Some(cfg.id),
            Self::Disconnect { session }
            | Self::ListDatabases { session }
            | Self::ListCollections { session, .. }
            | Self::Find { session, .. }
            | Self::ExplainFind { session, .. }
            | Self::Insert { session, .. }
            | Self::Replace { session, .. }
            | Self::Delete { session, .. }
            | Self::Clone { session, .. }
            | Self::ListIndexes { session, .. }
            | Self::IndexHistory { session, .. }
            | Self::EstimatedCount { session, .. }
            | Self::ProfilingState { session, .. }
            | Self::SetProfilingLevel { session, .. }
            | Self::QueryShapes { session, .. }
            | Self::CreateIndex { session, .. }
            | Self::DropIndex { session, .. }
            | Self::SetIndexHidden { session, .. }
            | Self::SetIndexTtl { session, .. }
            | Self::BulkCount { session, .. }
            | Self::BulkSample { session, .. }
            | Self::BulkPreview { session, .. }
            | Self::BulkUpdate { session, .. }
            | Self::BulkDelete { session, .. }
            | Self::BulkCancel { session, .. } => Some(*session),
            Self::Compare(_)
            | Self::Copy(_)
            | Self::TestConnection { .. }
            | Self::LockReadOnly { .. }
            | Self::SetSamplingExcluded { .. }
            | Self::PurgeHistory { .. } => None,
        }
    }
}

impl Event {
    /// The session this event belongs to; `None` for compare and connection-test events.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Connected { session, .. }
            | Self::Disconnected { session, .. }
            | Self::ConnectFailed { session, .. }
            | Self::Databases { session, .. }
            | Self::Collections { session, .. }
            | Self::Results { session, .. }
            | Self::Explain { session, .. }
            | Self::Mutated { session, .. }
            | Self::Indexes { session, .. }
            | Self::IndexHistory { session, .. }
            | Self::EstimatedCount { session, .. }
            | Self::ProfilingState { session, .. }
            | Self::QueryShapes { session, .. }
            | Self::IndexChanged { session, .. }
            | Self::Sampler { session, .. }
            | Self::Profiler { session, .. }
            | Self::BulkCount { session, .. }
            | Self::BulkSample { session, .. }
            | Self::BulkPreview { session, .. }
            | Self::BulkFinished { session, .. }
            | Self::BulkCancelResult { session, .. }
            | Self::Failed { session, .. }
            | Self::RequestFailed { session, .. } => Some(*session),
            Self::Compare(_) | Self::Copy(_) | Self::TestConnection { .. } => None,
        }
    }
}
