//! Consistent snapshots of the kernel's cumulative file I/O counters.

mod misses;
use misses::FileIoProgramMisses;

use std::time::{Duration, Instant};

use config_core::daemon::{EbpfCollectorConfig, FileIoSummaryConfig};
use libbpf_rs::{MapCore, MapFlags, MapHandle, Object};
use model_core::ids::TraceId;

use crate::loader::runtime_loss::LossDiagnostics;
use crate::loader::{AttachPlan, EbpfRuntime, LoaderError, object::map_handle};

// Matches ACTRAIL_FILE_IO_OBJECT_DELETE_FAIL in bpf/file/io/state.h.
const FILE_IO_OBJECT_DELETE_FAIL: usize = 5;
// In actrail_file_io_diagnostic order in bpf/file/io/state.h.
const FILE_IO_DIAGNOSTICS: [&str; 6] = [
    "file_io_object_insert_fail",
    "file_io_total_insert_fail",
    "file_io_path_capture_fail",
    "file_io_counter_overflow",
    "file_io_identity_fail",
    "file_io_object_delete_fail",
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileIoKey {
    pub trace_id: TraceId,
    pub generation: u64,
    pub token: u64,
    pub pid: u32,
    pub direction: u32,
    pub error_result: i32,
}

impl FileIoKey {
    fn decode(raw: &[u8]) -> Option<Self> {
        Some(Self {
            trace_id: TraceId::new(u64::from_ne_bytes(raw.get(0..8)?.try_into().ok()?)),
            generation: u64::from_ne_bytes(raw.get(8..16)?.try_into().ok()?),
            token: u64::from_ne_bytes(raw.get(16..24)?.try_into().ok()?),
            pid: u32::from_ne_bytes(raw.get(24..28)?.try_into().ok()?),
            direction: u32::from_ne_bytes(raw.get(28..32)?.try_into().ok()?),
            error_result: i32::from_ne_bytes(raw.get(32..36)?.try_into().ok()?),
        })
    }

    fn encode(self) -> [u8; 40] {
        let mut raw = [0; 40];
        raw[0..8].copy_from_slice(&self.trace_id.get().to_ne_bytes());
        raw[8..16].copy_from_slice(&self.generation.to_ne_bytes());
        raw[16..24].copy_from_slice(&self.token.to_ne_bytes());
        raw[24..28].copy_from_slice(&self.pid.to_ne_bytes());
        raw[28..32].copy_from_slice(&self.direction.to_ne_bytes());
        raw[32..36].copy_from_slice(&self.error_result.to_ne_bytes());
        raw
    }
}

pub(crate) struct FileIoSnapshot {
    pub key: FileIoKey,
    pub flags: u32,
    pub sequence: u64,
    pub count: u64,
    pub bytes: u64,
    pub first_ns: u64,
    pub last_ns: u64,
    pub path_status: u32,
    pub target_kind: u32,
    pub path: Option<String>,
    pub coverage_complete: bool,
}

impl FileIoSnapshot {
    fn decode(key: FileIoKey, raw: &[u8]) -> Option<Self> {
        let word = |offset| {
            Some(u32::from_ne_bytes(
                raw.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        let long = |offset| {
            Some(u64::from_ne_bytes(
                raw.get(offset..offset + 8)?.try_into().ok()?,
            ))
        };
        let length = usize::try_from(word(52)?).ok()?.min(256);
        let path = raw.get(64..64 + length)?;
        let path = path.split(|byte| *byte == 0).next()?;
        Some(Self {
            key,
            flags: word(4)?,
            sequence: long(8)?,
            count: long(16)?,
            bytes: long(24)?,
            first_ns: long(32)?,
            last_ns: long(40)?,
            path_status: word(56)?,
            target_kind: word(60)?,
            coverage_complete: true,
            path: (!path.is_empty()).then(|| String::from_utf8_lossy(path).into_owned()),
        })
    }
}

pub(super) struct FileIoSummaryMap {
    totals: MapHandle,
    diagnostics: MapHandle,
    diagnostic_baseline: [u64; 6],
    program_misses: FileIoProgramMisses,
    config_map: MapHandle,
    capture_disabled: bool,
    coverage_complete: bool,
    interval: Duration,
    last_snapshot: Instant,
}

impl FileIoSummaryMap {
    pub(super) fn from_object(
        object: &Object,
        config: &EbpfCollectorConfig,
        summary: &FileIoSummaryConfig,
        plan: &AttachPlan,
    ) -> Result<Option<Self>, LoaderError> {
        if !plan.file_io_summary_enabled() {
            return Ok(None);
        }
        let totals = map_handle(object, "file_io_totals", "file_io_summary")?;
        if totals.key_size() != 40 || totals.value_size() != 320 {
            return Err(LoaderError::new(
                "file_io_summary",
                "unexpected summary map layout",
            ));
        }
        let flags = |d: config_core::daemon::FileIoCollectionConfig| {
            u32::from(d.observed)
                | (u32::from(d.counts) << 1)
                | (u32::from(d.bytes) << 2)
                | (u32::from(d.errors) << 3)
        };
        let demand = plan.file_collection();
        let mut raw = [0_u8; 16];
        raw[0..4].copy_from_slice(&flags(demand.read).to_ne_bytes());
        raw[4..8].copy_from_slice(&flags(demand.write).to_ne_bytes());
        raw[8..12].copy_from_slice(&config.file_path_max_bytes.to_ne_bytes());
        raw[12..16].copy_from_slice(&u32::from(plan.file_tty_observation()).to_ne_bytes());
        map_handle(object, "file_io_config", "file_io_summary")?
            .update(&0_u32.to_ne_bytes(), &raw, MapFlags::ANY)
            .map_err(|e| LoaderError::new("file_io_summary", e.to_string()))?;
        Ok(Some(Self {
            totals,
            diagnostics: map_handle(object, "file_io_diagnostics", "file_io_summary")?,
            diagnostic_baseline: [0; 6],
            program_misses: FileIoProgramMisses::from_object(object)?,
            config_map: map_handle(object, "file_io_config", "file_io_summary")?,
            capture_disabled: false,
            coverage_complete: true,
            interval: Duration::from_millis(u64::from(summary.flush_interval_ms)),
            last_snapshot: Instant::now(),
        }))
    }

    fn snapshots(&mut self, force: bool, losses: &mut LossDiagnostics) -> Vec<FileIoSnapshot> {
        if !force && self.last_snapshot.elapsed() < self.interval {
            return Vec::new();
        }
        self.last_snapshot = Instant::now();
        let mut snapshots = Vec::new();
        for key in self.totals.keys() {
            let Some(parsed) = FileIoKey::decode(&key) else {
                continue;
            };
            match self.totals.lookup(&key, MapFlags::LOCK) {
                Ok(Some(raw)) => match FileIoSnapshot::decode(parsed, &raw) {
                    Some(snapshot) => snapshots.push(snapshot),
                    None => {
                        self.coverage_complete = false;
                        losses.record_count("file_io_invalid_summary", 1);
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    self.coverage_complete = false;
                    losses.record_error("file_io_snapshot_fail", || e.to_string());
                }
            }
        }
        for (index, previous) in self.diagnostic_baseline.iter_mut().enumerate() {
            match self
                .diagnostics
                .lookup(&(index as u32).to_ne_bytes(), MapFlags::ANY)
            {
                Ok(Some(raw)) if raw.len() == 8 => {
                    let current = u64::from_ne_bytes(raw.try_into().unwrap());
                    if current != *previous {
                        self.coverage_complete = false;
                        losses.record_count(
                            FILE_IO_DIAGNOSTICS[index],
                            current.saturating_sub(*previous),
                        );
                        *previous = current;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    self.coverage_complete = false;
                    losses.record_error("file_io_diagnostics_read_fail", || e.to_string());
                }
            }
        }
        match self.program_misses.poll() {
            Ok(misses) => {
                for missed in misses {
                    self.coverage_complete = false;
                    losses.record_detail(missed.program, missed.count, || {
                        format!(
                            "missed executions; identity_untrusted={}",
                            missed.identity_untrusted
                        )
                    });
                }
            }
            Err(error) => {
                self.coverage_complete = false;
                losses.record_error("file_io_program_diagnostics_fail", || format!("{error:?}"));
            }
        }
        if self.program_misses.identity_untrusted()
            || self.diagnostic_baseline[FILE_IO_OBJECT_DELETE_FAIL] != 0
        {
            snapshots.clear();
            if !self.capture_disabled {
                match self
                    .config_map
                    .update(&0_u32.to_ne_bytes(), &[0; 16], MapFlags::ANY)
                {
                    Ok(()) => {
                        self.capture_disabled = true;
                        losses.record_error("file_io_capture_disabled", || {
                            "object lifetime identity is no longer trustworthy".into()
                        });
                    }
                    Err(error) => {
                        losses.record_error("file_io_capture_disable_fail", || error.to_string())
                    }
                }
            }
        }
        for snapshot in &mut snapshots {
            snapshot.coverage_complete = self.coverage_complete;
        }
        snapshots
    }
}

impl EbpfRuntime {
    pub(crate) fn file_io_poll_timeout(&self) -> Option<Duration> {
        self.file_io_summaries.as_ref().map(|summaries| {
            summaries
                .interval
                .saturating_sub(summaries.last_snapshot.elapsed())
        })
    }

    pub(crate) fn file_io_snapshots(&mut self, force: bool) -> Vec<FileIoSnapshot> {
        let Some(summaries) = self.file_io_summaries.as_mut() else {
            return Vec::new();
        };
        summaries.snapshots(force, &mut self.loss_diagnostics)
    }

    pub(crate) fn remove_file_io_summary(&mut self, key: FileIoKey) -> bool {
        let Some(summaries) = self.file_io_summaries.as_ref() else {
            return true;
        };
        match summaries.totals.delete(&key.encode()) {
            Ok(()) => true,
            Err(error) => {
                self.loss_diagnostics
                    .record_error("file_io_summary_cleanup_fail", || error.to_string());
                false
            }
        }
    }
}
