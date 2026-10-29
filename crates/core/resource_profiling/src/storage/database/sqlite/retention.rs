use super::ResourceRetentionConfig;
use crate::storage::ResourceStorageError;
use std::time::{Duration, Instant, SystemTime};

/// The two sample tables share this budget and continue on the next write.
const SWEEP_BUDGET: Duration = Duration::from_millis(50);
const RECLAIM_INTERVAL: Duration = Duration::from_secs(3600);

pub(super) struct ResourceRetention {
    config: ResourceRetentionConfig,
    last_sweep: Option<Instant>,
    last_reclaim: Option<Instant>,
    /// The last sweep ran out of budget with expired rows left.
    behind: bool,
}

impl ResourceRetention {
    pub(super) fn new(config: ResourceRetentionConfig) -> Self {
        Self {
            config,
            last_sweep: None,
            last_reclaim: None,
            behind: false,
        }
    }

    /// Returns whether cached profiles/boot keys may have been reclaimed.
    pub(super) fn sweep(
        &mut self,
        connection: &rusqlite::Connection,
    ) -> Result<bool, ResourceStorageError> {
        if !self.config.enabled
            || (!self.behind
                && self.last_sweep.is_some_and(|at| {
                    at.elapsed() < Duration::from_secs(self.config.sweep_interval_seconds)
                }))
        {
            return Ok(false);
        }
        let started = Instant::now();
        self.last_sweep = Some(started);
        // An error also leaves the sweep pending for the next write.
        self.behind = true;
        let Some(before) =
            SystemTime::now().checked_sub(Duration::from_secs(self.config.max_age_seconds))
        else {
            return Ok(false);
        };
        let before = super::ResourceStorageSqliteBackend::timestamp(before)?;
        let batch = self.config.max_samples_per_sweep;
        for table in ["resource_workload_samples", "resource_host_samples"] {
            let mut delete = connection.prepare_cached(&format!(
                "DELETE FROM {table} WHERE id IN (SELECT id FROM {table} WHERE observed_at_ns < ?1 ORDER BY observed_at_ns LIMIT ?2)"
            ))?;
            loop {
                if started.elapsed() >= SWEEP_BUDGET {
                    return Ok(false);
                }
                let deleted = delete.execute(rusqlite::params![before, batch])?;
                if deleted < batch as usize {
                    break;
                }
            }
        }
        self.behind = false;
        if self
            .last_reclaim
            .is_some_and(|at| at.elapsed() < RECLAIM_INTERVAL)
        {
            return Ok(false);
        }
        let profiles = connection.prepare_cached(
            "DELETE FROM resource_trace_profiles WHERE NOT EXISTS
                (SELECT 1 FROM resource_workload_samples s WHERE s.trace_id=resource_trace_profiles.trace_id)"
        )?.execute([])?;
        let boots = connection.prepare_cached(
            "DELETE FROM resource_host_boots WHERE NOT EXISTS
                (SELECT 1 FROM resource_host_samples s WHERE s.boot_key=resource_host_boots.boot_key)"
        )?.execute([])?;
        self.last_reclaim = Some(Instant::now());
        Ok(profiles != 0 || boots != 0)
    }
}
