//! Idle interval SQLite schema and validation.

use rusqlite::Connection;

use crate::schema::require_column;

pub(crate) const CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS idle_intervals (
    interval_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    task_id TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER
);

CREATE INDEX IF NOT EXISTS idx_idle_intervals_trace_start
ON idle_intervals(trace_id, start_time, interval_id);

"#;

pub(crate) fn validate(connection: &Connection) -> Result<(), rusqlite::Error> {
    require_column(connection, "idle_intervals", "interval_id")?;
    require_column(connection, "idle_intervals", "trace_id")?;
    require_column(connection, "idle_intervals", "task_id")?;
    require_column(connection, "idle_intervals", "start_time")?;
    require_column(connection, "idle_intervals", "end_time")?;
    Ok(())
}
