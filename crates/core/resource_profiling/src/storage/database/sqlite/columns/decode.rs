use super::{
    HOST_COLUMNS, MEMORY_EVENTS, MEMORY_EVENTS_LOCAL, SOURCE_OVERRIDE, TraceProfile,
    WORKLOAD_COLUMNS, extra_codec,
};
use crate::{
    model::{
        MemoryEventCounters, ResourceAccountingCoverage, ResourceAccountingMethod, ResourceMetrics,
        ResourceSample, ResourceSampleKind, WorkloadResourceSample,
        host_resource::HostResourceSample,
    },
    storage::{ResourceStorageError, database::ResourceKind},
};
use model_core::{host_identity::HostBootId, ids::TraceId, process::ProcessIdentity};
use rusqlite::{Connection, Row};
use std::{
    collections::HashMap,
    time::{Duration, UNIX_EPOCH},
};

#[derive(Default)]
pub(crate) struct Decoder {
    profiles: HashMap<i64, TraceProfile>,
    boots: HashMap<i64, HostBootId>,
    keys: HashMap<u64, String>,
}

impl Decoder {
    pub(crate) fn clear_profiles_and_boots(&mut self) {
        self.profiles.clear();
        self.boots.clear();
    }

    pub(crate) fn columns(kind: ResourceKind) -> &'static str {
        match kind {
            ResourceKind::Host => HOST_COLUMNS,
            ResourceKind::Workload => WORKLOAD_COLUMNS,
        }
    }

    pub(crate) fn sample(
        &mut self,
        connection: &Connection,
        kind: ResourceKind,
        row: &Row<'_>,
    ) -> Result<ResourceSample, ResourceStorageError> {
        let at: i64 = row.get(1)?;
        let observed_at = u64::try_from(at)
            .ok()
            .and_then(|ns| UNIX_EPOCH.checked_add(Duration::from_nanos(ns)))
            .ok_or_else(|| ResourceStorageError("invalid resource timestamp".into()))?;
        match kind {
            ResourceKind::Host => {
                let boot = row.get(0)?;
                if !self.boots.contains_key(&boot) {
                    let bytes: Vec<u8> = connection
                        .prepare_cached(
                            "SELECT boot_id FROM resource_host_boots WHERE boot_key=?1",
                        )?
                        .query_row([boot], |row| row.get(0))?;
                    let bytes = bytes.try_into().map_err(|_| {
                        ResourceStorageError("invalid resource host boot ID".into())
                    })?;
                    self.boots.insert(boot, HostBootId::from_bytes(bytes));
                }
                Ok(ResourceSample::Host(HostResourceSample {
                    host_boot_id: self.boots[&boot],
                    observed_at,
                    cpu_percent_millis: integer(row, 2)?,
                    cpu_count: row.get(3)?,
                    memory_total_bytes: row.get::<_, i64>(4)? as u64,
                    memory_available_bytes: row.get::<_, i64>(5)? as u64,
                    memory_free_bytes: row.get::<_, i64>(6)? as u64,
                    load_average: [row.get(7)?, row.get(8)?, row.get(9)?],
                }))
            }
            ResourceKind::Workload => {
                let trace: i64 = row.get(0)?;
                if !self.profiles.contains_key(&trace) {
                    let profile = TraceProfile::load(connection, trace)?.ok_or_else(|| {
                        ResourceStorageError(format!(
                            "missing resource profile for trace {}",
                            trace as u64
                        ))
                    })?;
                    self.profiles.insert(trace, profile);
                }
                let profile = &self.profiles[&trace];
                let flags: i64 = row.get(3)?;
                let mut payload = ResourceMetrics {
                    scope: profile.scope.clone(),
                    subject: profile.subject.clone(),
                    accounting_method: match flags & 3 {
                        1 => ResourceAccountingMethod::CgroupV1,
                        2 => ResourceAccountingMethod::CgroupV2,
                        3 => ResourceAccountingMethod::ProcfsRssSum,
                        _ => return Err(invalid_flags()),
                    },
                    accounting_coverage: match flags >> 2 & 3 {
                        1 => ResourceAccountingCoverage::Exact,
                        2 => ResourceAccountingCoverage::BroaderThanTrace,
                        3 => ResourceAccountingCoverage::Partial,
                        _ => return Err(invalid_flags()),
                    },
                    sample_kind: match flags >> 4 & 3 {
                        1 => ResourceSampleKind::Periodic,
                        2 => ResourceSampleKind::Final,
                        _ => return Err(invalid_flags()),
                    },
                    cpu_percent_millis: integer(row, 6)?,
                    rss_kb: integer(row, 7)?,
                    virtual_memory_kb: integer(row, 8)?,
                    memory_current_bytes: integer(row, 9)?,
                    memory_peak_bytes: integer(row, 10)?,
                    memory_anon_bytes: integer(row, 11)?,
                    memory_file_bytes: integer(row, 12)?,
                    memory_swap_current_bytes: integer(row, 13)?,
                    cpu_usage_usec: integer(row, 14)?,
                    cpu_user_usec: integer(row, 15)?,
                    cpu_system_usec: integer(row, 16)?,
                    cpu_nr_throttled: integer(row, 17)?,
                    cpu_throttled_usec: integer(row, 18)?,
                    io_read_bytes: integer(row, 19)?,
                    io_write_bytes: integer(row, 20)?,
                    pids_current: integer(row, 21)?,
                    pids_peak: integer(row, 22)?,
                    process_rss_sum_kb: integer(row, 23)?,
                    memory_events: if flags & MEMORY_EVENTS != 0 {
                        Some(events(row, 24)?)
                    } else {
                        None
                    },
                    memory_events_local: if flags & MEMORY_EVENTS_LOCAL != 0 {
                        Some(events(row, 30)?)
                    } else {
                        None
                    },
                    metadata: profile.metadata.clone(),
                };
                if let Some(bytes) = row.get::<_, Option<Vec<u8>>>(36)? {
                    let extra = extra_codec::decode(&bytes, flags & SOURCE_OVERRIDE != 0)?;
                    if let Some((scope, subject)) = extra.source {
                        payload.scope = scope.to_owned();
                        payload.subject = subject.to_owned();
                    }
                    for (id, value) in extra.values {
                        payload
                            .metadata
                            .insert(self.key(connection, id)?.to_owned(), value.into_owned());
                    }
                    for id in extra.removed {
                        payload.metadata.remove(self.key(connection, id)?);
                    }
                } else if flags & SOURCE_OVERRIDE != 0 {
                    return Err(ResourceStorageError(
                        "missing resource source override".into(),
                    ));
                }
                if let Some(alert) = row.get::<_, Option<i64>>(4)? {
                    let value = match alert {
                        0 => "false",
                        1 => "true",
                        2 => "unknown",
                        _ => return Err(ResourceStorageError("invalid resource alert".into())),
                    };
                    payload.metadata.insert("alert".into(), value.into());
                }
                if let Some(interval) = integer(row, 5)? {
                    payload
                        .metadata
                        .insert("cpu_rate_interval_usec".into(), interval.to_string());
                }
                Ok(ResourceSample::Workload(WorkloadResourceSample {
                    trace_id: TraceId::new(trace as u64),
                    observed_at,
                    process: ProcessIdentity::new(row.get::<_, i64>(2)? as u64),
                    payload,
                }))
            }
        }
    }

    fn key(&mut self, connection: &Connection, id: u64) -> Result<&str, ResourceStorageError> {
        if !self.keys.contains_key(&id) {
            let key = connection
                .prepare_cached("SELECT key FROM resource_metadata_keys WHERE key_id=?1")?
                .query_row([id as i64], |row| row.get(0))?;
            self.keys.insert(id, key);
        }
        Ok(&self.keys[&id])
    }
}

fn integer(row: &Row<'_>, index: usize) -> Result<Option<u64>, ResourceStorageError> {
    Ok(row.get::<_, Option<i64>>(index)?.map(|value| value as u64))
}

fn events(row: &Row<'_>, start: usize) -> Result<MemoryEventCounters, ResourceStorageError> {
    Ok(MemoryEventCounters {
        low: integer(row, start)?,
        high: integer(row, start + 1)?,
        max: integer(row, start + 2)?,
        oom: integer(row, start + 3)?,
        oom_kill: integer(row, start + 4)?,
        oom_group_kill: integer(row, start + 5)?,
    })
}

fn invalid_flags() -> ResourceStorageError {
    ResourceStorageError("invalid resource sample flags".into())
}
