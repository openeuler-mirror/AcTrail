use alert_delivery_contract::{
    DeliverySeverity, DeliverySource, ForwardAlert, SandboxDeliverySource, SandboxProcessMarker,
};
use alert_forwarding::AlertForwardingPlugin;
use sandbox_alert_store::{SandboxAlertCommitPort, SandboxAlertKind, StoredSandboxAlert};
use serde_json::{Map, Value};

pub(crate) struct SandboxAlertForwarder {
    forwarding: AlertForwardingPlugin,
}

impl SandboxAlertForwarder {
    pub(crate) fn new(forwarding: AlertForwardingPlugin) -> Self {
        Self { forwarding }
    }

    fn normalize(&self, stored: StoredSandboxAlert) -> ForwardAlert {
        let alert = stored.alert;
        let mut extras = Map::new();
        extras.insert(
            "batch_sequence".to_string(),
            Value::from(alert.batch_sequence()),
        );
        extras.insert(
            "observation_index".to_string(),
            Value::from(alert.observation_index()),
        );
        let (severity, category, description, boot_id, process) = match alert.kind() {
            SandboxAlertKind::HighCpu {
                guest_boot_id,
                usage_basis_points,
                threshold_basis_points,
                ..
            } => {
                extras.insert(
                    "usage_basis_points".to_string(),
                    Value::from(usage_basis_points),
                );
                extras.insert(
                    "threshold_basis_points".to_string(),
                    Value::from(threshold_basis_points),
                );
                (
                    DeliverySeverity::Warning,
                    "sandbox.resource.high_cpu",
                    "Sandbox CPU usage crossed threshold",
                    guest_boot_id,
                    None,
                )
            }
            SandboxAlertKind::OomKilled {
                guest_boot_id,
                previous_count,
                current_count,
                delta,
                ..
            } => {
                extras.insert("previous_count".to_string(), Value::from(previous_count));
                extras.insert("current_count".to_string(), Value::from(current_count));
                extras.insert("delta".to_string(), Value::from(delta));
                (
                    DeliverySeverity::Critical,
                    "sandbox.resource.oom_killed",
                    "Sandbox OOM kill count increased",
                    guest_boot_id,
                    None,
                )
            }
            SandboxAlertKind::OomRisk {
                guest_boot_id,
                available_bytes,
                threshold_bytes,
                ..
            } => {
                extras.insert("available_bytes".to_string(), Value::from(available_bytes));
                extras.insert("threshold_bytes".to_string(), Value::from(threshold_bytes));
                (
                    DeliverySeverity::Warning,
                    "sandbox.resource.oom_risk",
                    "Sandbox available memory crossed threshold",
                    guest_boot_id,
                    None,
                )
            }
            SandboxAlertKind::HighRead {
                guest_boot_id,
                process,
                sample_started_ms,
                bytes,
                threshold_bytes,
                ..
            } => {
                Self::insert_io_extras(&mut extras, sample_started_ms, bytes, threshold_bytes);
                (
                    DeliverySeverity::Warning,
                    "sandbox.process.high_read",
                    "Sandbox process read bytes crossed threshold",
                    guest_boot_id,
                    Some(process),
                )
            }
            SandboxAlertKind::HighWrite {
                guest_boot_id,
                process,
                sample_started_ms,
                bytes,
                threshold_bytes,
                ..
            } => {
                Self::insert_io_extras(&mut extras, sample_started_ms, bytes, threshold_bytes);
                (
                    DeliverySeverity::Warning,
                    "sandbox.process.high_write",
                    "Sandbox process write bytes crossed threshold",
                    guest_boot_id,
                    Some(process),
                )
            }
        };
        let source = alert.source();
        ForwardAlert {
            detected_at_ms: alert.detected_at_ms(),
            severity,
            source: DeliverySource::Sandbox(SandboxDeliverySource {
                gateway_id: source.gateway_id(),
                sb_id: source.sb_id(),
                boot_id: *boot_id.as_bytes(),
                process: process.map(|process| SandboxProcessMarker {
                    pid: process.pid,
                    start_time_ticks: process.start_time_ticks,
                    executable_name: process.executable_name,
                }),
            }),
            category: category.to_string(),
            description: Some(description.to_string()),
            extras,
        }
    }

    fn insert_io_extras(
        extras: &mut Map<String, Value>,
        sample_started_ms: u64,
        bytes: u64,
        threshold_bytes: u64,
    ) {
        extras.insert(
            "sample_started_ms".to_string(),
            Value::from(sample_started_ms),
        );
        extras.insert("bytes".to_string(), Value::from(bytes));
        extras.insert("threshold_bytes".to_string(), Value::from(threshold_bytes));
    }

    fn category(kind: SandboxAlertKind) -> &'static str {
        match kind {
            SandboxAlertKind::HighCpu { .. } => "sandbox.resource.high_cpu",
            SandboxAlertKind::OomKilled { .. } => "sandbox.resource.oom_killed",
            SandboxAlertKind::OomRisk { .. } => "sandbox.resource.oom_risk",
            SandboxAlertKind::HighRead { .. } => "sandbox.process.high_read",
            SandboxAlertKind::HighWrite { .. } => "sandbox.process.high_write",
        }
    }
}

impl SandboxAlertCommitPort for SandboxAlertForwarder {
    fn committed(&self, alert: StoredSandboxAlert) {
        if !self
            .forwarding
            .accepts_category(Self::category(alert.alert.kind()))
        {
            return;
        }
        let _ = self.forwarding.try_publish(self.normalize(alert));
    }
}
