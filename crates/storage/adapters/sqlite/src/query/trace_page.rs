//! Read exact trace totals and one numbered metadata page in a shared snapshot.
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::ids::TraceId;
use rusqlite::{Connection, params_from_iter, types::Value};
use storage_core::{StorageError, TracePage, TracePageQuery};

use crate::{SqliteStorage, records::trace_from_row};

impl SqliteStorage {
    pub(crate) fn read_trace_page(
        &self,
        query: &TracePageQuery,
    ) -> Result<TracePage, StorageError> {
        TracePageReader {
            connection: &self.connection().borrow(),
            query,
        }
        .read()
    }
}

struct TracePageReader<'a> {
    connection: &'a Connection,
    query: &'a TracePageQuery,
}

impl TracePageReader<'_> {
    fn read(&self) -> Result<TracePage, StorageError> {
        let query = self.query;
        if query.page == 0 || query.limit == 0 {
            return Err(Self::error("page and limit must be positive"));
        }
        let offset = query
            .page
            .checked_sub(1)
            .and_then(|page| page.checked_mul(query.limit))
            .and_then(|offset| i64::try_from(offset).ok())
            .ok_or_else(|| Self::error("trace page offset exceeds SQLite range"))?;
        if query
            .start_time
            .zip(query.end_time)
            .is_some_and(|(start, end)| start > end)
        {
            return Err(Self::error("creation window start exceeds end"));
        }
        let mut source = String::from(" FROM traces trace");
        if query.trace_id.is_some() {
            source.push_str(" NOT INDEXED");
        } else if query.name.is_some() {
            source.push_str(" INDEXED BY idx_traces_name_created_id");
        } else {
            source.push_str(" INDEXED BY idx_traces_created_id");
        }
        source.push_str(
            " WHERE NOT EXISTS(SELECT 1 FROM tombstones tomb WHERE tomb.trace_id=trace.trace_id)",
        );
        let mut values = Vec::new();
        if let Some(id) = query.trace_id {
            source.push_str(" AND trace.trace_id=?");
            values.push(Value::Integer(Self::id(id)?));
        }
        if let Some(name) = &query.name {
            source.push_str(" AND trace.display_name=?");
            values.push(Value::Text(name.clone()));
        }
        if let Some(start) = query.start_time {
            source.push_str(" AND trace.created_at>=?");
            values.push(Value::Integer(Self::time(start)?));
        }
        if let Some(end) = query.end_time {
            source.push_str(" AND trace.created_at<=?");
            values.push(Value::Integer(Self::time(end)?));
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(Self::error)?;
        let total: u64 = transaction
            .query_row(
                &format!("SELECT COUNT(*){source}"),
                params_from_iter(&values),
                |row| row.get(0),
            )
            .map_err(Self::error)?;
        values.push(Value::Integer(
            i64::try_from(query.limit).map_err(Self::error)?,
        ));
        values.push(Value::Integer(offset));
        let traces = transaction.prepare(&format!("SELECT trace.*{source} ORDER BY trace.created_at DESC,trace.trace_id DESC LIMIT ? OFFSET ?"))
            .map_err(Self::error)?.query_map(params_from_iter(&values), trace_from_row)
            .map_err(Self::error)?.collect::<Result<Vec<_>, _>>().map_err(Self::error)?;
        transaction.commit().map_err(Self::error)?;
        Ok(TracePage {
            traces,
            page: query.page,
            total,
            total_pages: total.div_ceil(query.limit as u64),
        })
    }

    fn time(value: SystemTime) -> Result<i64, StorageError> {
        value
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
            .ok_or_else(|| Self::error("timestamp outside SQLite nanosecond range"))
    }

    fn id(value: TraceId) -> Result<i64, StorageError> {
        i64::try_from(value.get()).map_err(Self::error)
    }

    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("trace_page", error.to_string())
    }
}
