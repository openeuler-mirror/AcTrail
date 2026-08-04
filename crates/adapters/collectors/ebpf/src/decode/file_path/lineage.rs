//! Event-driven admission index for anonymous IPC descriptors used by stdio MCP.
//!
//! It identifies client/server fd bundles before payload projection. Only a
//! ready bundle may enter MCP JSON-RPC framing; ordinary stdio is not parsed.

use std::collections::{BTreeMap, BTreeSet};

use collector_event::RawCollectorEvent;
use config_core::daemon::IpcLineageConfig;
use model_core::ids::TraceId;
use model_core::process::ProcessObservation;

use super::state::ProcessFileKey;

mod bundle;
mod bundle_state;
mod tracker;

use bundle::{StdioBundle, StdioBundleLifecycle, StdioLineageDiagnostic};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::decode) enum FdIpcKind {
    Pipe,
    UnixSocket,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LineageProcessId {
    host_pid: u32,
    generation: u64,
}

impl LineageProcessId {
    fn from_observation(process: &ProcessObservation) -> Option<Self> {
        let (host_pid, generation) = if let Some(host) = &process.host {
            (
                host.pid,
                host.start_boottime_ns.unwrap_or(host.start_time_ticks),
            )
        } else {
            let namespace = process.namespace.as_ref()?;
            (namespace.pid, namespace.start_time_ticks)
        };
        (host_pid != 0 && generation != 0).then_some(Self {
            host_pid,
            generation,
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IpcChannelId {
    creator_process: LineageProcessId,
    created_ktime_ns: u64,
    fd_a: u32,
    fd_b: u32,
}

impl IpcChannelId {
    fn stable_id(&self, trace_id: TraceId) -> String {
        format!(
            "ipc:{}:{}:{}:{}:{}:{}",
            trace_id.get(),
            self.creator_process.host_pid,
            self.creator_process.generation,
            self.created_ktime_ns,
            self.fd_a,
            self.fd_b,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum IpcEndpointSide {
    A,
    B,
    Read,
    Write,
}

impl IpcEndpointSide {
    fn opposite(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
            Self::Read => Self::Write,
            Self::Write => Self::Read,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipeDirection {
    Read,
    Write,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IpcBindingOrigin {
    Created,
    Inherited,
    Duplicated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IpcEndpointBinding {
    channel_id: IpcChannelId,
    kind: FdIpcKind,
    side: IpcEndpointSide,
    pipe_direction: Option<PipeDirection>,
    origin: IpcBindingOrigin,
    close_on_exec: bool,
}

impl IpcEndpointBinding {
    fn created(
        channel_id: IpcChannelId,
        kind: FdIpcKind,
        side: IpcEndpointSide,
        close_on_exec: bool,
    ) -> Self {
        Self {
            channel_id,
            kind,
            side,
            pipe_direction: match side {
                IpcEndpointSide::Read => Some(PipeDirection::Read),
                IpcEndpointSide::Write => Some(PipeDirection::Write),
                IpcEndpointSide::A | IpcEndpointSide::B => None,
            },
            origin: IpcBindingOrigin::Created,
            close_on_exec,
        }
    }

    fn inherited(&self) -> Self {
        let mut inherited = self.clone();
        inherited.origin = IpcBindingOrigin::Inherited;
        inherited
    }

    fn duplicated(&self, close_on_exec: bool) -> Self {
        let mut duplicated = self.clone();
        duplicated.origin = IpcBindingOrigin::Duplicated;
        duplicated.close_on_exec = close_on_exec;
        duplicated
    }

    fn supports_server_stdin(&self) -> bool {
        self.kind == FdIpcKind::UnixSocket || self.pipe_direction == Some(PipeDirection::Read)
    }

    fn supports_server_output(&self) -> bool {
        self.kind == FdIpcKind::UnixSocket || self.pipe_direction == Some(PipeDirection::Write)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IpcEndpointOwner {
    process: LineageProcessId,
    fd: u32,
    side: IpcEndpointSide,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IpcProcessState {
    observation: ProcessObservation,
    fds: BTreeMap<u32, IpcEndpointBinding>,
    exec_ktime_ns: Option<u64>,
}

impl IpcProcessState {
    fn new(observation: ProcessObservation) -> Self {
        Self {
            observation,
            fds: BTreeMap::new(),
            exec_ktime_ns: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TraceLineageState {
    processes: BTreeMap<LineageProcessId, IpcProcessState>,
    parents: BTreeMap<LineageProcessId, LineageProcessId>,
    owners_by_channel: BTreeMap<IpcChannelId, BTreeSet<IpcEndpointOwner>>,
    active_bundles: BTreeMap<LineageProcessId, StdioBundle>,
    bundles_by_channel: BTreeMap<IpcChannelId, BTreeSet<LineageProcessId>>,
    last_gaps: BTreeMap<LineageProcessId, &'static str>,
    diagnostics: BTreeMap<&'static str, u64>,
    pending_lifecycle: Vec<StdioBundleLifecycle>,
    pending_degradations: Vec<StdioLineageDiagnostic>,
    candidate_fd_count: usize,
    disabled_reason: Option<&'static str>,
}

impl TraceLineageState {
    fn observe_process(
        &mut self,
        process: &ProcessObservation,
        config: &IpcLineageConfig,
    ) -> Result<LineageProcessId, &'static str> {
        if let Some(reason) = self.disabled_reason {
            return Err(reason);
        }
        let id = LineageProcessId::from_observation(process)
            .ok_or("lineage_process_identity_missing")?;
        if let Some(state) = self.processes.get_mut(&id) {
            state.observation = process.clone();
            return Ok(id);
        }
        if self.processes.len() >= config.max_processes_per_trace as usize {
            return Err("lineage_process_capacity_exhausted");
        }
        self.processes
            .insert(id, IpcProcessState::new(process.clone()));
        Ok(id)
    }

    fn disable(
        &mut self,
        trace_id: TraceId,
        process: ProcessObservation,
        reason: &'static str,
        observed_ktime_ns: u64,
    ) {
        if self.disabled_reason.is_some() {
            return;
        }
        self.increment_diagnostic(reason);
        self.pending_degradations.push(StdioLineageDiagnostic {
            trace_id,
            process,
            operation: "lineage_disabled",
            observed_ktime_ns,
            reason,
        });
        let servers = self.active_bundles.keys().copied().collect::<Vec<_>>();
        for server in servers {
            self.close_bundle(trace_id, server, observed_ktime_ns, reason);
        }
        self.processes.clear();
        self.parents.clear();
        self.owners_by_channel.clear();
        self.bundles_by_channel.clear();
        self.last_gaps.clear();
        self.candidate_fd_count = 0;
        self.disabled_reason = Some(reason);
    }

    fn increment_diagnostic(&mut self, reason: &'static str) {
        let count = self.diagnostics.entry(reason).or_default();
        *count = count.saturating_add(1);
    }

    fn record_gap(&mut self, process: LineageProcessId, reason: &'static str) -> bool {
        if self.last_gaps.insert(process, reason) == Some(reason) {
            return false;
        }
        self.increment_diagnostic(reason);
        true
    }

    fn bind_fd(
        &mut self,
        process: LineageProcessId,
        fd: u32,
        binding: IpcEndpointBinding,
        config: &IpcLineageConfig,
    ) -> Result<BTreeSet<IpcChannelId>, &'static str> {
        let replacing = self
            .processes
            .get(&process)
            .is_some_and(|state| state.fds.contains_key(&fd));
        if !replacing && self.candidate_fd_count >= config.max_candidate_fds_per_trace as usize {
            return Err("lineage_candidate_fd_capacity_exhausted");
        }
        let mut changed = BTreeSet::new();
        if let Some(previous) = self.unbind_fd(process, fd) {
            changed.insert(previous.channel_id);
        }
        let Some(state) = self.processes.get_mut(&process) else {
            return Err("lineage_process_state_missing");
        };
        state.fds.insert(fd, binding.clone());
        self.candidate_fd_count = self.candidate_fd_count.saturating_add(1);
        self.owners_by_channel
            .entry(binding.channel_id.clone())
            .or_default()
            .insert(IpcEndpointOwner {
                process,
                fd,
                side: binding.side,
            });
        changed.insert(binding.channel_id);
        Ok(changed)
    }

    fn unbind_fd(&mut self, process: LineageProcessId, fd: u32) -> Option<IpcEndpointBinding> {
        let binding = self
            .processes
            .get_mut(&process)
            .and_then(|state| state.fds.remove(&fd))?;
        self.candidate_fd_count = self.candidate_fd_count.saturating_sub(1);
        let remove_channel =
            if let Some(owners) = self.owners_by_channel.get_mut(&binding.channel_id) {
                owners.remove(&IpcEndpointOwner {
                    process,
                    fd,
                    side: binding.side,
                });
                owners.is_empty()
            } else {
                false
            };
        if remove_channel {
            self.owners_by_channel.remove(&binding.channel_id);
        }
        Some(binding)
    }

    fn remove_process_state(&mut self, process: LineageProcessId) -> BTreeSet<IpcChannelId> {
        let fds = self
            .processes
            .get(&process)
            .map(|state| state.fds.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut changed = BTreeSet::new();
        for fd in fds {
            if let Some(binding) = self.unbind_fd(process, fd) {
                changed.insert(binding.channel_id);
            }
        }
        self.processes.remove(&process);
        self.parents.remove(&process);
        self.last_gaps.remove(&process);
        changed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IpcLineageTracker {
    enabled: bool,
    config: IpcLineageConfig,
    traces: BTreeMap<TraceId, TraceLineageState>,
    archived_diagnostics: BTreeMap<&'static str, u64>,
    archived_lifecycle: Vec<StdioBundleLifecycle>,
    archived_degradations: Vec<StdioLineageDiagnostic>,
    pending_output_traces: BTreeSet<TraceId>,
}

impl Default for IpcLineageTracker {
    fn default() -> Self {
        Self::new(IpcLineageConfig::default(), true)
    }
}
