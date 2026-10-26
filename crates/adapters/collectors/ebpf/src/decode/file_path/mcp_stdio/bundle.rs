use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessObservation;

use super::{FdIpcKind, IpcChannelId, IpcEndpointBinding, LineageProcessId};

impl FdIpcKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::UnixSocket => "unix_socket",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct StdioBundleId {
    pub(super) server: LineageProcessId,
    pub(super) exec_ktime_ns: u64,
}

impl StdioBundleId {
    fn stable_id(&self, trace_id: TraceId) -> String {
        format!(
            "stdio-bundle:{}:{}:{}:{}",
            trace_id.get(),
            self.server.host_pid,
            self.server.generation,
            self.exec_ktime_ns,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StdioBundle {
    pub(super) id: StdioBundleId,
    pub(super) stdin: IpcEndpointBinding,
    pub(super) stdout: IpcEndpointBinding,
    pub(super) stderr: Option<IpcEndpointBinding>,
    pub(super) client: LineageProcessId,
}

impl StdioBundle {
    pub(super) fn channels(&self) -> BTreeSet<IpcChannelId> {
        let mut channels = BTreeSet::from([
            self.stdin.channel_id.clone(),
            self.stdout.channel_id.clone(),
        ]);
        if let Some(stderr) = &self.stderr {
            channels.insert(stderr.channel_id.clone());
        }
        channels
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StdioBundleLifecycle {
    pub(super) trace_id: TraceId,
    pub(super) server: ProcessObservation,
    pub(super) operation: &'static str,
    pub(super) bundle: StdioBundle,
    pub(super) observed_ktime_ns: u64,
    pub(super) reason: Option<&'static str>,
}

impl StdioBundleLifecycle {
    pub(super) fn into_raw_event(self) -> RawCollectorEvent {
        let mut metadata = BTreeMap::from([
            ("operation".to_string(), self.operation.to_string()),
            (
                "bundle_id".to_string(),
                self.bundle.id.stable_id(self.trace_id),
            ),
            (
                "exec_ktime_ns".to_string(),
                self.bundle.id.exec_ktime_ns.to_string(),
            ),
            (
                "observed_ktime_ns".to_string(),
                self.observed_ktime_ns.to_string(),
            ),
            (
                "stdin_channel_id".to_string(),
                self.bundle.stdin.channel_id.stable_id(self.trace_id),
            ),
            (
                "stdin_kind".to_string(),
                self.bundle.stdin.kind.as_str().to_string(),
            ),
            (
                "stdout_channel_id".to_string(),
                self.bundle.stdout.channel_id.stable_id(self.trace_id),
            ),
            (
                "stdout_kind".to_string(),
                self.bundle.stdout.kind.as_str().to_string(),
            ),
            (
                "client_host_pid".to_string(),
                self.bundle.client.host_pid.to_string(),
            ),
            (
                "client_generation".to_string(),
                self.bundle.client.generation.to_string(),
            ),
        ]);
        if let Some(stderr) = &self.bundle.stderr {
            metadata.insert(
                "stderr_channel_id".to_string(),
                stderr.channel_id.stable_id(self.trace_id),
            );
            metadata.insert("stderr_kind".to_string(), stderr.kind.as_str().to_string());
        }
        if let Some(reason) = self.reason {
            metadata.insert("reason".to_string(), reason.to_string());
        }
        RawCollectorEvent {
            envelope: RawEventEnvelope {
                trace_id: Some(self.trace_id),
                observed_at: SystemTime::now(),
                process: self.server,
                collector: CollectorName::new("ebpf"),
            },
            payload: RawObservationPayload::Ipc {
                channel: "stdio_bundle".to_string(),
                peer: Some(format!("host-pid:{}", self.bundle.client.host_pid)),
                metadata,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StdioLineageDiagnostic {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessObservation,
    pub(super) operation: &'static str,
    pub(super) observed_ktime_ns: u64,
    pub(super) reason: &'static str,
}

impl StdioLineageDiagnostic {
    pub(super) fn into_raw_event(self) -> RawCollectorEvent {
        RawCollectorEvent {
            envelope: RawEventEnvelope {
                trace_id: Some(self.trace_id),
                observed_at: SystemTime::now(),
                process: self.process,
                collector: CollectorName::new("ebpf"),
            },
            payload: RawObservationPayload::Ipc {
                channel: "stdio_bundle".to_string(),
                peer: None,
                metadata: BTreeMap::from([
                    ("operation".to_string(), self.operation.to_string()),
                    (
                        "observed_ktime_ns".to_string(),
                        self.observed_ktime_ns.to_string(),
                    ),
                    ("reason".to_string(), self.reason.to_string()),
                ]),
            },
        }
    }
}
