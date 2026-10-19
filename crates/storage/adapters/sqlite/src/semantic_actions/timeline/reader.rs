//! Index-bounded action start-time windows and trace-wide timestamp endpoints.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::ids::TraceId;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkRole, SemanticActionStatus,
};
use storage_core::{StorageError, TimelineBounds, TimelineCursor, TimelinePage, TimelineQuery};

use crate::SqliteStorage;
use crate::records::decode_time;
use crate::semantic_actions::codebook::sqlite::{
    action_kind_code, decode_link_origin, decode_link_role,
};
use crate::semantic_actions::store::action_from_row;

// The existing row decoder accepts absent cold attributes and hydrates only these hot facts.
const COLUMNS: &str = "ids.action_id, action.trace_id, action.kind_code,
 COALESCE(state.failure_title, action.title) AS stored_title, action.file_path_id,
 (SELECT path_text FROM file_paths WHERE trace_id=action.trace_id AND path_id=action.file_path_id) AS file_path_text,
 action.start_time, state.end_time, action.process_id, state.status_code, state.completeness_code,
 state.finalization_reason, state.failure_body_format, state.failure_http_status, state.failure_http_reason,
 state.command_invocation_kind, state.command_tool_name, state.tool_result_binding,
 NULL AS attributes_encoding_code";

impl SqliteStorage {
    pub(crate) fn read_timeline_bounds(
        &self,
        trace_id: TraceId,
    ) -> Result<TimelineBounds, StorageError> {
        if self.is_purged(trace_id) {
            return Err(StorageError::new("timeline", "trace has been purged"));
        }
        // Separate scalar subqueries seek the two index endpoints in one read snapshot.
        // Combining MIN and MAX in one aggregate would scan all of the trace's rows.
        self.connection()
            .borrow()
            .query_row(
                "SELECT
             (SELECT start_time FROM semantic_actions INDEXED BY idx_semantic_actions_trace_start
              WHERE trace_id=?1 ORDER BY start_time ASC,action_key ASC LIMIT 1),
             (SELECT start_time FROM semantic_actions INDEXED BY idx_semantic_actions_trace_start
              WHERE trace_id=?1 ORDER BY start_time DESC,action_key DESC LIMIT 1)",
                [trace_id.get()],
                |row| {
                    Ok(TimelineBounds {
                        first_start_time: row.get::<_, Option<i64>>(0)?.map(decode_time),
                        last_start_time: row.get::<_, Option<i64>>(1)?.map(decode_time),
                    })
                },
            )
            .map_err(TimelineReader::error)
    }

    pub(crate) fn read_timeline_page(
        &self,
        query: &TimelineQuery,
    ) -> Result<TimelinePage, StorageError> {
        if self.is_purged(query.trace_id) {
            return Err(StorageError::new("timeline", "trace has been purged"));
        }
        TimelineReader {
            connection: &self.connection().borrow(),
            query,
        }
        .read()
    }
}

struct Candidate {
    key: i64,
    start: i64,
}

struct TimelineReader<'a> {
    connection: &'a Connection,
    query: &'a TimelineQuery,
}

impl TimelineReader<'_> {
    fn read(&self) -> Result<TimelinePage, StorageError> {
        let query = self.query;
        if query.limit == 0 || query.candidate_limit == 0 || query.link_limit == 0 {
            return Err(StorageError::new("timeline", "limits must be positive"));
        }
        if query
            .start_time
            .zip(query.end_time)
            .is_some_and(|(start, end)| start > end)
        {
            return Err(StorageError::new("timeline", "window start exceeds end"));
        }
        if let Some(start) = query.start_time {
            Self::time(start)?;
        }
        let kinds: BTreeSet<_> = query.kinds.iter().copied().map(action_kind_code).collect();
        let ranges: Vec<_> = if kinds.is_empty() {
            vec![None]
        } else {
            kinds.into_iter().map(Some).collect()
        };
        let per_range = query.candidate_limit / ranges.len();
        if per_range == 0 {
            return Err(StorageError::new(
                "timeline",
                "candidate limit must cover each selected kind",
            ));
        }
        let mut candidates = Vec::new();
        let mut boundary = None;
        let mut scanned = 0;
        for kind in ranges {
            let rows = self.candidates(kind, per_range)?;
            scanned += rows.len();
            if rows.len() == per_range {
                if let Some(last) = rows.last() {
                    let end = (last.start, last.key);
                    boundary = Some(boundary.map_or(end, |previous: (i64, i64)| previous.min(end)));
                }
            }
            candidates.extend(rows);
        }
        candidates.sort_unstable_by_key(|candidate| (candidate.start, candidate.key));
        // A full range might have another row immediately after its last row. Only
        // advance through the common safe prefix, preserving every kind's ordering.
        // The sum of all index reads remains <= candidate_limit.
        candidates
            .retain(|candidate| boundary.is_none_or(|end| (candidate.start, candidate.key) <= end));
        let mut page = TimelinePage {
            scanned,
            ..TimelinePage::default()
        };
        let mut consumed = 0;
        let mut selected_keys = Vec::new();
        for candidate in &candidates {
            if page.actions.len() == query.limit {
                break;
            }
            consumed += 1;
            if let Some(mut action) = self.action(candidate.key)? {
                // Calls own their request start; completed response owns the displayed outcome.
                page.links_truncated |= self.call_outcome(candidate.key, &mut action)?;
                selected_keys.push(candidate.key);
                page.actions.push(action);
            }
        }
        if consumed < candidates.len() || boundary.is_some() {
            if let Some(last) = candidates.get(consumed.saturating_sub(1)) {
                page.next_cursor = Some(TimelineCursor {
                    start_time: decode_time(last.start),
                    action_key: last.key,
                });
            }
        }
        self.relationships(&selected_keys, &mut page)?;
        Ok(page)
    }

    fn time(value: SystemTime) -> Result<i64, StorageError> {
        value
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
            .ok_or_else(|| {
                StorageError::new("timeline", "timestamp outside SQLite nanosecond range")
            })
    }

    fn candidates(&self, kind: Option<i16>, limit: usize) -> Result<Vec<Candidate>, StorageError> {
        let query = self.query;
        let mut values = vec![Value::Integer(
            i64::try_from(query.trace_id.get()).map_err(Self::error)?,
        )];
        let index = match (query.process.is_some(), kind.is_some()) {
            (true, true) => "idx_semantic_actions_timeline_process_kind",
            (true, false) => "idx_semantic_actions_timeline_process",
            (false, true) => "idx_semantic_actions_trace_kind_start",
            // action_key is INTEGER PRIMARY KEY, the implicit final index column.
            (false, false) => "idx_semantic_actions_trace_start",
        };
        let mut sql = format!(
            "SELECT action_key,start_time FROM semantic_actions INDEXED BY {index} WHERE trace_id=?"
        );
        if let Some(process) = &query.process {
            sql.push_str(" AND process_id=?");
            values.push(Value::Integer(
                i64::try_from(process.get()).map_err(Self::error)?,
            ));
        }
        if let Some(kind) = kind {
            sql.push_str(" AND kind_code=?");
            values.push(Value::Integer(i64::from(kind)));
        }
        if let Some(after) = query.after {
            if after.action_key <= 0 {
                return Err(StorageError::new(
                    "timeline",
                    "cursor action key must be positive",
                ));
            }
            sql.push_str(" AND (start_time,action_key)>(?,?)");
            values.push(Value::Integer(Self::time(after.start_time)?));
            values.push(Value::Integer(after.action_key));
        }
        if let Some(start) = query.start_time {
            sql.push_str(" AND start_time>=?");
            values.push(Value::Integer(Self::time(start)?));
        }
        if let Some(end) = query.end_time {
            sql.push_str(" AND start_time<=?");
            values.push(Value::Integer(Self::time(end)?));
        }
        sql.push_str(" ORDER BY start_time,action_key LIMIT ?");
        values.push(Value::Integer(i64::try_from(limit).map_err(Self::error)?));
        let mut statement = self.connection.prepare(&sql).map_err(Self::error)?;
        statement
            .query_map(params_from_iter(values), |row| {
                Ok(Candidate {
                    key: row.get(0)?,
                    start: row.get(1)?,
                })
            })
            .map_err(Self::error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Self::error)
    }

    fn action(&self, key: i64) -> Result<Option<SemanticAction>, StorageError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM semantic_actions action
            JOIN semantic_action_ids ids ON ids.action_key=action.action_key
            JOIN semantic_action_state state ON state.action_key=action.action_key
            WHERE action.trace_id=?1 AND action.action_key=?2 AND action.action_valid_code=1"
        );
        self.connection
            .prepare_cached(&sql)
            .map_err(Self::error)?
            .query_row(params![self.query.trace_id.get(), key], action_from_row)
            .optional()
            .map_err(Self::error)
    }

    fn call_outcome(&self, key: i64, call: &mut SemanticAction) -> Result<bool, StorageError> {
        if call.kind != SemanticActionKind::LlmCall || call.status != SemanticActionStatus::Unknown
        {
            return Ok(false);
        }
        // A call has one authoritative response edge. Limit applies before endpoint
        // validation so malformed/missing endpoints cannot cause an unbounded scan.
        let role = crate::semantic_actions::codebook::sqlite::link_role_code(
            SemanticActionLinkRole::LlmCallResponse,
        );
        let mut statement = self.connection.prepare_cached(
            "SELECT child_action_key,valid FROM semantic_action_links INDEXED BY idx_semantic_action_links_timeline_parent_role
             WHERE trace_id=?1 AND parent_action_key=?2 AND role_code=?3 LIMIT 2").map_err(Self::error)?;
        let children = statement
            .query_map(params![self.query.trace_id.get(), key, role], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(Self::error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Self::error)?;
        if children.len() > 1 || children.first().is_some_and(|(_, valid)| !valid) {
            return Ok(true);
        }
        if let Some((child, true)) = children.first() {
            if let Some(response) = self.action(*child)? {
                if response.kind == SemanticActionKind::LlmResponse
                    && response.process == call.process
                    && response.completeness == SemanticActionCompleteness::Complete
                    && matches!(
                        response.status,
                        SemanticActionStatus::Success | SemanticActionStatus::Error
                    )
                    && response.end_time.is_some_and(|end| end >= call.start_time)
                {
                    call.status = response.status;
                    call.end_time = response.end_time;
                }
            } else {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn relationships(&self, selected: &[i64], page: &mut TimelinePage) -> Result<(), StorageError> {
        let selected_set: BTreeSet<_> = selected.iter().copied().collect();
        let mut endpoint_keys = BTreeSet::new();
        let mut edges = BTreeMap::new();
        let mut scanned = 0usize;
        for key in selected {
            for incoming in [true, false] {
                if scanned >= self.query.link_limit {
                    page.links_truncated = true;
                    break;
                }
                let endpoint = if incoming {
                    "child_action_key"
                } else {
                    "parent_action_key"
                };
                let index = if incoming {
                    "idx_semantic_action_links_trace_child_role"
                } else {
                    "idx_semantic_action_links_timeline_parent_role"
                };
                let sql = format!(
                    "SELECT parent_action_key,child_action_key,role_code,origin_code,valid
                    FROM semantic_action_links INDEXED BY {index} WHERE trace_id=? AND {endpoint}=? LIMIT ?"
                );
                let remaining = self.query.link_limit - scanned;
                let mut statement = self.connection.prepare_cached(&sql).map_err(Self::error)?;
                let rows = statement
                    .query_map(params![self.query.trace_id.get(), key, remaining], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, bool>(4)?,
                        ))
                    })
                    .map_err(Self::error)?;
                for row in rows {
                    let (parent, child, role, origin, valid) = row.map_err(Self::error)?;
                    if scanned == self.query.link_limit {
                        page.links_truncated = true;
                        break;
                    }
                    scanned += 1;
                    if valid {
                        endpoint_keys.extend([parent, child]);
                        edges.insert((parent, child, role), origin);
                    }
                }
                // A full budget conservatively reports partial, including the case
                // where this happened to be the last edge. No extra edge is read.
                page.links_truncated |= scanned == self.query.link_limit;
            }
        }
        let mut actions_by_key = BTreeMap::new();
        for (key, action) in selected.iter().zip(&page.actions) {
            actions_by_key.insert(*key, action.clone());
        }
        for key in endpoint_keys.difference(&selected_set) {
            if page.context_actions.len() >= self.query.link_limit {
                page.links_truncated = true;
                break;
            }
            if let Some(mut action) = self.action(*key)? {
                page.links_truncated |= self.call_outcome(*key, &mut action)?;
                page.context_actions.push(action.clone());
                actions_by_key.insert(*key, action);
            } else {
                page.links_truncated = true;
            }
        }
        for ((parent, child, role), origin) in edges {
            let (Some(parent), Some(child)) =
                (actions_by_key.get(&parent), actions_by_key.get(&child))
            else {
                page.links_truncated = true;
                continue;
            };
            page.links.push(SemanticActionLink {
                trace_id: self.query.trace_id,
                parent_action_id: parent.action_id.clone(),
                child_action_id: child.action_id.clone(),
                role: decode_link_role(role).map_err(Self::error)?,
                origin: decode_link_origin(origin).map_err(Self::error)?,
                valid: true,
                evidence: Vec::new(),
                attributes: BTreeMap::new(),
            });
        }
        Ok(())
    }

    fn error(error: impl std::fmt::Display) -> StorageError {
        StorageError::new("timeline", error.to_string())
    }
}
