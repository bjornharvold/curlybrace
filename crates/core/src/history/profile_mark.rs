//! The profile poller's watermark, persisted so a restart or reconnect continues where the last
//! run stopped instead of counting the entries still in `system.profile` a second time.
//!
//! The mark is keyed by (connection, host, db), the host being the node the page was read from.
//! Reading takes the newest mark over all hosts of a database: at first sight the node is not
//! yet known, and a profile entry already recorded stays recorded whichever node it came from.

use super::db::{HistoryDb, sql_err};
use crate::error::Result;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

/// Where reading one database's profile continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMark {
    pub connection: Uuid,
    pub host: String,
    pub db: String,
    /// The newest `ts` (Unix ms) of the entries recorded.
    pub ts_millis: i64,
}

impl HistoryDb {
    /// Records a page of observations and moves the mark past it, in ONE transaction: a failure
    /// leaves both as they were, so the page is read and counted again, never half. A purged
    /// connection records nothing (checked under the connection lock). The mark only moves
    /// forward.
    pub fn record_page(
        &self,
        rows: &[super::shapes::ShapeObservation],
        mark: Option<&ProfileMark>,
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(sql_err)?;
        self.insert_observations(&tx, rows)?;
        if let Some(mark) = mark.filter(|m| !self.is_purged(m.connection)) {
            tx.execute(
                "INSERT INTO profile_mark (connection, host, db, ts_millis)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (connection, host, db) DO UPDATE SET
                     ts_millis = MAX(ts_millis, excluded.ts_millis)",
                params![
                    mark.connection.to_string(),
                    mark.host,
                    mark.db,
                    mark.ts_millis
                ],
            )
            .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)
    }

    /// The newest stored mark of `db` over all nodes, if any.
    pub fn profile_mark(&self, connection: Uuid, db: &str) -> Result<Option<i64>> {
        self.lock()
            .query_row(
                "SELECT MAX(ts_millis) FROM profile_mark WHERE connection = ?1 AND db = ?2",
                params![connection.to_string(), db],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(sql_err)
    }
}
