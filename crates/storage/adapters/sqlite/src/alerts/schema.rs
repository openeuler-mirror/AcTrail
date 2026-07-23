//! Alert-specific SQLite schema and validation.

use rusqlite::Connection;

pub(crate) const CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS alert_definitions (
    alert_definition_id INTEGER PRIMARY KEY,
    producer_plugin_id TEXT NOT NULL,
    definition_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    severity_code INTEGER NOT NULL,
    payload_schema_id TEXT NOT NULL,
    UNIQUE (producer_plugin_id, definition_key)
);

CREATE TABLE IF NOT EXISTS trace_alert_authorizations (
    trace_id INTEGER PRIMARY KEY,
    alert_token BLOB NOT NULL CHECK (length(alert_token) = 32)
);

CREATE TRIGGER IF NOT EXISTS reject_trace_alert_token_rotation
BEFORE INSERT ON traces
WHEN EXISTS (
    SELECT 1 FROM trace_alert_authorizations
    WHERE trace_id = NEW.trace_id AND alert_token != NEW.alert_token
)
BEGIN
    SELECT RAISE(ABORT, 'trace alert token cannot change');
END;

CREATE TRIGGER IF NOT EXISTS persist_trace_alert_authorization
AFTER INSERT ON traces
BEGIN
    INSERT OR IGNORE INTO trace_alert_authorizations (trace_id, alert_token)
    VALUES (NEW.trace_id, NEW.alert_token);
END;

CREATE TABLE IF NOT EXISTS alerts (
    alert_id INTEGER PRIMARY KEY,
    trace_id INTEGER NOT NULL,
    alert_definition_id INTEGER NOT NULL REFERENCES alert_definitions(alert_definition_id),
    created_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_alerts_latest
ON alerts(created_at DESC, alert_id DESC);

CREATE INDEX IF NOT EXISTS idx_alerts_trace_latest
ON alerts(trace_id, created_at DESC, alert_id DESC);
"#;

pub(crate) fn validate(connection: &Connection) -> Result<(), rusqlite::Error> {
    SelfValidator::new(connection)
        .require_column("alert_definitions", "alert_definition_id")?
        .require_column("alert_definitions", "producer_plugin_id")?
        .require_column("alert_definitions", "definition_key")?
        .require_column("alert_definitions", "payload_schema_id")?
        .require_column("trace_alert_authorizations", "trace_id")?
        .require_column("trace_alert_authorizations", "alert_token")?
        .require_column("alerts", "alert_id")?
        .require_column("alerts", "trace_id")?
        .require_column("alerts", "alert_definition_id")?
        .require_column("alerts", "created_at")?
        .require_column("alerts", "payload_json")?;
    Ok(())
}

struct SelfValidator<'connection> {
    connection: &'connection Connection,
}

impl<'connection> SelfValidator<'connection> {
    const fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    fn require_column(self, table: &str, column: &str) -> Result<Self, rusqlite::Error> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        for row in rows {
            if row? == column {
                return Ok(self);
            }
        }
        Err(rusqlite::Error::InvalidQuery)
    }
}
