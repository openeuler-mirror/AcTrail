//! Select recent IDs with reverse index seeks; leave detailed hydration to the caller.
use crate::SqliteStorage;
use crate::semantic_actions::codebook::sqlite::action_kind_code;
use rusqlite::params;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};
use storage_core::{RecentActionsQuery, StorageError};

impl SqliteStorage {
    pub(crate) fn read_recent_action_ids(
        &self,
        query: &RecentActionsQuery,
    ) -> Result<Vec<String>, StorageError> {
        if self.is_purged(query.trace_id) {
            return Err(RecentActionsReader::error("trace has been purged"));
        }
        RecentActionsReader {
            storage: self,
            query,
        }
        .read()
    }
}

struct RecentActionsReader<'a> {
    storage: &'a SqliteStorage,
    query: &'a RecentActionsQuery,
}
impl RecentActionsReader<'_> {
    fn read(&self) -> Result<Vec<String>, StorageError> {
        let query = self.query;
        if query.limit == 0 || query.start > query.end {
            return Err(Self::error("recent action limit and window must be valid"));
        }
        let limit = i64::try_from(query.limit).map_err(Self::error)?;
        let start = Self::time(query.start)?;
        let end = Self::time(query.end)?;
        let kinds: BTreeSet<_> = query.kinds.iter().copied().map(action_kind_code).collect();
        let connection = self.storage.connection().borrow();
        let mut statement = connection.prepare_cached(
            "SELECT action_key,start_time FROM semantic_actions INDEXED BY idx_semantic_actions_timeline_process_kind
             WHERE trace_id=?1 AND process_id=?2 AND kind_code=?3 AND start_time>=?4 AND start_time<=?5
             ORDER BY start_time DESC,action_key DESC LIMIT ?6"
        ).map_err(Self::error)?;
        let mut selected = Vec::new();
        // Each kind reads at most limit index entries, regardless of trace size.
        for kind in kinds {
            let rows = statement
                .query_map(
                    params![
                        query.trace_id.get(),
                        query.process.get(),
                        kind,
                        start,
                        end,
                        limit
                    ],
                    |row| Ok((row.get::<_, i64>(1)?, row.get::<_, i64>(0)?)),
                )
                .map_err(Self::error)?;
            for row in rows {
                selected.push(row.map_err(Self::error)?);
            }
        }
        selected.sort_unstable_by(|a, b| b.cmp(a));
        selected.truncate(query.limit);
        let mut ids = connection
            .prepare_cached(
                "SELECT action_id FROM semantic_action_ids WHERE action_key=?1 AND trace_id=?2",
            )
            .map_err(Self::error)?;
        selected
            .into_iter()
            .map(|(_, key)| {
                ids.query_row(params![key, query.trace_id.get()], |row| row.get(0))
                    .map_err(Self::error)
            })
            .collect()
    }
    fn time(time: SystemTime) -> Result<i64, StorageError> {
        time.duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|v| i64::try_from(v.as_nanos()).ok())
            .ok_or_else(|| Self::error("timestamp outside SQLite nanosecond range"))
    }
    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("recent_action_ids", error.to_string())
    }
}
