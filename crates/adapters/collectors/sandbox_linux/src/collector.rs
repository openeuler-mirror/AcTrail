use std::time::{SystemTime, UNIX_EPOCH};

use sandbox_observation::{GuestResourceSnapshot, OomVictimObservation, ProcessIoCounters};

use crate::ebpf::EbpfIoCollector;
use crate::procfs::ProcfsReader;
use crate::resource::LinuxResourceReader;
use crate::{SandboxLinuxConfig, SandboxLinuxError};

/// One fail-local read cycle from the Guest collector.
pub struct CollectionCycle {
    pub process_io: Vec<ProcessIoCounters>,
    pub oom_victims: Vec<OomVictimObservation>,
    pub resources: Option<GuestResourceSnapshot>,
    pub kernel_diagnostics: KernelCollectionDiagnostics,
    pub failures: Vec<SandboxLinuxError>,
}

/// One process-I/O polling result, independent from resource sampling cadence.
pub struct ProcessIoCycle {
    pub process_io: Vec<ProcessIoCounters>,
    pub oom_victims: Vec<OomVictimObservation>,
    pub kernel_diagnostics: KernelCollectionDiagnostics,
    pub failures: Vec<SandboxLinuxError>,
}

/// Kernel-side capacity losses observed since the previous collection cycle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KernelCollectionDiagnostics {
    pub pending_io_drops: u64,
    pub aggregate_drops: u64,
    pub descendant_tracking_drops: u64,
    pub oom_event_drops: u64,
    pub oom_comm_drops: u64,
}

/// Owns only Guest process discovery and eBPF I/O aggregation.
pub struct SandboxProcessIoCollector {
    ebpf: EbpfIoCollector,
    procfs: ProcfsReader,
    boot_id: sandbox_observation::GuestBootId,
    previous_sample_ms: u64,
}

impl SandboxProcessIoCollector {
    pub fn start(config: SandboxLinuxConfig) -> Result<Self, SandboxLinuxError> {
        let procfs = ProcfsReader::open(config.procfs_root.clone())?;
        let boot_id = procfs.boot_id()?;
        let ebpf = EbpfIoCollector::start(&config, &procfs)?;
        Ok(Self {
            ebpf,
            procfs,
            boot_id,
            previous_sample_ms: now_ms()?,
        })
    }

    pub fn poll(&mut self) -> ProcessIoCycle {
        let mut failures = Vec::new();
        if let Err(error) = self.ebpf.refresh_roots(&self.procfs) {
            failures.push(error);
        }
        let collected = match now_ms() {
            Ok(sample_ended_ms) => {
                let collected =
                    self.ebpf
                        .collect(self.boot_id, self.previous_sample_ms, sample_ended_ms);
                self.previous_sample_ms = sample_ended_ms;
                collected
            }
            Err(error) => {
                failures.push(error);
                crate::ebpf::EbpfCollection {
                    process_io: Vec::new(),
                    oom_victims: Vec::new(),
                    diagnostics: KernelCollectionDiagnostics::default(),
                    failures: Vec::new(),
                }
            }
        };
        failures.extend(collected.failures);
        ProcessIoCycle {
            process_io: collected.process_io,
            oom_victims: collected.oom_victims,
            kernel_diagnostics: collected.diagnostics,
            failures,
        }
    }

    pub fn reset_publication(&mut self) -> Result<(), SandboxLinuxError> {
        self.ebpf.reset_publication()
    }

    pub fn activate_publication(&mut self, generation: u64) -> Result<(), SandboxLinuxError> {
        self.ebpf.activate_publication(generation)
    }
}

/// Optional owner for callers that intentionally use one combined cadence.
pub struct SandboxLinuxCollector {
    process_io: SandboxProcessIoCollector,
    resources: LinuxResourceReader,
}

impl SandboxLinuxCollector {
    pub fn start(config: SandboxLinuxConfig) -> Result<Self, SandboxLinuxError> {
        let resources = LinuxResourceReader::open(config.procfs_root.clone())?;
        let process_io = SandboxProcessIoCollector::start(config)?;
        Ok(Self {
            process_io,
            resources,
        })
    }

    pub fn collect(&mut self) -> CollectionCycle {
        let process_cycle = self.process_io.poll();
        let mut failures = process_cycle.failures;
        let resources = match self.resources.sample() {
            Ok(resources) => Some(resources),
            Err(error) => {
                failures.push(error);
                None
            }
        };
        CollectionCycle {
            process_io: process_cycle.process_io,
            oom_victims: process_cycle.oom_victims,
            resources,
            kernel_diagnostics: process_cycle.kernel_diagnostics,
            failures,
        }
    }
}

fn now_ms() -> Result<u64, SandboxLinuxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SandboxLinuxError::new("sample_clock", error.to_string()))?
        .as_millis()
        .try_into()
        .map_err(|error| {
            SandboxLinuxError::new("sample_clock", format!("timestamp overflow: {error}"))
        })
}
