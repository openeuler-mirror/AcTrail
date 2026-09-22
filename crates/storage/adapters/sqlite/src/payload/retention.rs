//! A bounded, connection-owned cache derived exclusively from stored bytes.

use std::collections::BTreeMap;

use model_core::payload::PayloadSourceBoundary;
use rusqlite::{Connection, params};
use storage_core::PayloadRetentionLimits;

#[derive(Default)]
pub(crate) struct PayloadRetentionState {
    limits: Option<PayloadRetentionLimits>,
    capacity: usize,
    counts: BTreeMap<u64, [u64; 3]>,
}

/// Persistent storage facts, independent of collector capture completeness.
#[derive(Clone, Copy)]
pub(super) enum StorageOmission {
    None = 0,
    RetentionLimit = 1,
}

impl PayloadRetentionState {
    pub(crate) fn configure(&mut self, limits: PayloadRetentionLimits, capacity: usize) {
        self.limits = Some(limits);
        self.capacity = capacity;
        self.invalidate();
    }

    pub(crate) fn invalidate(&mut self) {
        self.counts.clear();
    }

    pub(crate) fn forget_trace(&mut self, trace_id: u64) {
        self.counts.remove(&trace_id);
    }

    pub(super) fn source_index(source: PayloadSourceBoundary) -> usize {
        match source {
            PayloadSourceBoundary::TlsUserSpace => 0,
            PayloadSourceBoundary::Syscall => 1,
            PayloadSourceBoundary::Stdio => 2,
        }
    }

    pub(super) fn retained_bytes(
        &mut self,
        connection: &Connection,
        trace_id: u64,
        source: PayloadSourceBoundary,
    ) -> rusqlite::Result<u64> {
        if self.limits.is_none() {
            return Ok(0);
        }
        if let Some(counts) = self.counts.get(&trace_id) {
            return Ok(counts[Self::source_index(source)]);
        }
        let mut counts = [0_u64; 3];
        let mut statement = connection.prepare_cached(
            "SELECT (segment_meta >> 2) & 3, COALESCE(SUM(length(bytes)), 0)
             FROM payload_segments WHERE trace_id = ?1 GROUP BY (segment_meta >> 2) & 3",
        )?;
        let mut rows = statement.query(params![trace_id])?;
        while let Some(row) = rows.next()? {
            let index = row.get::<_, usize>(0)?;
            let count = counts.get_mut(index).ok_or(rusqlite::Error::InvalidQuery)?;
            *count = row.get(1)?;
        }
        let retained = counts[Self::source_index(source)];
        if self.capacity != 0 {
            if self.counts.len() >= self.capacity {
                self.counts.pop_first();
            }
            self.counts.insert(trace_id, counts);
        }
        Ok(retained)
    }

    pub(super) fn select_body<'a>(
        &self,
        source: PayloadSourceBoundary,
        retained: u64,
        replaced: u64,
        bytes: &'a [u8],
    ) -> rusqlite::Result<(&'a [u8], StorageOmission)> {
        if bytes.is_empty() {
            return Ok((bytes, StorageOmission::None));
        }
        let Some(limits) = self.limits else {
            return Ok((bytes, StorageOmission::None));
        };
        let other = retained
            .checked_sub(replaced)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if u128::from(other) + bytes.len() as u128 > u128::from(limits.for_source(source)) {
            Ok((&[], StorageOmission::RetentionLimit))
        } else {
            Ok((bytes, StorageOmission::None))
        }
    }

    pub(super) fn record_write(
        &mut self,
        trace_id: u64,
        source: PayloadSourceBoundary,
        retained: u64,
        replaced: u64,
        written: usize,
    ) -> rusqlite::Result<()> {
        if let Some(counts) = self.counts.get_mut(&trace_id) {
            counts[Self::source_index(source)] = retained
                .checked_sub(replaced)
                .and_then(|value| value.checked_add(written as u64))
                .ok_or(rusqlite::Error::InvalidQuery)?;
        }
        Ok(())
    }
}
