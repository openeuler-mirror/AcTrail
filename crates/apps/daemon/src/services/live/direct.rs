//! Consume TLS discovery facts before optional process projection.

use collector_event::{RawCollectorEvent, RawObservationPayload};
use collector_instance::CollectorInstance;
use model_core::capability::Capability;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::tls_sync::DirectObject;

impl StorageAttachService {
    pub(super) fn discover_direct_mappings(&mut self) {
        for event in self.collector.take_tls_mapping_events() {
            if !self.tls_sync.direct_discovery_enabled() {
                continue;
            }
            let pid = event.common.subject.observer_namespace_tgid;
            if let Some(object) =
                DirectObject::open_mapping(pid, event.start, event.end, &event.file_identity)
            {
                if let Some(resolution) = self.tls_sync.submit_direct(object) {
                    resolution.attach(&mut self.collector);
                }
            }
        }
    }

    pub(super) fn discover_direct_exec(
        &mut self,
        event: &RawCollectorEvent,
        runtime: &TraceRuntime,
    ) -> bool {
        if event.envelope.collector != self.collector.descriptor().name {
            return true;
        }
        let RawObservationPayload::Process {
            operation,
            exec_file_identity,
            ..
        } = &event.payload
        else {
            return true;
        };
        if operation != "exec" {
            return true;
        }
        let Some(trace) = event.envelope.trace_id.and_then(|id| runtime.get_trace(id)) else {
            return true;
        };
        let Some(plan) = trace
            .sensor_plan
            .collectors
            .iter()
            .find(|plan| plan.collector_name == event.envelope.collector)
        else {
            return false;
        };
        if self.tls_sync.direct_discovery_enabled()
            && plan.capabilities.contains(&Capability::TlsPlaintextPayload)
        {
            if let (Some(identity), Some(host)) = (
                exec_file_identity.as_ref(),
                event.envelope.process.host.as_ref(),
            ) {
                // host.pid is the observer's procfs coordinate; the file facts validate the opened object.
                if let Some(object) = DirectObject::open(host.pid, identity) {
                    if let Some(resolution) = self.tls_sync.submit_direct(object) {
                        resolution.attach(&mut self.collector);
                    }
                } else {
                    tracing::debug!(pid = host.pid, "TLS direct executable file unavailable");
                }
            }
        }
        plan.capabilities.contains(&Capability::ProcLifecycle)
            || plan.capabilities.contains(&Capability::ProcExecContext)
    }
}
