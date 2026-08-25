//! Private eBPF object loading and aggregate-map decoding.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use libbpf_rs::{ErrorKind, Link, MapCore, MapFlags, MapHandle, Object, ObjectBuilder};
use libbpf_tracepoint_attach::{
    TracepointAttachOutcome, TracepointProgramAttacher, TracepointRequirement,
};
use sandbox_observation::{
    GuestBootId, OomVictimAttribution, OomVictimObservation, ProcessIoCounters, ProcessMarker,
};

use crate::SandboxLinuxError;
use crate::collector::KernelCollectionDiagnostics;
use crate::config::SandboxLinuxConfig;
use crate::procfs::{ProcessCommSnapshot, ProcessLineageMember, ProcfsReader};

const ROOT_MARKER_SIZE: usize = 32;
const KERNEL_COUNTER_SIZE: usize = 48;
const KERNEL_OOM_EVENT_SIZE: usize = 72;
const FORK_PID_OFFSET_MAP: &str = "fork_pid_offset";
const FORK_PID_OFFSET_KEY: u32 = 0;
const MAX_TRACEPOINT_FIELD_OFFSET: u32 = 4096;

pub(crate) struct EbpfCollection {
    pub(crate) process_io: Vec<ProcessIoCounters>,
    pub(crate) oom_victims: Vec<OomVictimObservation>,
    pub(crate) diagnostics: KernelCollectionDiagnostics,
    pub(crate) failures: Vec<SandboxLinuxError>,
}

#[derive(Clone, Copy)]
struct OomClockAnchor {
    monotonic_ns: u64,
    wall_ms: u64,
}

impl OomClockAnchor {
    fn capture() -> Result<Self, SandboxLinuxError> {
        let mut monotonic = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut monotonic) };
        if result != 0 || monotonic.tv_sec < 0 || monotonic.tv_nsec < 0 {
            return Err(SandboxLinuxError::new(
                "read_oom_clock",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let seconds = u64::try_from(monotonic.tv_sec)
            .map_err(|error| SandboxLinuxError::new("read_oom_clock", error.to_string()))?;
        let nanoseconds = u64::try_from(monotonic.tv_nsec)
            .map_err(|error| SandboxLinuxError::new("read_oom_clock", error.to_string()))?;
        let monotonic_ns = seconds
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_add(nanoseconds))
            .ok_or_else(|| SandboxLinuxError::new("read_oom_clock", "clock overflow"))?;
        let wall_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| SandboxLinuxError::new("read_oom_clock", error.to_string()))?
                .as_millis(),
        )
        .map_err(|error| SandboxLinuxError::new("read_oom_clock", error.to_string()))?;
        Ok(Self {
            monotonic_ns,
            wall_ms,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TracepointTarget {
    category: String,
    name: String,
}

impl TracepointTarget {
    fn display(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }
}

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
    oom_events: MapHandle,
    process_comms: MapHandle,
    publication_state: MapHandle,
    oom_event_capacity: usize,
    publication_generation: u64,
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
        Self::resize_map(&mut open_object, "oom_events", config.oom_event_capacity)?;
        Self::resize_map(
            &mut open_object,
            "process_comms",
            config.tracked_process_capacity,
        )?;
        let object = open_object
            .load()
            .map_err(|error| SandboxLinuxError::new("load_bpf_object", error.to_string()))?;
        Self::configure_fork_pid_offset(&object)?;
        let tracked_processes = Self::map_handle(&object, "tracked_processes")?;
        let pending_io = Self::map_handle(&object, "pending_io")?;
        let aggregates = Self::map_handle(&object, "io_aggregates")?;
        let diagnostics = Self::map_handle(&object, "collection_diagnostics")?;
        let oom_events = Self::map_handle(&object, "oom_events")?;
        let process_comms = Self::map_handle(&object, "process_comms")?;
        let publication_state = Self::map_handle(&object, "publication_state")?;
        Self::validate_map_layout(&tracked_processes, "tracked_processes", 4, ROOT_MARKER_SIZE)?;
        Self::validate_map_layout(&pending_io, "pending_io", 8, 40)?;
        Self::validate_map_layout(
            &aggregates,
            "io_aggregates",
            ROOT_MARKER_SIZE,
            KERNEL_COUNTER_SIZE,
        )?;
        Self::validate_map_layout(&diagnostics, "collection_diagnostics", 4, 8)?;
        Self::validate_map_layout(&oom_events, "oom_events", 0, KERNEL_OOM_EVENT_SIZE)?;
        Self::validate_map_layout(&process_comms, "process_comms", 4, 16)?;
        Self::validate_map_layout(&publication_state, "publication_state", 4, 8)?;
        let mut links = Vec::new();
        let tracepoint_attacher = TracepointProgramAttacher::new();
        for program in object.progs_mut() {
            let name = program.name().to_string_lossy().into_owned();
            let outcome = tracepoint_attacher
                .attach(&program, &name, TracepointRequirement::Required)
                .map_err(|error| {
                    SandboxLinuxError::new(
                        "attach_bpf_program",
                        format!("cannot attach {name}: {error}"),
                    )
                })?;
            let link = match outcome {
                TracepointAttachOutcome::Attached(link) => link,
                TracepointAttachOutcome::NotTracepoint => program.attach().map_err(|error| {
                    SandboxLinuxError::new(
                        "attach_bpf_program",
                        format!("cannot attach {name}: {error}"),
                    )
                })?,
                TracepointAttachOutcome::Unavailable => {
                    unreachable!("required tracepoint cannot be unavailable")
                }
            };
            links.push(link);
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
            oom_events,
            process_comms,
            publication_state,
            oom_event_capacity: config.oom_event_capacity as usize,
            publication_generation: 0,
            root_process_names: config.root_process_names.clone(),
            baselines: HashMap::new(),
            diagnostic_baseline: KernelCollectionDiagnostics::default(),
            root_refresh_interval: config.root_refresh_interval,
            next_root_refresh: Instant::now() + config.root_refresh_interval,
        };
        collector.seed_process_comms(&lineages.process_comms)?;
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
        self.seed_process_comms(&lineages.process_comms)?;
        self.seed_lineages(&lineages.members)
    }

    pub(crate) fn collect(
        &mut self,
        boot_id: GuestBootId,
        sample_started_ms: u64,
        sample_ended_ms: u64,
    ) -> EbpfCollection {
        let (oom_victims, oom_failure) = self.drain_oom_events(boot_id);
        let mut failures = oom_failure.into_iter().collect::<Vec<_>>();
        let process_io = match self.collect_process_io(boot_id, sample_started_ms, sample_ended_ms)
        {
            Ok(process_io) => process_io,
            Err(error) => {
                failures.push(error);
                Vec::new()
            }
        };
        let diagnostics = match self.collect_diagnostics() {
            Ok(diagnostics) => diagnostics,
            Err(error) => {
                failures.push(error);
                KernelCollectionDiagnostics::default()
            }
        };
        EbpfCollection {
            process_io,
            oom_victims,
            diagnostics,
            failures,
        }
    }

    fn collect_process_io(
        &mut self,
        boot_id: GuestBootId,
        sample_started_ms: u64,
        sample_ended_ms: u64,
    ) -> Result<Vec<ProcessIoCounters>, SandboxLinuxError> {
        let active_roots = self.active_roots()?;
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
        self.reclaim_inactive_roots(&active_roots);
        Ok(observations)
    }

    fn collect_diagnostics(&mut self) -> Result<KernelCollectionDiagnostics, SandboxLinuxError> {
        let current_diagnostics = self.read_diagnostics()?;
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
            oom_event_drops: current_diagnostics
                .oom_event_drops
                .saturating_sub(self.diagnostic_baseline.oom_event_drops),
            oom_comm_drops: current_diagnostics
                .oom_comm_drops
                .saturating_sub(self.diagnostic_baseline.oom_comm_drops),
        };
        self.diagnostic_baseline = current_diagnostics;
        Ok(diagnostics)
    }

    pub(crate) fn reset_publication(&mut self) -> Result<(), SandboxLinuxError> {
        self.set_publication_generation(0)?;
        let _ = self.drain_oom_events(GuestBootId::new([0; 16]));
        Ok(())
    }

    pub(crate) fn activate_publication(
        &mut self,
        generation: u64,
    ) -> Result<(), SandboxLinuxError> {
        if generation == 0 {
            return Err(SandboxLinuxError::new(
                "set_publication_generation",
                "publication generation must be non-zero",
            ));
        }
        self.set_publication_generation(generation)
    }

    fn set_publication_generation(&mut self, generation: u64) -> Result<(), SandboxLinuxError> {
        self.publication_state
            .update(
                &0_u32.to_ne_bytes(),
                &generation.to_ne_bytes(),
                MapFlags::ANY,
            )
            .map_err(|error| {
                SandboxLinuxError::new("set_publication_generation", error.to_string())
            })?;
        self.publication_generation = generation;
        Ok(())
    }

    fn drain_oom_events(
        &self,
        boot_id: GuestBootId,
    ) -> (Vec<OomVictimObservation>, Option<SandboxLinuxError>) {
        let anchor = match OomClockAnchor::capture() {
            Ok(anchor) => anchor,
            Err(error) => return (Vec::new(), Some(error)),
        };
        let mut observations = Vec::new();
        let mut failure = None;
        for _ in 0..self.oom_event_capacity {
            let raw = match self.oom_events.lookup_and_delete(&[]) {
                Ok(Some(raw)) => raw,
                Ok(None) => break,
                Err(error) => {
                    failure = Some(SandboxLinuxError::new("read_oom_event", error.to_string()));
                    break;
                }
            };
            match Self::decode_oom_event(&raw, boot_id, self.publication_generation, anchor) {
                Ok(Some(observation)) => observations.push(observation),
                Ok(None) => {}
                Err(error) if failure.is_none() => failure = Some(error),
                Err(_) => {}
            }
        }
        (observations, failure)
    }

    fn decode_oom_event(
        raw: &[u8],
        boot_id: GuestBootId,
        publication_generation: u64,
        anchor: OomClockAnchor,
    ) -> Result<Option<OomVictimObservation>, SandboxLinuxError> {
        if raw.len() != KERNEL_OOM_EVENT_SIZE || raw[37..40] != [0; 3] {
            return Err(SandboxLinuxError::new(
                "decode_oom_event",
                "unexpected kernel OOM event layout",
            ));
        }
        let event_boot_ns = u64::from_ne_bytes(raw[0..8].try_into().expect("checked OOM event"));
        let event_generation =
            u64::from_ne_bytes(raw[8..16].try_into().expect("checked OOM event"));
        if event_generation == 0 || event_generation != publication_generation {
            return Ok(None);
        }
        let victim_pid = u32::from_ne_bytes(raw[16..20].try_into().expect("checked OOM event"));
        let victim_comm = raw[20..36].try_into().expect("checked OOM event");
        let raw_root = &raw[40..72];
        let (attribution, monitored_root) = match raw[36] {
            0 if raw_root.iter().all(|byte| *byte == 0) => (OomVictimAttribution::Unknown, None),
            1 => (
                OomVictimAttribution::Monitored,
                Some(RootKey::decode(raw_root)?.marker()),
            ),
            2 if raw_root.iter().all(|byte| *byte == 0) => {
                (OomVictimAttribution::Unmonitored, None)
            }
            _ => {
                return Err(SandboxLinuxError::new(
                    "decode_oom_event",
                    "invalid kernel OOM attribution",
                ));
            }
        };
        let age_ms = anchor.monotonic_ns.saturating_sub(event_boot_ns) / 1_000_000;
        let observation = OomVictimObservation {
            guest_boot_id: boot_id,
            detected_at_ms: anchor.wall_ms.saturating_sub(age_ms),
            victim_pid,
            victim_comm,
            attribution,
            monitored_root,
        }
        .validate()
        .map_err(|message| SandboxLinuxError::new("decode_oom_event", message))?;
        Ok(Some(observation))
    }

    fn seed_lineages(&self, members: &[ProcessLineageMember]) -> Result<(), SandboxLinuxError> {
        for member in members {
            let pid = member.pid.to_ne_bytes();
            if self
                .tracked_processes
                .lookup(&pid, MapFlags::ANY)
                .map_err(|error| {
                    SandboxLinuxError::new(
                        "seed_lineage",
                        format!("cannot inspect tracked pid {}: {error}", member.pid),
                    )
                })?
                .is_some()
            {
                continue;
            }
            match self.tracked_processes.update(
                &pid,
                &RootKey::from(member.root).encode(),
                MapFlags::NO_EXIST,
            ) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(SandboxLinuxError::new(
                        "seed_lineage",
                        format!(
                            "cannot track process pid {} under root pid {}: {error}",
                            member.pid, member.root.pid
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn seed_process_comms(
        &self,
        processes: &[ProcessCommSnapshot],
    ) -> Result<(), SandboxLinuxError> {
        for process in processes {
            match self.process_comms.update(
                &process.pid.to_ne_bytes(),
                &process.executable_name,
                MapFlags::NO_EXIST,
            ) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(SandboxLinuxError::new(
                        "seed_process_comm",
                        format!("cannot cache process pid {} comm: {error}", process.pid),
                    ));
                }
            }
        }
        Ok(())
    }

    fn active_roots(&self) -> Result<HashSet<RootKey>, SandboxLinuxError> {
        let process_keys = self.tracked_processes.keys().collect::<Vec<_>>();
        let mut roots = HashSet::with_capacity(process_keys.len());
        for process_key in process_keys {
            let Some(raw_root) = self
                .tracked_processes
                .lookup(&process_key, MapFlags::ANY)
                .map_err(|error| {
                    SandboxLinuxError::new("read_tracked_process", error.to_string())
                })?
            else {
                continue;
            };
            roots.insert(RootKey::decode(&raw_root)?);
        }
        Ok(roots)
    }

    fn reclaim_inactive_roots(&mut self, active_roots: &HashSet<RootKey>) {
        let inactive_roots = self
            .baselines
            .keys()
            .copied()
            .filter(|root| !active_roots.contains(root))
            .collect::<Vec<_>>();
        for root in inactive_roots {
            let raw_root = root.encode();
            match self.aggregates.delete(&raw_root) {
                Ok(()) => {
                    self.baselines.remove(&root);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.baselines.remove(&root);
                }
                Err(_) => {}
            }
        }
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

    fn configure_fork_pid_offset(object: &Object) -> Result<(), SandboxLinuxError> {
        let target = TracepointTarget {
            category: "sched".to_owned(),
            name: "sched_process_fork".to_owned(),
        };
        let offset = read_tracepoint_field_offset(&target, "child_pid")?;
        let map = Self::map_handle(object, FORK_PID_OFFSET_MAP)?;
        Self::validate_map_layout(&map, FORK_PID_OFFSET_MAP, 4, 4)?;
        map.update(
            &FORK_PID_OFFSET_KEY.to_ne_bytes(),
            &offset.to_ne_bytes(),
            MapFlags::ANY,
        )
        .map_err(|error| SandboxLinuxError::new("configure_fork_pid_offset", error.to_string()))
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
        let mut seen = [false; 5];
        let batch = self
            .diagnostics
            .lookup_batch(5, MapFlags::ANY, MapFlags::ANY)
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
                3 => diagnostics.oom_event_drops = value,
                4 => diagnostics.oom_comm_drops = value,
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

fn read_tracepoint_field_offset(
    target: &TracepointTarget,
    field_name: &str,
) -> Result<u32, SandboxLinuxError> {
    let mut errors = Vec::new();
    for root in tracefs_roots()? {
        let path = root
            .join("events")
            .join(&target.category)
            .join(&target.name)
            .join("format");
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let Some(offset) = parse_tracepoint_field_offset(&content, field_name) {
                    return Ok(offset);
                }
                errors.push(format!(
                    "{}: field {field_name} is missing or is not a nonzero u32 field",
                    path.display()
                ));
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(SandboxLinuxError::new(
        "configure_fork_pid_offset",
        format!(
            "tracepoint {} field {field_name} is unavailable: {}",
            target.display(),
            errors.join("; ")
        ),
    ))
}

fn parse_tracepoint_field_offset(content: &str, field_name: &str) -> Option<u32> {
    for line in content.lines().map(str::trim) {
        let mut parts = line.split(';').map(str::trim);
        let Some(field) = parts.next().and_then(|part| part.strip_prefix("field:")) else {
            continue;
        };
        if field.split_whitespace().last() != Some(field_name) {
            continue;
        }
        let mut offset = None;
        let mut size = None;
        for part in parts {
            if let Some(value) = part.strip_prefix("offset:") {
                offset = value.trim().parse::<u32>().ok();
            } else if let Some(value) = part.strip_prefix("size:") {
                size = value.trim().parse::<u32>().ok();
            }
        }
        let offset = offset?;
        return (offset > 0
            && offset <= MAX_TRACEPOINT_FIELD_OFFSET
            && size == Some(std::mem::size_of::<u32>() as u32))
        .then_some(offset);
    }
    None
}

fn tracefs_roots() -> Result<Vec<PathBuf>, SandboxLinuxError> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
        SandboxLinuxError::new(
            "configure_fork_pid_offset",
            format!("cannot read /proc/self/mountinfo: {error}"),
        )
    })?;
    let roots = mountinfo
        .lines()
        .filter_map(parse_tracefs_mount)
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(SandboxLinuxError::new(
            "configure_fork_pid_offset",
            "tracefs mount is missing",
        ));
    }
    Ok(roots)
}

fn parse_tracefs_mount(line: &str) -> Option<PathBuf> {
    let (mount_fields, fs_fields) = line.split_once(" - ")?;
    if fs_fields.split_whitespace().next()? != "tracefs" {
        return None;
    }
    let mut fields = mount_fields.split_whitespace();
    let _mount_id = fields.next()?;
    let _parent_id = fields.next()?;
    let _device = fields.next()?;
    let _root = fields.next()?;
    fields.next().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::parse_tracepoint_field_offset;

    const LEGACY_FORK_FORMAT: &str = r#"
name: sched_process_fork
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:char parent_comm[16];	offset:8;	size:16;	signed:1;
	field:pid_t parent_pid;	offset:24;	size:4;	signed:1;
	field:char child_comm[16];	offset:28;	size:16;	signed:1;
	field:pid_t child_pid;	offset:44;	size:4;	signed:1;
"#;

    const DYNAMIC_FORK_FORMAT: &str = r#"
name: sched_process_fork
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:__data_loc char[] parent_comm;	offset:8;	size:4;	signed:0;
	field:pid_t parent_pid;	offset:12;	size:4;	signed:1;
	field:__data_loc char[] child_comm;	offset:16;	size:4;	signed:0;
	field:pid_t child_pid;	offset:20;	size:4;	signed:1;
"#;

    #[test]
    fn parses_legacy_fork_child_pid_offset() {
        assert_eq!(
            parse_tracepoint_field_offset(LEGACY_FORK_FORMAT, "child_pid"),
            Some(44)
        );
    }

    #[test]
    fn parses_dynamic_fork_child_pid_offset() {
        assert_eq!(
            parse_tracepoint_field_offset(DYNAMIC_FORK_FORMAT, "child_pid"),
            Some(20)
        );
    }

    #[test]
    fn rejects_missing_zero_or_non_u32_tracepoint_fields() {
        assert_eq!(
            parse_tracepoint_field_offset(DYNAMIC_FORK_FORMAT, "missing_pid"),
            None
        );
        assert_eq!(
            parse_tracepoint_field_offset(
                "field:pid_t child_pid; offset:0; size:4; signed:1;",
                "child_pid"
            ),
            None
        );
        assert_eq!(
            parse_tracepoint_field_offset(
                "field:long child_pid; offset:20; size:8; signed:1;",
                "child_pid"
            ),
            None
        );
    }
}
