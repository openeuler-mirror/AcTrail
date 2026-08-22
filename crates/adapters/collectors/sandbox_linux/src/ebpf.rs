//! Private eBPF object loading and aggregate-map decoding.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, Instant};

use libbpf_rs::{Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder};
use sandbox_observation::{GuestBootId, ProcessIoCounters, ProcessMarker};

use crate::SandboxLinuxError;
use crate::collector::KernelCollectionDiagnostics;
use crate::config::SandboxLinuxConfig;
use crate::procfs::{ProcessLineageMember, ProcfsReader};

const ROOT_MARKER_SIZE: usize = 32;
const KERNEL_COUNTER_SIZE: usize = 48;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RootKey {
    pid: u32,
    start_time_ticks: u64,
    executable_name: [u8; 16],
}

impl From<ProcessMarker> for RootKey {
    fn from(marker: ProcessMarker) -> Self {
        Self {
            pid: marker.pid,
            start_time_ticks: marker.start_time_ticks,
            executable_name: marker.executable_name,
        }
    }
}

impl RootKey {
    fn marker(self) -> ProcessMarker {
        ProcessMarker {
            pid: self.pid,
            start_time_ticks: self.start_time_ticks,
            executable_name: self.executable_name,
        }
    }

    fn encode(self) -> [u8; ROOT_MARKER_SIZE] {
        let mut raw = [0_u8; ROOT_MARKER_SIZE];
        raw[0..4].copy_from_slice(&self.pid.to_ne_bytes());
        raw[8..16].copy_from_slice(&self.start_time_ticks.to_ne_bytes());
        raw[16..32].copy_from_slice(&self.executable_name);
        raw
    }

    fn decode(raw: &[u8]) -> Result<Self, SandboxLinuxError> {
        if raw.len() != ROOT_MARKER_SIZE {
            return Err(SandboxLinuxError::new(
                "decode_aggregate",
                format!("unexpected root marker size {}", raw.len()),
            ));
        }
        Ok(Self {
            pid: u32::from_ne_bytes(raw[0..4].try_into().expect("checked root marker size")),
            start_time_ticks: u64::from_ne_bytes(
                raw[8..16].try_into().expect("checked root marker size"),
            ),
            executable_name: raw[16..32].try_into().expect("checked root marker size"),
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct KernelCounters {
    read_operations: u64,
    read_bytes: u64,
    write_operations: u64,
    write_bytes: u64,
    failed_read_operations: u64,
    failed_write_operations: u64,
}

impl KernelCounters {
    fn decode(raw: &[u8]) -> Result<Self, SandboxLinuxError> {
        if raw.len() != KERNEL_COUNTER_SIZE {
            return Err(SandboxLinuxError::new(
                "decode_aggregate",
                format!("unexpected aggregate counter size {}", raw.len()),
            ));
        }
        let value = |offset: usize| {
            u64::from_ne_bytes(
                raw[offset..offset + 8]
                    .try_into()
                    .expect("checked aggregate counter size"),
            )
        };
        Ok(Self {
            read_operations: value(0),
            read_bytes: value(8),
            write_operations: value(16),
            write_bytes: value(24),
            failed_read_operations: value(32),
            failed_write_operations: value(40),
        })
    }

    fn delta(self, baseline: Self) -> Self {
        Self {
            read_operations: self
                .read_operations
                .saturating_sub(baseline.read_operations),
            read_bytes: self.read_bytes.saturating_sub(baseline.read_bytes),
            write_operations: self
                .write_operations
                .saturating_sub(baseline.write_operations),
            write_bytes: self.write_bytes.saturating_sub(baseline.write_bytes),
            failed_read_operations: self
                .failed_read_operations
                .saturating_sub(baseline.failed_read_operations),
            failed_write_operations: self
                .failed_write_operations
                .saturating_sub(baseline.failed_write_operations),
        }
    }

    fn is_empty(self) -> bool {
        self.read_operations == 0
            && self.read_bytes == 0
            && self.write_operations == 0
            && self.write_bytes == 0
            && self.failed_read_operations == 0
            && self.failed_write_operations == 0
    }
}

pub(crate) struct EbpfIoCollector {
    _object: Object,
    _links: Vec<Link>,
    tracked_processes: MapHandle,
    aggregates: MapHandle,
    diagnostics: MapHandle,
    root_process_names: Vec<[u8; 16]>,
    baselines: HashMap<RootKey, KernelCounters>,
    diagnostic_baseline: KernelCollectionDiagnostics,
    root_refresh_interval: Duration,
    next_root_refresh: Instant,
}

impl EbpfIoCollector {
    pub(crate) fn start(
        config: &SandboxLinuxConfig,
        procfs: &ProcfsReader,
    ) -> Result<Self, SandboxLinuxError> {
        let lineages = procfs.discover_lineages(&config.root_process_names)?;
        if config.require_initial_root && lineages.root_count == 0 {
            return Err(SandboxLinuxError::new(
                "discover_roots",
                format!(
                    "none of the configured root processes is running under {}",
                    procfs.root().display()
                ),
            ));
        }

        let object_bytes = include_bytes!(env!("ACTRAIL_SANDBOX_BPF_OBJECT"));
        let mut open_object = ObjectBuilder::default()
            .open_memory(object_bytes)
            .map_err(|error| SandboxLinuxError::new("open_bpf_object", error.to_string()))?;
        Self::resize_map(
            &mut open_object,
            "tracked_processes",
            config.tracked_process_capacity,
        )?;
        Self::resize_map(&mut open_object, "pending_io", config.pending_io_capacity)?;
        Self::resize_map(&mut open_object, "io_aggregates", config.aggregate_capacity)?;
        let object = open_object
            .load()
            .map_err(|error| SandboxLinuxError::new("load_bpf_object", error.to_string()))?;
        let tracked_processes = Self::map_handle(&object, "tracked_processes")?;
        let pending_io = Self::map_handle(&object, "pending_io")?;
        let aggregates = Self::map_handle(&object, "io_aggregates")?;
        let diagnostics = Self::map_handle(&object, "collection_diagnostics")?;
        Self::validate_map_layout(&tracked_processes, "tracked_processes", 4, ROOT_MARKER_SIZE)?;
        Self::validate_map_layout(&pending_io, "pending_io", 8, 40)?;
        Self::validate_map_layout(
            &aggregates,
            "io_aggregates",
            ROOT_MARKER_SIZE,
            KERNEL_COUNTER_SIZE,
        )?;
        Self::validate_map_layout(&diagnostics, "collection_diagnostics", 4, 8)?;
        let mut links = Vec::new();
        for program in object.progs_mut() {
            let name = program.name().to_string_lossy().into_owned();
            links.push(program.attach().map_err(|error| {
                SandboxLinuxError::new(
                    "attach_bpf_program",
                    format!("cannot attach {name}: {error}"),
                )
            })?);
        }
        if links.is_empty() {
            return Err(SandboxLinuxError::new(
                "attach_bpf_program",
                "sandbox eBPF object contains no attachable programs",
            ));
        }
        let collector = Self {
            _object: object,
            _links: links,
            tracked_processes,
            aggregates,
            diagnostics,
            root_process_names: config.root_process_names.clone(),
            baselines: HashMap::new(),
            diagnostic_baseline: KernelCollectionDiagnostics::default(),
            root_refresh_interval: config.root_refresh_interval,
            next_root_refresh: Instant::now() + config.root_refresh_interval,
        };
        collector.seed_lineages(&lineages.members)?;
        Ok(collector)
    }

    pub(crate) fn refresh_roots(&mut self, procfs: &ProcfsReader) -> Result<(), SandboxLinuxError> {
        let now = Instant::now();
        if now < self.next_root_refresh {
            return Ok(());
        }
        self.next_root_refresh = now + self.root_refresh_interval;
        let lineages = procfs.discover_lineages(&self.root_process_names)?;
        self.seed_lineages(&lineages.members)
    }

    pub(crate) fn collect(
        &mut self,
        boot_id: GuestBootId,
        sample_started_ms: u64,
        sample_ended_ms: u64,
    ) -> Result<(Vec<ProcessIoCounters>, KernelCollectionDiagnostics), SandboxLinuxError> {
        let keys = self.aggregates.keys().collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(keys.len());
        for raw_key in keys {
            let key = RootKey::decode(&raw_key)?;
            let Some(raw_value) = self
                .aggregates
                .lookup(&raw_key, MapFlags::ANY)
                .map_err(|error| SandboxLinuxError::new("read_aggregate", error.to_string()))?
            else {
                continue;
            };
            let current = KernelCounters::decode(&raw_value)?;
            snapshots.push((key, current));
        }
        let current_diagnostics = self.read_diagnostics()?;

        let mut observations = Vec::with_capacity(snapshots.len());
        for (key, current) in snapshots {
            let baseline = self.baselines.insert(key, current).unwrap_or_default();
            let delta = current.delta(baseline);
            if delta.is_empty() {
                continue;
            }
            observations.push(ProcessIoCounters {
                guest_boot_id: boot_id,
                process: key.marker(),
                sample_started_ms,
                sample_ended_ms,
                read_operations: delta.read_operations,
                read_bytes: delta.read_bytes,
                write_operations: delta.write_operations,
                write_bytes: delta.write_bytes,
                failed_read_operations: delta.failed_read_operations,
                failed_write_operations: delta.failed_write_operations,
            });
        }
        observations.sort_unstable_by_key(|item| (item.process.pid, item.process.start_time_ticks));
        let diagnostics = KernelCollectionDiagnostics {
            pending_io_drops: current_diagnostics
                .pending_io_drops
                .saturating_sub(self.diagnostic_baseline.pending_io_drops),
            aggregate_drops: current_diagnostics
                .aggregate_drops
                .saturating_sub(self.diagnostic_baseline.aggregate_drops),
            descendant_tracking_drops: current_diagnostics
                .descendant_tracking_drops
                .saturating_sub(self.diagnostic_baseline.descendant_tracking_drops),
        };
        self.diagnostic_baseline = current_diagnostics;
        Ok((observations, diagnostics))
    }

    fn seed_lineages(&self, members: &[ProcessLineageMember]) -> Result<(), SandboxLinuxError> {
        for member in members {
            self.tracked_processes
                .update(
                    &member.pid.to_ne_bytes(),
                    &RootKey::from(member.root).encode(),
                    MapFlags::ANY,
                )
                .map_err(|error| {
                    SandboxLinuxError::new(
                        "seed_lineage",
                        format!(
                            "cannot track process pid {} under root pid {}: {error}",
                            member.pid, member.root.pid
                        ),
                    )
                })?;
        }
        Ok(())
    }

    fn resize_map(
        object: &mut libbpf_rs::OpenObject,
        name: &'static str,
        max_entries: u32,
    ) -> Result<(), SandboxLinuxError> {
        object
            .maps_mut()
            .find(|map| map.name() == OsStr::new(name))
            .ok_or_else(|| {
                SandboxLinuxError::new("resize_bpf_map", format!("map {name} is missing"))
            })?
            .set_max_entries(max_entries)
            .map_err(|error| SandboxLinuxError::new("resize_bpf_map", error.to_string()))
    }

    fn map_handle(object: &Object, name: &'static str) -> Result<MapHandle, SandboxLinuxError> {
        let map = object
            .maps()
            .find(|map| map.name() == OsStr::new(name))
            .ok_or_else(|| {
                SandboxLinuxError::new("open_bpf_map", format!("map {name} is missing"))
            })?;
        MapHandle::try_from(&map)
            .map_err(|error| SandboxLinuxError::new("open_bpf_map", error.to_string()))
    }

    fn validate_map_layout(
        map: &MapHandle,
        name: &'static str,
        key_size: usize,
        value_size: usize,
    ) -> Result<(), SandboxLinuxError> {
        if map.key_size() as usize != key_size || map.value_size() as usize != value_size {
            return Err(SandboxLinuxError::new(
                "validate_bpf_abi",
                format!(
                    "map {name} layout is key={} value={}, expected key={key_size} value={value_size}",
                    map.key_size(),
                    map.value_size()
                ),
            ));
        }
        Ok(())
    }

    fn read_diagnostics(&self) -> Result<KernelCollectionDiagnostics, SandboxLinuxError> {
        let mut diagnostics = KernelCollectionDiagnostics::default();
        let mut seen = [false; 3];
        let batch = self
            .diagnostics
            .lookup_batch(3, MapFlags::ANY, MapFlags::ANY)
            .map_err(|error| {
                SandboxLinuxError::new("read_kernel_diagnostics", error.to_string())
            })?;
        for (raw_key, raw_value) in batch {
            let key = raw_key
                .get(..4)
                .and_then(|value| value.try_into().ok())
                .map(u32::from_ne_bytes)
                .ok_or_else(|| {
                    SandboxLinuxError::new(
                        "read_kernel_diagnostics",
                        format!("unexpected diagnostic key size {}", raw_key.len()),
                    )
                })?;
            let value = raw_value
                .get(..8)
                .and_then(|value| value.try_into().ok())
                .map(u64::from_ne_bytes)
                .ok_or_else(|| {
                    SandboxLinuxError::new(
                        "read_kernel_diagnostics",
                        format!("unexpected diagnostic value size {}", raw_value.len()),
                    )
                })?;
            let Some(slot) = seen.get_mut(key as usize) else {
                continue;
            };
            *slot = true;
            match key {
                0 => diagnostics.pending_io_drops = value,
                1 => diagnostics.aggregate_drops = value,
                2 => diagnostics.descendant_tracking_drops = value,
                _ => {}
            }
        }
        if seen.iter().any(|seen| !seen) {
            return Err(SandboxLinuxError::new(
                "read_kernel_diagnostics",
                "diagnostic map did not return all counters",
            ));
        }
        Ok(diagnostics)
    }
}
