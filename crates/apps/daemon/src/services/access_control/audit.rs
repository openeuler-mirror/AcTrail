use access_control::lsm::LsmAuditEvent;
use control_contract::reply::ControlError;
use model_core::event::{
    DomainEvent, EnforcementPayload, EventEnvelope, EventFlags, EventKind, EventPayload,
};
use model_core::ids::{CollectorName, TraceId};
use std::collections::BTreeMap;
use trace_runtime::registry::TraceRuntime;

use super::LsmAccessControl;
use crate::services::attach::StorageAttachService;

impl StorageAttachService {
    pub(in crate::services) fn drain_lsm_audit(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        self.lsm_boundary_alerts
            .flush_due(&self.alert_ingress, trace_runtime);
        let audits = self
            .collector
            .drain_lsm_events()
            .map_err(|message| ControlError::new("lsm_audit", message))?;
        let mut events = Vec::with_capacity(audits.len());
        let mut records = Vec::new();
        for audit in audits {
            let trace_id = TraceId::new(audit.trace_id);
            if trace_runtime.get_trace(trace_id).is_none() {
                tracing::warn!(%trace_id, "LSM audit references an unavailable trace");
                continue;
            }
            let observation = match self.collector.resolve_lsm_observation(&audit) {
                Ok(observation) => observation,
                Err(message) => {
                    tracing::warn!(%trace_id, %message, "LSM audit identity is unavailable");
                    continue;
                }
            };
            let (process, record) = match self.resolve_process_observation(observation) {
                Ok(resolution) => resolution,
                Err(error) => {
                    tracing::warn!(%trace_id, error.message, "LSM audit process registration failed");
                    continue;
                }
            };
            records.extend(record);
            let observed_at = self.collector.lsm_observed_at(&audit);
            self.lsm_boundary_alerts.record(
                &audit,
                process.get(),
                observed_at,
                &self.alert_ingress,
                trace_runtime,
            );
            if !audit.retain_audit {
                continue;
            }
            let payload = match LsmAuditProjection::new(&audit).payload() {
                Ok(payload) => payload,
                Err(message) => {
                    tracing::warn!(%trace_id, %message, "LSM audit projection failed");
                    continue;
                }
            };
            events.push(DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id,
                    observed_at,
                    process,
                    collector: CollectorName::new(LsmAccessControl::COLLECTOR_NAME),
                    kind: EventKind::Enforcement,
                    flags: EventFlags {
                        metadata_partial: audit.error != 0,
                        ..EventFlags::clean()
                    },
                },
                EventPayload::Enforcement(payload),
            ));
        }
        self.persist_observed_event_batch_with_process_records(trace_runtime, events, records)
    }
}

struct LsmAuditProjection<'a> {
    audit: &'a LsmAuditEvent,
}

impl<'a> LsmAuditProjection<'a> {
    fn new(audit: &'a LsmAuditEvent) -> Self {
        Self { audit }
    }

    fn payload(&self) -> Result<EnforcementPayload, String> {
        let audit = self.audit;
        let (subject, operation) = match audit.domain {
            1 => ("file-access", "open"),
            2 => ("file-access", "mkdir"),
            3 => ("file-access", "rmdir"),
            4 => ("command-execution", "exec"),
            5 => ("network-action", "connect"),
            other => return Err(format!("unknown LSM audit domain {other}")),
        };
        let (decision, result) = match audit.decision {
            0 => ("allow", "allowed"),
            1 => ("deny", "denied"),
            other => return Err(format!("unknown LSM audit decision {other}")),
        };
        let source = if audit.error != 0 {
            "failure"
        } else if audit.rule_id.is_empty() {
            "default"
        } else {
            "rule"
        };
        let mut metadata = BTreeMap::from([
            ("subject".into(), subject.into()),
            ("decision_source".into(), source.into()),
            ("rule_revision".into(), audit.revision.to_string()),
            ("policy_owner_instance_id".into(), audit.owner.clone()),
            ("kernel_tgid".into(), audit.kernel_tgid.to_string()),
            ("observer_tgid".into(), audit.observer_tgid.to_string()),
            (
                "start_boottime_ns".into(),
                audit.start_boottime_ns.to_string(),
            ),
            ("signal_result".into(), audit.signal_result.to_string()),
        ]);
        if audit.error != 0 {
            metadata.insert("resource_error".into(), audit.error.to_string());
        }
        if audit.domain == 5 {
            metadata.insert("remote_endpoint".into(), audit.resource.clone());
        }
        Ok(EnforcementPayload {
            backend: "bpf-lsm".into(),
            operation: operation.into(),
            decision: decision.into(),
            path: (audit.domain != 5 && !audit.resource.is_empty()).then(|| audit.resource.clone()),
            rule_id: (!audit.rule_id.is_empty()).then(|| audit.rule_id.clone()),
            result: result.into(),
            metadata,
        })
    }
}
