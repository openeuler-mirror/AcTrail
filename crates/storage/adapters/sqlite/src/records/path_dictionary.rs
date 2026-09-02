//! Shared, bounded single-writer interner for trace-local paths.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};
use store_write_contract::WriteError;

const ENTRY_OVERHEAD_BYTES: usize = 32;

#[derive(Default)]
struct TracePaths {
    entries: HashMap<Box<str>, i64>,
    retained_bytes: usize,
}

pub(crate) struct PathInterner {
    traces: HashMap<u64, TracePaths>,
    retained_bytes: usize,
    capacity_bytes: usize,
    transaction_entries: Option<Vec<(u64, i64)>>,
    transaction_terminal_traces: HashSet<u64>,
}

impl PathInterner {
    pub(crate) fn new(capacity_bytes: usize) -> Self {
        Self {
            traces: HashMap::new(),
            retained_bytes: 0,
            capacity_bytes,
            transaction_entries: None,
            transaction_terminal_traces: HashSet::new(),
        }
    }

    pub(crate) fn intern(
        &mut self,
        connection: &rusqlite::Connection,
        trace_id: u64,
        path: &str,
    ) -> Result<i64, WriteError> {
        if let Some(path_id) = self
            .traces
            .get(&trace_id)
            .and_then(|trace| trace.entries.get(path))
        {
            return Ok(*path_id);
        }
        let entry_bytes = entry_size(path.len());
        let path_id = connection
            .prepare_cached("SELECT path_id FROM file_paths WHERE trace_id = ?1 AND path_text = ?2")
            .and_then(|mut statement| {
                statement
                    .query_row(params![trace_id, &path], |row| row.get::<_, i64>(0))
                    .optional()
            })
            .map_err(|error| WriteError::new("read_event_path_id", error.to_string()))?;
        let path_id = match path_id {
            Some(path_id) => path_id,
            None => {
                connection
                    .execute(
                        "INSERT INTO file_paths (trace_id, path_text) VALUES (?1, ?2)",
                        params![trace_id, &path],
                    )
                    .map_err(|error| {
                        WriteError::new("insert_event_payload_path", error.to_string())
                    })?;
                connection.last_insert_rowid()
            }
        };
        if self.capacity_bytes != 0
            && entry_bytes <= self.capacity_bytes.saturating_sub(self.retained_bytes)
        {
            let trace = self.traces.entry(trace_id).or_default();
            trace.retained_bytes = trace.retained_bytes.saturating_add(entry_bytes);
            trace.entries.insert(path.into(), path_id);
            self.retained_bytes = self.retained_bytes.saturating_add(entry_bytes);
            if let Some(entries) = self.transaction_entries.as_mut() {
                entries.push((trace_id, path_id));
            }
        }
        Ok(path_id)
    }

    pub(crate) fn begin_transaction(&mut self) {
        debug_assert!(self.transaction_entries.is_none());
        self.transaction_entries = Some(Vec::new());
        self.transaction_terminal_traces.clear();
    }

    pub(crate) fn commit_transaction(&mut self) {
        self.transaction_entries = None;
        for trace_id in std::mem::take(&mut self.transaction_terminal_traces) {
            self.remove_trace(trace_id);
        }
    }

    pub(crate) fn rollback_transaction(&mut self) {
        let Some(entries) = self.transaction_entries.take() else {
            return;
        };
        self.transaction_terminal_traces.clear();
        let mut by_trace = HashMap::<u64, HashSet<i64>>::new();
        for (trace_id, path_id) in entries {
            by_trace.entry(trace_id).or_default().insert(path_id);
        }
        for (trace_id, path_ids) in by_trace {
            let mut empty = false;
            if let Some(trace) = self.traces.get_mut(&trace_id) {
                let mut removed_bytes = 0usize;
                trace.entries.retain(|path, path_id| {
                    if path_ids.contains(path_id) {
                        removed_bytes = removed_bytes.saturating_add(entry_size(path.len()));
                        false
                    } else {
                        true
                    }
                });
                trace.retained_bytes = trace.retained_bytes.saturating_sub(removed_bytes);
                self.retained_bytes = self.retained_bytes.saturating_sub(removed_bytes);
                empty = trace.entries.is_empty();
            }
            if empty {
                self.traces.remove(&trace_id);
            }
        }
    }

    pub(crate) fn finish_trace(&mut self, trace_id: u64) {
        if self.transaction_entries.is_some() {
            self.transaction_terminal_traces.insert(trace_id);
        } else {
            self.remove_trace(trace_id);
        }
    }

    pub(crate) fn remove_trace(&mut self, trace_id: u64) {
        if let Some(trace) = self.traces.remove(&trace_id) {
            self.retained_bytes = self.retained_bytes.saturating_sub(trace.retained_bytes);
        }
    }
}

const fn entry_size(path_bytes: usize) -> usize {
    path_bytes.saturating_add(ENTRY_OVERHEAD_BYTES)
}
