use std::collections::{HashMap, HashSet};

use model_core::event::DomainEvent;
use rusqlite::{OptionalExtension, params};
use store_write_contract::WriteError;

use crate::config::{EventRecordLayout, SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES};

use super::codec::{CODEC_VERSION, KIND_COUNT, compress_block, encode_event, encode_kind_counts};

#[derive(Clone, Copy)]
struct PendingState {
    event_count: usize,
    framed_bytes: usize,
}

impl PendingState {
    fn uncompressed_bytes(self) -> usize {
        1usize
            .saturating_add(varint_len(self.event_count as u64))
            .saturating_add(self.framed_bytes)
    }
}

struct PendingBlock {
    frames: Vec<Vec<u8>>,
    framed_bytes: usize,
    first_event_id: u64,
    max_event_id: u64,
    min_observed_at: i64,
    max_observed_at: i64,
    kind_counts: [u64; KIND_COUNT],
}

impl PendingBlock {
    fn new(capacity: usize) -> Self {
        Self {
            frames: Vec::with_capacity(capacity),
            framed_bytes: 0,
            first_event_id: u64::MAX,
            max_event_id: 0,
            min_observed_at: i64::MAX,
            max_observed_at: i64::MIN,
            kind_counts: [0; KIND_COUNT],
        }
    }

    fn would_overflow(
        &self,
        frame_bytes: usize,
        max_events: usize,
        max_uncompressed_bytes: usize,
    ) -> bool {
        self.frames.len() >= max_events
            || block_bytes(
                self.frames.len().saturating_add(1),
                self.framed_bytes
                    .saturating_add(framed_event_bytes(frame_bytes)),
            ) > max_uncompressed_bytes
    }

    fn push(
        &mut self,
        event_id: u64,
        observed_at: i64,
        kind_index: usize,
        frame: Vec<u8>,
    ) -> Result<(), WriteError> {
        let Some(kind_count) = self.kind_counts.get_mut(kind_index) else {
            return Err(WriteError::new(
                "read_event_record_pending",
                "persisted event tail has an unknown kind code",
            ));
        };
        *kind_count = kind_count.saturating_add(1);
        self.first_event_id = self.first_event_id.min(event_id);
        self.max_event_id = self.max_event_id.max(event_id);
        self.min_observed_at = self.min_observed_at.min(observed_at);
        self.max_observed_at = self.max_observed_at.max(observed_at);
        self.framed_bytes = self
            .framed_bytes
            .saturating_add(framed_event_bytes(frame.len()));
        self.frames.push(frame);
        Ok(())
    }

    fn persist(
        &mut self,
        connection: &rusqlite::Connection,
        trace_id: u64,
        zstd_level: i32,
    ) -> Result<(), WriteError> {
        if self.frames.is_empty() {
            return Ok(());
        }
        let uncompressed_bytes = block_bytes(self.frames.len(), self.framed_bytes);
        let compressed = compress_block(&self.frames, zstd_level)
            .map_err(|error| WriteError::new("compress_event_record_block", error.to_string()))?;
        connection
            .execute(
                "INSERT INTO event_record_blocks (
                    trace_id, first_event_id, max_event_id, min_observed_at, max_observed_at,
                    event_count, kind_counts, codec_version, uncompressed_bytes, encoded_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    trace_id,
                    self.first_event_id,
                    self.max_event_id,
                    self.min_observed_at,
                    self.max_observed_at,
                    self.frames.len(),
                    encode_kind_counts(&self.kind_counts),
                    CODEC_VERSION,
                    uncompressed_bytes,
                    compressed,
                ],
            )
            .map_err(|error| WriteError::new("insert_event_record_block", error.to_string()))?;
        self.frames.clear();
        self.framed_bytes = 0;
        self.first_event_id = u64::MAX;
        self.max_event_id = 0;
        self.min_observed_at = i64::MAX;
        self.max_observed_at = i64::MIN;
        self.kind_counts.fill(0);
        Ok(())
    }
}

pub(crate) struct EventRecordBlockWriter {
    layout: EventRecordLayout,
    max_events: usize,
    max_uncompressed_bytes: usize,
    zstd_level: i32,
    transaction_active: bool,
    event_id_high_water: u64,
    transaction_start_event_id: Option<u64>,
    transaction_event_id_claims: HashMap<u64, u64>,
    terminal_traces: HashSet<u64>,
    poisoned: bool,
}

impl EventRecordBlockWriter {
    pub(crate) fn new(
        layout: EventRecordLayout,
        max_events: usize,
        max_uncompressed_bytes: usize,
        zstd_level: i32,
        event_id_high_water: u64,
    ) -> Self {
        Self {
            layout,
            max_events,
            max_uncompressed_bytes,
            zstd_level,
            transaction_active: false,
            event_id_high_water,
            transaction_start_event_id: None,
            transaction_event_id_claims: HashMap::new(),
            terminal_traces: HashSet::new(),
            poisoned: false,
        }
    }

    pub(crate) const fn enabled(&self) -> bool {
        matches!(self.layout, EventRecordLayout::Blocks)
    }

    pub(crate) fn observe_event_id(
        &mut self,
        connection: &rusqlite::Connection,
        event_id: u64,
    ) -> Result<(), WriteError> {
        if self.poisoned {
            return Err(WriteError::new(
                "observe_event_id",
                "event writer is poisoned after an uncertain transaction rollback",
            ));
        }
        if !self.transaction_active {
            return Err(WriteError::new(
                "observe_event_id",
                "event id observation requires an active storage transaction",
            ));
        }
        let transaction_start = self.transaction_start_event_id.ok_or_else(|| {
            WriteError::new(
                "claim_event_id",
                "active event transaction is missing its high-water snapshot",
            )
        })?;
        let word_id = event_id / EVENT_ID_CLAIM_WORD_BITS;
        let bit = 1u64 << (event_id % EVENT_ID_CLAIM_WORD_BITS);
        let transaction_claims = self
            .transaction_event_id_claims
            .get(&word_id)
            .copied()
            .unwrap_or(0);
        if transaction_claims & bit != 0 {
            return Err(WriteError::new(
                "claim_event_id",
                format!("event id {event_id} is duplicated in the current transaction"),
            ));
        }
        if event_id <= transaction_start {
            let claimed_bits = connection
                .query_row(
                    "SELECT claimed_bits FROM event_id_claim_words WHERE word_id = ?1",
                    [word_id],
                    |row| row.get::<_, u64>(0),
                )
                .optional()
                .map_err(|error| WriteError::new("read_event_id_claim", error.to_string()))?
                .unwrap_or(0);
            let persisted_row = self.enabled()
                && connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?1)",
                        [event_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|error| {
                        WriteError::new("find_row_event_id_claim", error.to_string())
                    })?;
            if claimed_bits & bit != 0 || persisted_row {
                return Err(WriteError::new(
                    "claim_event_id",
                    format!("event id {event_id} is already persisted"),
                ));
            }
        }
        if self.enabled() {
            self.transaction_event_id_claims
                .insert(word_id, transaction_claims | bit);
        }
        self.event_id_high_water = self.event_id_high_water.max(event_id);
        Ok(())
    }

    pub(crate) fn begin_transaction(
        &mut self,
        connection: &rusqlite::Connection,
    ) -> Result<(), WriteError> {
        if self.poisoned {
            return Err(WriteError::new(
                "begin_event_transaction",
                "event writer is poisoned after an uncertain transaction rollback",
            ));
        }
        debug_assert!(!self.transaction_active);
        debug_assert!(self.transaction_event_id_claims.is_empty());
        debug_assert!(self.terminal_traces.is_empty());
        let persisted = connection
            .query_row(
                "SELECT last_event_id FROM event_id_high_water WHERE singleton = 1",
                [],
                |row| row.get::<_, u64>(0),
            )
            .map_err(|error| WriteError::new("read_event_id_high_water", error.to_string()))?;
        self.event_id_high_water = persisted;
        self.transaction_start_event_id = Some(persisted);
        self.transaction_active = true;
        Ok(())
    }

    pub(crate) fn append(
        &mut self,
        connection: &rusqlite::Connection,
        event: DomainEvent,
        payload_path_id: Option<i64>,
    ) -> Result<(), WriteError> {
        if !self.transaction_active {
            return Err(WriteError::new(
                "append_event_record_block",
                "event block append requires an active storage transaction",
            ));
        }
        let trace_id = event.envelope.trace_id.get();
        let encoded = encode_event(event, payload_path_id)
            .map_err(|error| WriteError::new("encode_event_record_block", error.to_string()))?;
        let frame_bytes = framed_event_bytes(encoded.bytes.len());
        let single_block_bytes = block_bytes(1, frame_bytes);
        if single_block_bytes > self.max_uncompressed_bytes {
            return Err(WriteError::new(
                "append_event_record_block",
                format!(
                    "encoded event requires {} bytes, above configured block limit {}",
                    single_block_bytes, self.max_uncompressed_bytes
                ),
            ));
        }

        if let Some(state) = self.pending_state(connection, trace_id)? {
            let next_count = state.event_count.saturating_add(1);
            let next_framed_bytes = state.framed_bytes.saturating_add(frame_bytes);
            if state.event_count >= self.max_events
                || block_bytes(next_count, next_framed_bytes) > self.max_uncompressed_bytes
            {
                self.flush_trace(connection, trace_id)?;
            }
        }

        connection
            .execute(
                "INSERT INTO event_record_pending (
                    event_id, trace_id, observed_at, kind_code, encoded_frame
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    encoded.event_id,
                    trace_id,
                    encoded.observed_at,
                    encoded.kind_index,
                    encoded.bytes,
                ],
            )
            .map_err(|error| WriteError::new("insert_event_record_pending", error.to_string()))?;
        let state = connection
            .query_row(
                "INSERT INTO event_record_pending_state (
                    trace_id, event_count, framed_bytes
                 ) VALUES (?1, 1, ?2)
                 ON CONFLICT(trace_id) DO UPDATE SET
                    event_count = event_count + 1,
                    framed_bytes = framed_bytes + excluded.framed_bytes
                 RETURNING event_count, framed_bytes",
                params![trace_id, frame_bytes],
                |row| {
                    Ok(PendingState {
                        event_count: row.get(0)?,
                        framed_bytes: row.get(1)?,
                    })
                },
            )
            .map_err(|error| WriteError::new("update_event_record_pending", error.to_string()))?;
        if state.event_count >= self.max_events
            || state.uncompressed_bytes() >= self.max_uncompressed_bytes
        {
            self.flush_trace(connection, trace_id)?;
        }
        Ok(())
    }

    pub(crate) fn has_pending_trace(
        &self,
        connection: &rusqlite::Connection,
        trace_id: u64,
    ) -> Result<bool, WriteError> {
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM event_record_pending_state WHERE trace_id = ?1
                 )",
                [trace_id],
                |row| row.get(0),
            )
            .map_err(|error| WriteError::new("find_event_record_pending", error.to_string()))
    }

    pub(crate) fn mark_terminal_trace(&mut self, trace_id: u64) -> Result<(), WriteError> {
        if !self.transaction_active {
            return Err(WriteError::new(
                "mark_terminal_trace",
                "terminal trace marking requires an active storage transaction",
            ));
        }
        self.terminal_traces.insert(trace_id);
        Ok(())
    }

    pub(crate) fn flush_terminal_traces(
        &mut self,
        connection: &rusqlite::Connection,
    ) -> Result<(), WriteError> {
        let traces = self.terminal_traces.iter().copied().collect::<Vec<_>>();
        for trace_id in traces {
            self.flush_trace(connection, trace_id)?;
        }
        Ok(())
    }

    pub(crate) fn flush_trace(
        &mut self,
        connection: &rusqlite::Connection,
        trace_id: u64,
    ) -> Result<(), WriteError> {
        if !self.transaction_active {
            return Err(WriteError::new(
                "flush_event_record_pending",
                "event tail compaction requires an active storage transaction",
            ));
        }
        let Some(state) = self.pending_state(connection, trace_id)? else {
            return Ok(());
        };
        if state.event_count == 0 || state.framed_bytes == 0 {
            return Err(WriteError::new(
                "flush_event_record_pending",
                "persisted event tail has invalid empty metadata",
            ));
        }

        let mut statement = connection
            .prepare(
                "SELECT event_id, observed_at, kind_code,
                        length(encoded_frame), encoded_frame
                 FROM event_record_pending
                 WHERE trace_id = ?1
                 ORDER BY event_id ASC",
            )
            .map_err(|error| WriteError::new("prepare_event_record_pending", error.to_string()))?;
        let mut rows = statement
            .query([trace_id])
            .map_err(|error| WriteError::new("query_event_record_pending", error.to_string()))?;
        let mut block = PendingBlock::new(self.max_events.min(state.event_count));
        let mut actual_event_count = 0usize;
        let mut actual_framed_bytes = 0usize;
        while let Some(row) = rows
            .next()
            .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?
        {
            let event_id = row
                .get::<_, u64>(0)
                .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?;
            let observed_at = row
                .get::<_, i64>(1)
                .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?;
            let kind_index = row
                .get::<_, usize>(2)
                .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?;
            let encoded_length = row
                .get::<_, usize>(3)
                .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?;
            if encoded_length == 0
                || block_bytes(1, framed_event_bytes(encoded_length))
                    > SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES
            {
                return Err(WriteError::new(
                    "read_event_record_pending",
                    "persisted pending event exceeds the codec safety boundary",
                ));
            }
            let frame = row
                .get::<_, Vec<u8>>(4)
                .map_err(|error| WriteError::new("read_event_record_pending", error.to_string()))?;
            if frame.len() != encoded_length {
                return Err(WriteError::new(
                    "read_event_record_pending",
                    "persisted pending event length changed while reading",
                ));
            }
            if !block.frames.is_empty()
                && block.would_overflow(frame.len(), self.max_events, self.max_uncompressed_bytes)
            {
                block.persist(connection, trace_id, self.zstd_level)?;
            }
            actual_event_count = actual_event_count.saturating_add(1);
            actual_framed_bytes =
                actual_framed_bytes.saturating_add(framed_event_bytes(frame.len()));
            block.push(event_id, observed_at, kind_index, frame)?;
        }
        drop(rows);
        drop(statement);
        if actual_event_count != state.event_count || actual_framed_bytes != state.framed_bytes {
            return Err(WriteError::new(
                "read_event_record_pending",
                "persisted event tail metadata does not match its frames",
            ));
        }
        block.persist(connection, trace_id, self.zstd_level)?;
        connection
            .execute(
                "DELETE FROM event_record_pending WHERE trace_id = ?1",
                [trace_id],
            )
            .map_err(|error| WriteError::new("delete_event_record_pending", error.to_string()))?;
        connection
            .execute(
                "DELETE FROM event_record_pending_state WHERE trace_id = ?1",
                [trace_id],
            )
            .map_err(|error| {
                WriteError::new("delete_event_record_pending_state", error.to_string())
            })?;
        Ok(())
    }

    pub(crate) fn persist_transaction_state(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<(), WriteError> {
        let Some(transaction_start) = self.transaction_start_event_id else {
            return Err(WriteError::new(
                "persist_event_transaction_state",
                "event high-water persistence requires an active storage transaction",
            ));
        };
        for (word_id, claimed_bits) in &self.transaction_event_id_claims {
            connection
                .execute(
                    "INSERT INTO event_id_claim_words (word_id, claimed_bits)
                     VALUES (?1, ?2)
                     ON CONFLICT(word_id) DO UPDATE SET
                        claimed_bits = event_id_claim_words.claimed_bits | excluded.claimed_bits",
                    params![word_id, claimed_bits],
                )
                .map_err(|error| WriteError::new("persist_event_id_claim", error.to_string()))?;
        }
        if self.event_id_high_water == transaction_start {
            return Ok(());
        }
        connection
            .execute(
                "UPDATE event_id_high_water SET last_event_id = ?1 WHERE singleton = 1",
                [self.event_id_high_water],
            )
            .and_then(|updated| {
                if updated == 1 {
                    Ok(())
                } else {
                    Err(rusqlite::Error::InvalidQuery)
                }
            })
            .map_err(|error| WriteError::new("persist_event_id_high_water", error.to_string()))
    }

    pub(crate) fn commit_transaction(&mut self) {
        self.transaction_active = false;
        self.transaction_start_event_id = None;
        self.transaction_event_id_claims.clear();
        self.terminal_traces.clear();
    }

    pub(crate) fn rollback_transaction(&mut self) {
        self.transaction_active = false;
        self.transaction_event_id_claims.clear();
        self.terminal_traces.clear();
        if let Some(event_id) = self.transaction_start_event_id.take() {
            self.event_id_high_water = event_id;
        }
    }

    pub(crate) fn poison_transaction(&mut self) {
        self.transaction_active = false;
        self.transaction_start_event_id = None;
        self.transaction_event_id_claims.clear();
        self.terminal_traces.clear();
        self.poisoned = true;
    }

    fn pending_state(
        &self,
        connection: &rusqlite::Connection,
        trace_id: u64,
    ) -> Result<Option<PendingState>, WriteError> {
        connection
            .query_row(
                "SELECT event_count, framed_bytes
                 FROM event_record_pending_state WHERE trace_id = ?1",
                [trace_id],
                |row| {
                    Ok(PendingState {
                        event_count: row.get(0)?,
                        framed_bytes: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|error| WriteError::new("read_event_record_pending_state", error.to_string()))
    }
}

const EVENT_ID_CLAIM_WORD_BITS: u64 = 63;

const fn framed_event_bytes(frame_bytes: usize) -> usize {
    varint_len(frame_bytes as u64).saturating_add(frame_bytes)
}

const fn block_bytes(event_count: usize, framed_bytes: usize) -> usize {
    1usize
        .saturating_add(varint_len(event_count as u64))
        .saturating_add(framed_bytes)
}

const fn varint_len(mut value: u64) -> usize {
    let mut bytes = 1usize;
    while value >= 0x80 {
        value >>= 7;
        bytes += 1;
    }
    bytes
}
