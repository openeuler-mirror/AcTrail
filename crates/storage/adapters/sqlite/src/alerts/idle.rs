use std::time::{Duration, SystemTime};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use storage_core::{IdleInterval, IdlePage, IdleQuery, StorageError};

use crate::{
    SqliteStorage,
    records::{decode_time, decode_trace_lifecycle},
};

impl SqliteStorage {
    pub(crate) fn read_idle_page(&self, query: &IdleQuery) -> Result<IdlePage, StorageError> {
        if self.is_purged(query.trace_id) {
            return Err(StorageError::new("idle_page", "trace has been purged"));
        }
        IdleReader {
            connection: &self.connection().borrow(),
            query,
        }
        .read()
    }
}

struct IdleReader<'a> {
    connection: &'a Connection,
    query: &'a IdleQuery,
}

struct Episode {
    key: String,
    session: String,
    task: String,
    start: SystemTime,
    end: Option<SystemTime>,
}

impl Episode {
    fn parse(payload: &str) -> Result<Self, StorageError> {
        let value: Value = serde_json::from_str(payload).map_err(IdleReader::error)?;
        Ok(Self {
            key: Self::text(&value, "episode_key")?,
            session: Self::text(&value, "session_id")?,
            task: Self::text(&value, "task_id")?,
            start: Self::time(&Self::text(&value, "started_at_unix_nanos")?)?,
            end: match value.get("ended_at_unix_nanos") {
                None | Some(Value::Null) => None,
                Some(Value::String(raw)) => Some(Self::time(raw)?),
                Some(_) => {
                    return Err(StorageError::new(
                        "idle_page",
                        "invalid episode end timestamp",
                    ));
                }
            },
        })
    }

    fn text(value: &Value, key: &str) -> Result<String, StorageError> {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| StorageError::new("idle_page", format!("invalid episode field {key}")))
    }

    fn time(raw: &str) -> Result<SystemTime, StorageError> {
        let nanos: u128 = raw.parse().map_err(IdleReader::error)?;
        let seconds = u64::try_from(nanos / 1_000_000_000).map_err(IdleReader::error)?;
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::new(seconds, (nanos % 1_000_000_000) as u32))
            .ok_or_else(|| StorageError::new("idle_page", "episode timestamp overflow"))
    }
}

impl IdleReader<'_> {
    fn read(&self) -> Result<IdlePage, StorageError> {
        let query = self.query;
        if query.limit == 0 || query.candidate_limit < query.limit {
            return Err(StorageError::new(
                "idle_page",
                "limit must be positive and candidate_limit must be at least limit",
            ));
        }
        if query.from.zip(query.to).is_some_and(|(from, to)| from > to) {
            return Err(StorageError::new("idle_page", "window start exceeds end"));
        }
        let after = i64::try_from(query.after.unwrap_or(0)).map_err(Self::error)?;
        let budget = i64::try_from(query.candidate_limit).map_err(Self::error)?;
        let terminal = self.terminal()?;
        let mut page = IdlePage::empty(terminal.unwrap_or_else(SystemTime::now));
        let Some(detected) = self.definition("hang_detected")? else {
            return Ok(page);
        };
        let recovered = self.definition("hang_recovered")?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT alert_id,payload_json FROM alerts INDEXED BY idx_alerts_trace_definition_id
             WHERE trace_id=?1 AND alert_definition_id=?2 AND alert_id>?3
             ORDER BY alert_id ASC LIMIT ?4",
            )
            .map_err(Self::error)?;
        let mut rows = statement
            .query(params![query.trace_id.get(), detected, after, budget])
            .map_err(Self::error)?;
        let mut last = None;
        while page.scanned < query.candidate_limit && page.intervals.len() < query.limit {
            let Some(row) = rows.next().map_err(Self::error)? else {
                break;
            };
            let id: u64 = row.get(0).map_err(Self::error)?;
            let episode = Episode::parse(&row.get::<_, String>(1).map_err(Self::error)?)?;
            page.scanned += 1;
            last = Some(id);
            // Duplicate delivery of one episode must not create multiple intervals.
            if self.first_detected(detected, &episode.key)? != Some(id) {
                continue;
            }
            if terminal.is_some_and(|end| episode.start >= end) {
                continue;
            }
            let recovered_end = recovered
                .map(|definition| self.recovery(definition, id, &episode))
                .transpose()?
                .flatten();
            let end = match (recovered_end, terminal) {
                (Some(recovered), Some(terminal)) => Some(recovered.min(terminal)),
                (Some(recovered), None) => Some(recovered),
                (None, terminal) => terminal,
            };
            if end.is_some_and(|end| end <= episode.start)
                || query
                    .from
                    .is_some_and(|from| end.is_some_and(|end| end < from))
                || query.to.is_some_and(|to| episode.start > to)
            {
                continue;
            }
            page.intervals.push(IdleInterval {
                id: format!("agent-hang-{id}"),
                session_id: episode.session,
                task_id: episode.task,
                start_time: episode.start,
                end_time: end,
            });
        }
        // Avoid an extra read solely to prove exhaustion. A full boundary can yield
        // one empty terminal page, including when every candidate was filtered out.
        page.has_more =
            page.scanned == query.candidate_limit || page.intervals.len() == query.limit;
        page.next_cursor = page.has_more.then_some(last).flatten();
        Ok(page)
    }

    fn definition(&self, key: &str) -> Result<Option<u64>, StorageError> {
        self.connection.query_row(
            "SELECT alert_definition_id FROM alert_definitions WHERE producer_plugin_id='actrail.agent-idle' AND definition_key=?1",
            [key], |row| row.get(0)).optional().map_err(Self::error)
    }

    fn first_detected(&self, definition: u64, episode: &str) -> Result<Option<u64>, StorageError> {
        self.connection.prepare_cached(
            "SELECT alert_id FROM alerts INDEXED BY idx_alerts_trace_definition_episode
             WHERE trace_id=?1 AND alert_definition_id=?2 AND json_extract(payload_json,'$.episode_key')=?3
             ORDER BY alert_id ASC LIMIT 1").map_err(Self::error)?
            .query_row(params![self.query.trace_id.get(),definition,episode], |row| row.get(0))
            .optional().map_err(Self::error)
    }

    fn recovery(
        &self,
        definition: u64,
        detected: u64,
        episode: &Episode,
    ) -> Result<Option<SystemTime>, StorageError> {
        let payload: Option<String> = self.connection.prepare_cached(
            "SELECT payload_json FROM alerts INDEXED BY idx_alerts_trace_definition_episode
             WHERE trace_id=?1 AND alert_definition_id=?2 AND json_extract(payload_json,'$.episode_key')=?3 AND alert_id>?4
             ORDER BY alert_id ASC LIMIT 1").map_err(Self::error)?
            .query_row(params![self.query.trace_id.get(),definition,&episode.key,detected], |row| row.get(0))
            .optional().map_err(Self::error)?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let recovery = Episode::parse(&payload)?;
        if recovery.key != episode.key
            || recovery.session != episode.session
            || recovery.task != episode.task
            || recovery.start != episode.start
        {
            return Err(StorageError::new(
                "idle_page",
                "recovery episode identity differs from detected episode",
            ));
        }
        recovery.end.map(Some).ok_or_else(|| {
            StorageError::new("idle_page", "recovery episode is missing end timestamp")
        })
    }

    fn terminal(&self) -> Result<Option<SystemTime>, StorageError> {
        self.connection.query_row(
            "SELECT lifecycle_state,completed_at,exited_at,failed_at FROM traces WHERE trace_id=?1",
            [self.query.trace_id.get()], |row| {
                let state = decode_trace_lifecycle(&row.get::<_,String>(0)?)?;
                let times = [row.get::<_,Option<i64>>(1)?,row.get::<_,Option<i64>>(2)?,row.get::<_,Option<i64>>(3)?];
                Ok(state.is_terminal().then(|| times.into_iter().flatten().min().map(decode_time)).flatten())
            }).map_err(Self::error)
    }

    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("idle_page", error.to_string())
    }
}
