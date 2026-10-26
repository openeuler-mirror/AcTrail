//! Cluster index identity and numbered trace metadata pages.
use super::{ClusterTraceRow, open_index, trace_row_json};
use rusqlite::{Connection, OptionalExtension, Row, params_from_iter, types::Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct ClusterTracePageQuery {
    pub page: usize,
    pub trace_id: Option<String>,
    pub name: Option<String>,
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
    pub limit: usize,
}

pub(super) struct ClusterIndex {
    connection: Connection,
}
impl ClusterIndex {
    pub(super) fn open(root: &Path) -> Result<Self, String> {
        Ok(Self {
            connection: open_index(root)?,
        })
    }
    pub(super) fn get(&self, uid: &str) -> Result<ClusterTraceRow, String> {
        if uid.is_empty() {
            return Err("cluster trace UID must not be empty".into());
        }
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM imported_traces WHERE trace_uid=?1"),
                [uid],
                Self::decode,
            )
            .optional()
            .map_err(|e| format!("read cluster trace: {e}"))?
            .ok_or_else(|| format!("cluster trace {uid} not found"))
    }
    pub(super) fn page(&self, query: &ClusterTracePageQuery) -> Result<String, String> {
        if query.page == 0 || query.limit == 0 {
            return Err("page and limit must be positive".into());
        }
        let offset = query
            .page
            .checked_sub(1)
            .and_then(|page| page.checked_mul(query.limit))
            .and_then(|offset| i64::try_from(offset).ok())
            .ok_or("trace page offset exceeds SQLite range")?;
        let mut source = " FROM imported_traces WHERE 1=1".to_string();
        let mut values = Vec::new();
        if let Some(uid) = &query.trace_id {
            source.push_str(" AND trace_uid=?");
            values.push(Value::Text(uid.clone()));
        }
        if let Some(name) = &query.name {
            source.push_str(" AND COALESCE(display_name,trace_uid)=?");
            values.push(Value::Text(name.clone()));
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        if query.start_time.is_some() || query.end_time.is_some() {
            // Imported timestamps are decimal millisecond strings. Reject malformed values
            // rather than let SQLite's permissive CAST silently change filter membership.
            let invalid: bool = transaction.query_row(&format!(
                "SELECT EXISTS(SELECT 1{source} AND created_unix_ms IS NOT NULL AND (created_unix_ms='' OR created_unix_ms GLOB '*[^0-9]*' OR length(ltrim(created_unix_ms,'0'))>19 OR (length(ltrim(created_unix_ms,'0'))=19 AND ltrim(created_unix_ms,'0')>'9223372036854775807')))"
            ), params_from_iter(&values), |row| row.get(0)).map_err(|e| e.to_string())?;
            if invalid {
                return Err("invalid cluster creation milliseconds".into());
            }
        }
        if query
            .start_time
            .zip(query.end_time)
            .is_some_and(|(start, end)| start > end)
        {
            return Err("trace creation time from must not exceed to".into());
        }
        if let Some(start) = query.start_time {
            let nanos = start
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            source.push_str(" AND CAST(created_unix_ms AS INTEGER)>=?");
            values.push(Value::Integer(
                i64::try_from(nanos.div_ceil(1_000_000)).map_err(|e| e.to_string())?,
            ));
        }
        if let Some(end) = query.end_time {
            let nanos = end
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            source.push_str(" AND CAST(created_unix_ms AS INTEGER)<=?");
            values.push(Value::Integer(
                i64::try_from(nanos / 1_000_000).map_err(|e| e.to_string())?,
            ));
        }
        let total: u64 = transaction
            .query_row(
                &format!("SELECT COUNT(*){source}"),
                params_from_iter(&values),
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        values.push(Value::Integer(
            i64::try_from(query.limit).map_err(|e| e.to_string())?,
        ));
        values.push(Value::Integer(offset));
        let rows = transaction
            .prepare(&format!(
                "SELECT {COLUMNS}{source} ORDER BY trace_uid ASC LIMIT ? OFFSET ?"
            ))
            .map_err(|e| e.to_string())?
            .query_map(params_from_iter(&values), Self::decode)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        let rendered = rows
            .iter()
            .map(trace_row_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!(
            "{{\"traces\":[{}],\"page\":{},\"total\":{},\"total_pages\":{}}}",
            rendered.join(","),
            query.page,
            total,
            total.div_ceil(query.limit as u64)
        ))
    }
    fn decode(row: &Row<'_>) -> Result<ClusterTraceRow, rusqlite::Error> {
        Ok(ClusterTraceRow {
            trace_uid: row.get(0)?,
            cluster_id: row.get(1)?,
            node_ip: row.get(2)?,
            node_id: row.get(3)?,
            node_name: row.get(4)?,
            local_trace_id: row.get(5)?,
            display_name: row.get(6)?,
            profile_name: row.get(7)?,
            lifecycle_state: row.get(8)?,
            health: row.get(9)?,
            created_unix_ms: row.get(10)?,
            started_unix_ms: row.get(11)?,
            completed_unix_ms: row.get(12)?,
            imported_at: row.get(13)?,
            bundle_path: row.get(14)?,
            graph_json_path: row.get(15)?,
            process_count: row.get::<_, i64>(16)?.max(0) as u64,
            event_count: row.get::<_, i64>(17)?.max(0) as u64,
            payload_segment_count: row.get::<_, i64>(18)?.max(0) as u64,
            retained_payload_bytes: row.get::<_, i64>(19)?.max(0) as u64,
            semantic_action_count: row.get::<_, i64>(20)?.max(0) as u64,
            semantic_link_count: row.get::<_, i64>(21)?.max(0) as u64,
            diagnostic_count: row.get::<_, i64>(22)?.max(0) as u64,
            manifest_json: row.get(23)?,
        })
    }
}
const COLUMNS: &str = "trace_uid, cluster_id, node_ip, node_id, node_name, local_trace_id, display_name, profile_name, lifecycle_state, health, created_unix_ms, started_unix_ms, completed_unix_ms, imported_at, bundle_path, graph_json_path, process_count, event_count, payload_segment_count, retained_payload_bytes, semantic_action_count, semantic_link_count, diagnostic_count, manifest_json";
