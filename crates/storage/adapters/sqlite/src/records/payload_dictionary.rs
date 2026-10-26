//! Bounded single-writer interning for exact event payload bytes.

use std::collections::{HashMap, HashSet};

use rusqlite::params;
use store_write_contract::WriteError;

const ENTRY_OVERHEAD_BYTES: usize = 32;

pub(crate) enum StoredEventPayload {
    Dictionary(i64),
    Inline(Vec<u8>),
}

#[derive(Default)]
struct TracePayloads {
    entries: HashMap<Box<[u8]>, i64>,
    retained_bytes: usize,
}

pub(crate) struct EventPayloadDictionary {
    traces: HashMap<u64, TracePayloads>,
    retained_bytes: usize,
    capacity_bytes: usize,
    transaction_entries: Option<Vec<(u64, i64)>>,
    transaction_terminal_traces: HashSet<u64>,
}

impl EventPayloadDictionary {
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
        payload: Vec<u8>,
    ) -> Result<StoredEventPayload, WriteError> {
        if let Some(payload_id) = self
            .traces
            .get(&trace_id)
            .and_then(|trace| trace.entries.get(payload.as_slice()))
        {
            return Ok(StoredEventPayload::Dictionary(*payload_id));
        }
        let entry_bytes = entry_size(payload.len());
        if self.capacity_bytes == 0
            || entry_bytes > self.capacity_bytes.saturating_sub(self.retained_bytes)
        {
            return Ok(StoredEventPayload::Inline(payload));
        }
        connection
            .execute(
                "INSERT INTO event_payload_dictionary (trace_id, payload) VALUES (?1, ?2)",
                params![trace_id, &payload],
            )
            .map_err(|error| {
                WriteError::new("insert_event_payload_dictionary", error.to_string())
            })?;
        let payload_id = connection.last_insert_rowid();
        let trace = self.traces.entry(trace_id).or_default();
        trace.retained_bytes = trace.retained_bytes.saturating_add(entry_bytes);
        trace.entries.insert(payload.into_boxed_slice(), payload_id);
        self.retained_bytes = self.retained_bytes.saturating_add(entry_bytes);
        if let Some(entries) = self.transaction_entries.as_mut() {
            entries.push((trace_id, payload_id));
        }
        Ok(StoredEventPayload::Dictionary(payload_id))
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
        for (trace_id, payload_id) in entries {
            by_trace.entry(trace_id).or_default().insert(payload_id);
        }
        for (trace_id, payload_ids) in by_trace {
            let mut trace_is_empty = false;
            if let Some(trace) = self.traces.get_mut(&trace_id) {
                let mut removed_bytes = 0usize;
                trace.entries.retain(|payload, payload_id| {
                    if payload_ids.contains(payload_id) {
                        removed_bytes = removed_bytes.saturating_add(entry_size(payload.len()));
                        false
                    } else {
                        true
                    }
                });
                trace.retained_bytes = trace.retained_bytes.saturating_sub(removed_bytes);
                self.retained_bytes = self.retained_bytes.saturating_sub(removed_bytes);
                trace_is_empty = trace.entries.is_empty();
            }
            if trace_is_empty {
                self.traces.remove(&trace_id);
            }
        }
    }

    pub(crate) fn remove_trace(&mut self, trace_id: u64) {
        if let Some(trace) = self.traces.remove(&trace_id) {
            self.retained_bytes = self.retained_bytes.saturating_sub(trace.retained_bytes);
        }
    }

    pub(crate) fn finish_trace(&mut self, trace_id: u64) {
        if self.transaction_entries.is_some() {
            self.transaction_terminal_traces.insert(trace_id);
        } else {
            self.remove_trace(trace_id);
        }
    }
}

const fn entry_size(payload_bytes: usize) -> usize {
    payload_bytes.saturating_add(ENTRY_OVERHEAD_BYTES)
}
