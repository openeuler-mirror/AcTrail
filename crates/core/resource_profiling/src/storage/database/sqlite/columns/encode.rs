use super::{
    HOST_COLUMNS, MEMORY_EVENTS, MEMORY_EVENTS_LOCAL, SOURCE_OVERRIDE, TraceProfile,
    WORKLOAD_COLUMNS, extra_codec,
};
use crate::{
    model::{
        ResourceAccountingCoverage, ResourceAccountingMethod, ResourceMetrics, ResourceSample,
        ResourceSampleKind,
    },
    storage::ResourceStorageError,
};
use model_core::host_identity::HostBootId;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Encoder {
    boots: HashMap<HostBootId, i64>,
    profiles: HashMap<i64, TraceProfile>,
    keys: HashMap<String, u64>,
}

impl Encoder {
    /// A rolled-back transaction may have populated any of these caches.
    pub(crate) fn clear(&mut self) {
        self.clear_profiles_and_boots();
        self.keys.clear();
    }

    pub(crate) fn clear_profiles_and_boots(&mut self) {
        self.profiles.clear();
        self.boots.clear();
    }

    pub(crate) fn write(
        &mut self,
        connection: &Connection,
        samples: &[ResourceSample],
        started: Instant,
        timeout: Duration,
    ) -> Result<(), ResourceStorageError> {
        let mut host_insert = connection.prepare_cached(&format!(
            "INSERT INTO resource_host_samples ({HOST_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
        ))?;
        let mut workload_insert = connection.prepare_cached(&format!(
            "INSERT INTO resource_workload_samples ({WORKLOAD_COLUMNS}) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,
                ?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37)"
        ))?;
        for sample in samples {
            if started.elapsed() >= timeout {
                return Err(ResourceStorageError("resource SQLite batch timeout".into()));
            }
            let observed_at =
                super::super::ResourceStorageSqliteBackend::timestamp(sample.observed_at())?;
            match sample {
                ResourceSample::Host(sample) => {
                    let boot = self.boot(connection, sample.host_boot_id)?;
                    host_insert.execute(params![
                        boot,
                        observed_at,
                        integer(sample.cpu_percent_millis),
                        sample.cpu_count,
                        sample.memory_total_bytes as i64,
                        sample.memory_available_bytes as i64,
                        sample.memory_free_bytes as i64,
                        sample.load_average[0],
                        sample.load_average[1],
                        sample.load_average[2],
                    ])?;
                }
                ResourceSample::Workload(sample) => {
                    let trace = sample.trace_id.get() as i64;
                    let payload = &sample.payload;
                    let mut metadata = payload.metadata.clone();
                    let alert = match metadata.get("alert").map(String::as_str) {
                        Some("false") => Some(0),
                        Some("true") => Some(1),
                        Some("unknown") => Some(2),
                        _ => None,
                    };
                    if alert.is_some() {
                        metadata.remove("alert");
                    }
                    let interval = metadata
                        .get("cpu_rate_interval_usec")
                        .and_then(|value| extra_codec::canonical_integer(value));
                    if interval.is_some() {
                        metadata.remove("cpu_rate_interval_usec");
                    }

                    if !self.profiles.contains_key(&trace) {
                        let profile = match TraceProfile::load(connection, trace)? {
                            Some(profile) => profile,
                            None => {
                                connection.prepare_cached(
                                    "INSERT INTO resource_trace_profiles (trace_id,scope,subject,metadata) VALUES (?1,?2,?3,?4)"
                                )?.execute(params![trace, payload.scope, payload.subject, serde_json::to_string(&metadata)?])?;
                                TraceProfile {
                                    scope: payload.scope.clone(),
                                    subject: payload.subject.clone(),
                                    metadata: metadata.clone(),
                                }
                            }
                        };
                        self.profiles.insert(trace, profile);
                    }
                    let profile = &self.profiles[&trace];
                    let source = (profile.scope != payload.scope
                        || profile.subject != payload.subject)
                        .then_some((payload.scope.as_str(), payload.subject.as_str()));
                    let mut changed = Vec::new();
                    // Split the borrow so dictionary inserts do not require cloning the profile.
                    for (key, value) in &metadata {
                        if profile.metadata.get(key) != Some(value) {
                            changed.push((
                                Self::key(connection, &mut self.keys, key)?,
                                value.as_str(),
                            ));
                        }
                    }
                    let mut removed = Vec::new();
                    for key in profile.metadata.keys() {
                        if !metadata.contains_key(key) {
                            removed.push(Self::key(connection, &mut self.keys, key)?);
                        }
                    }
                    let extra = extra_codec::encode(source, &changed, &removed);
                    let flags = flags(payload) | if source.is_some() { SOURCE_OVERRIDE } else { 0 };
                    let events = payload.memory_events.as_ref();
                    let local = payload.memory_events_local.as_ref();
                    workload_insert.execute(params![
                        trace,
                        observed_at,
                        sample.process.get() as i64,
                        flags,
                        alert,
                        integer(interval),
                        integer(payload.cpu_percent_millis),
                        integer(payload.rss_kb),
                        integer(payload.virtual_memory_kb),
                        integer(payload.memory_current_bytes),
                        integer(payload.memory_peak_bytes),
                        integer(payload.memory_anon_bytes),
                        integer(payload.memory_file_bytes),
                        integer(payload.memory_swap_current_bytes),
                        integer(payload.cpu_usage_usec),
                        integer(payload.cpu_user_usec),
                        integer(payload.cpu_system_usec),
                        integer(payload.cpu_nr_throttled),
                        integer(payload.cpu_throttled_usec),
                        integer(payload.io_read_bytes),
                        integer(payload.io_write_bytes),
                        integer(payload.pids_current),
                        integer(payload.pids_peak),
                        integer(payload.process_rss_sum_kb),
                        integer(events.and_then(|e| e.low)),
                        integer(events.and_then(|e| e.high)),
                        integer(events.and_then(|e| e.max)),
                        integer(events.and_then(|e| e.oom)),
                        integer(events.and_then(|e| e.oom_kill)),
                        integer(events.and_then(|e| e.oom_group_kill)),
                        integer(local.and_then(|e| e.low)),
                        integer(local.and_then(|e| e.high)),
                        integer(local.and_then(|e| e.max)),
                        integer(local.and_then(|e| e.oom)),
                        integer(local.and_then(|e| e.oom_kill)),
                        integer(local.and_then(|e| e.oom_group_kill)),
                        extra,
                    ])?;
                    if payload.sample_kind == ResourceSampleKind::Final {
                        self.profiles.remove(&trace);
                    }
                }
            }
        }
        Ok(())
    }

    fn boot(
        &mut self,
        connection: &Connection,
        boot: HostBootId,
    ) -> Result<i64, ResourceStorageError> {
        if let Some(&key) = self.boots.get(&boot) {
            return Ok(key);
        }
        connection
            .prepare_cached("INSERT OR IGNORE INTO resource_host_boots (boot_id) VALUES (?1)")?
            .execute([boot.as_bytes().as_slice()])?;
        let key = connection
            .prepare_cached("SELECT boot_key FROM resource_host_boots WHERE boot_id=?1")?
            .query_row([boot.as_bytes().as_slice()], |row| row.get(0))?;
        self.boots.insert(boot, key);
        Ok(key)
    }

    fn key(
        connection: &Connection,
        keys: &mut HashMap<String, u64>,
        key: &str,
    ) -> Result<u64, ResourceStorageError> {
        if let Some(&id) = keys.get(key) {
            return Ok(id);
        }
        let stored = connection
            .prepare_cached("SELECT key_id FROM resource_metadata_keys WHERE key=?1")?
            .query_row([key], |row| row.get::<_, i64>(0))
            .optional()?;
        let id = match stored {
            Some(id) => id,
            None => {
                connection
                    .prepare_cached("INSERT INTO resource_metadata_keys (key) VALUES (?1)")?
                    .execute([key])?;
                connection.last_insert_rowid()
            }
        } as u64;
        keys.insert(key.to_owned(), id);
        Ok(id)
    }
}

fn integer(value: Option<u64>) -> Option<i64> {
    value.map(|value| value as i64)
}

fn flags(payload: &ResourceMetrics) -> i64 {
    let method = match payload.accounting_method {
        ResourceAccountingMethod::CgroupV1 => 1,
        ResourceAccountingMethod::CgroupV2 => 2,
        ResourceAccountingMethod::ProcfsRssSum => 3,
    };
    let coverage = match payload.accounting_coverage {
        ResourceAccountingCoverage::Exact => 1,
        ResourceAccountingCoverage::BroaderThanTrace => 2,
        ResourceAccountingCoverage::Partial => 3,
    };
    let kind = match payload.sample_kind {
        ResourceSampleKind::Periodic => 1,
        ResourceSampleKind::Final => 2,
    };
    method
        | coverage << 2
        | kind << 4
        | if payload.memory_events.is_some() {
            MEMORY_EVENTS
        } else {
            0
        }
        | if payload.memory_events_local.is_some() {
            MEMORY_EVENTS_LOCAL
        } else {
            0
        }
}
