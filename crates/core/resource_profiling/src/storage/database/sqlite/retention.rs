use super::ResourceRetentionConfig;
use crate::storage::ResourceStorageError;
use std::time::{Duration, Instant, SystemTime};
pub(super) struct ResourceRetention {
    config: ResourceRetentionConfig,
    last_sweep: Option<Instant>,
}
impl ResourceRetention {
    pub(super) fn new(config: ResourceRetentionConfig) -> Self {
        Self {
            config,
            last_sweep: None,
        }
    }
    pub(super) fn sweep(
        &mut self,
        connection: &rusqlite::Connection,
    ) -> Result<(), ResourceStorageError> {
        if !self.config.enabled
            || self.last_sweep.is_some_and(|at| {
                at.elapsed() < Duration::from_secs(self.config.sweep_interval_seconds)
            })
        {
            return Ok(());
        }
        self.last_sweep = Some(Instant::now());
        let Some(before) =
            SystemTime::now().checked_sub(Duration::from_secs(self.config.max_age_seconds))
        else {
            return Ok(());
        };
        let before = super::ResourceStorageSqliteBackend::timestamp(before)?;
        connection.execute("DELETE FROM resource_samples WHERE id IN (SELECT id FROM resource_samples WHERE observed_at_ns < ?1 ORDER BY observed_at_ns LIMIT ?2)", rusqlite::params![before, self.config.max_samples_per_sweep])?;
        Ok(())
    }
}
