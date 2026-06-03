//! Probe session orchestration.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::thread;
use std::time::Instant;

use tls_probe_point_finder::ProbePointPlan;
use tls_probe_point_finder::fast::FastProbeRequest;

use crate::capture::config::{
    ABI_MAX_CAPTURE_BYTES, ABI_MAX_RUSTLS_CHUNKS, BPF_EVENT_HEADER_BYTES, CaptureConfig,
};
use crate::capture::ebpf::BpfPayloadRuntime;
use crate::capture::event::CaptureEvent;
use crate::capture::ring_stats::RingLostStats;
use crate::capture::target::PausedTarget;
use crate::{ToolError, ToolResult};

pub(crate) struct ProbeSession;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProbeRunResult {
    pub(crate) status: ExitStatus,
    pub(crate) ring_lost_stats: RingLostStats,
}

impl ProbeSession {
    pub(crate) fn run(
        config: CaptureConfig,
        mut on_event: impl FnMut(CaptureEvent) -> ToolResult<()>,
    ) -> ToolResult<ProbeRunResult> {
        validate_config(&config)?;
        let plan = resolve_plan(&config)?;
        validate_plan(&plan)?;

        let launch_command = launch_command_for_plan(&config.command, &plan)?;
        let mut target = PausedTarget::spawn(&launch_command)?;
        let mut runtime = match BpfPayloadRuntime::load(&config, &plan, target.pid()) {
            Ok(runtime) => runtime,
            Err(error) => {
                target.terminate();
                return Err(error);
            }
        };
        target.resume()?;

        let mut exit_status = None;
        let mut drain_until = None;
        loop {
            for event in runtime.poll_events()? {
                on_event(event)?;
            }
            if exit_status.is_none() {
                if let Some(status) = target.try_wait()? {
                    exit_status = Some(status);
                    drain_until = Some(Instant::now() + config.drain_after_exit);
                }
            }
            if drain_until.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            thread::sleep(config.poll_interval);
        }
        for event in runtime.poll_events()? {
            on_event(event)?;
        }
        for event in runtime.finish_events()? {
            on_event(event)?;
        }
        let status = exit_status.ok_or_else(|| {
            target.terminate();
            ToolError::new("target did not exit before probe loop ended")
        })?;
        let ring_lost_stats = runtime.ring_lost_stats()?;
        Ok(ProbeRunResult {
            status,
            ring_lost_stats,
        })
    }
}

fn validate_config(config: &CaptureConfig) -> ToolResult<()> {
    if config.command.is_empty() {
        return Err(ToolError::new("probe command is empty"));
    }
    if config.max_capture_bytes == 0 || config.max_capture_bytes > ABI_MAX_CAPTURE_BYTES {
        return Err(ToolError::new(format!(
            "max_capture_bytes must be in 1..={ABI_MAX_CAPTURE_BYTES}, got {}",
            config.max_capture_bytes
        )));
    }
    if config.rustls_chunks == 0 || config.rustls_chunks > ABI_MAX_RUSTLS_CHUNKS {
        return Err(ToolError::new(format!(
            "rustls_chunks must be in 1..={ABI_MAX_RUSTLS_CHUNKS}, got {}",
            config.rustls_chunks
        )));
    }
    let needed_ring_bytes = u32::try_from(config.max_capture_bytes)
        .ok()
        .and_then(|bytes| bytes.checked_add(BPF_EVENT_HEADER_BYTES))
        .ok_or_else(|| ToolError::new("ring buffer size validation overflow"))?;
    if config.ring_buffer_bytes < needed_ring_bytes {
        return Err(ToolError::new(format!(
            "ring_buffer_bytes must be at least max_capture_bytes + event header ({needed_ring_bytes}), got {}",
            config.ring_buffer_bytes
        )));
    }
    if config.pending_ops == 0 {
        return Err(ToolError::new("pending_ops must be positive"));
    }
    if config.assemble_buffer_bytes == 0 {
        return Err(ToolError::new("assemble_buffer_bytes must be positive"));
    }
    if config.decode_input_bytes == 0 {
        return Err(ToolError::new("decode_input_bytes must be positive"));
    }
    if config.decode_output_bytes == 0 {
        return Err(ToolError::new("decode_output_bytes must be positive"));
    }
    if config.decode_reader_buffer_bytes == 0 {
        return Err(ToolError::new(
            "decode_reader_buffer_bytes must be positive",
        ));
    }
    Ok(())
}

fn resolve_plan(config: &CaptureConfig) -> ToolResult<ProbePointPlan> {
    let Some(program) = config.command.first() else {
        return Err(ToolError::new("probe command is empty"));
    };
    tls_probe_point_finder::fast::resolve(FastProbeRequest {
        binary: program.into(),
        arch: config.arch,
        provider: config.provider,
        source: config.source,
        match_limit: config.match_limit,
        libraries: config.libraries.clone(),
        library_search_dirs: config.library_search_dirs.clone(),
    })
    .map_err(Into::into)
}

fn validate_plan(plan: &ProbePointPlan) -> ToolResult<()> {
    let host_arch = std::env::consts::ARCH;
    if plan.binary.architecture != host_arch {
        return Err(ToolError::new(format!(
            "BPF object is built for {host_arch}, but probe binary is {}",
            plan.binary.architecture
        )));
    }
    if !plan.has_payload_closure() {
        return Err(ToolError::new("probe plan does not have payload closure"));
    }
    Ok(())
}

fn launch_command_for_plan(
    command: &[OsString],
    plan: &ProbePointPlan,
) -> ToolResult<Vec<OsString>> {
    let Some(program) = command.first() else {
        return Err(ToolError::new("probe command is empty"));
    };
    let entry = resolve_command_path(program)?;
    let plan_binary = std::fs::canonicalize(&plan.binary.path).map_err(|error| {
        ToolError::new(format!(
            "cannot resolve probe binary {}: {error}",
            plan.binary.path.display()
        ))
    })?;
    if hidden_sibling_binary(&entry).is_some_and(|sibling| sibling == plan_binary) {
        let mut rewritten = command.to_vec();
        rewritten[0] = plan_binary.into_os_string();
        return Ok(rewritten);
    }
    Ok(command.to_vec())
}

fn resolve_command_path(program: &OsStr) -> ToolResult<PathBuf> {
    let raw = Path::new(program);
    if raw.components().count() > 1 {
        return std::fs::canonicalize(raw)
            .map_err(|error| ToolError::new(format!("cannot resolve {}: {error}", raw.display())));
    }
    let path_var = std::env::var_os("PATH").ok_or_else(|| ToolError::new("PATH is not set"))?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(raw);
        if candidate.is_file() {
            return std::fs::canonicalize(&candidate).map_err(|error| {
                ToolError::new(format!("cannot resolve {}: {error}", candidate.display()))
            });
        }
    }
    Err(ToolError::new(format!(
        "command not found on PATH: {}",
        program.to_string_lossy()
    )))
}

fn hidden_sibling_binary(entry: &Path) -> Option<PathBuf> {
    let parent = entry.parent()?;
    let file_name = entry.file_name()?;
    let mut hidden_name = OsString::from(".");
    hidden_name.push(file_name);
    std::fs::canonicalize(parent.join(hidden_name)).ok()
}
