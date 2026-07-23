use std::time::{Instant, SystemTime};

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use recording_runtime::RecordingWriter;

use crate::services::attach::StorageAttachService;

use super::broker::AlertIngressIssue;

impl StorageAttachService {
    pub(in crate::services) fn drain_alert_ingress_impl(&mut self) -> Result<(), ControlError> {
        let issues = self.alert_ingress.drain_requests(self.storage.as_mut())?;
        for issue in issues {
            self.persist_alert_ingress_issue(issue)?;
        }
        Ok(())
    }

    pub(in crate::services) fn close_and_drain_alert_instance_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<(), ControlError> {
        self.alert_ingress.close_instance(instance_id)?;
        self.drain_alerts(AlertDrainTarget::Instance(instance_id))
    }

    pub(in crate::services) fn unregister_alert_instance_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<(), ControlError> {
        self.alert_ingress.unregister_plugin(instance_id)
    }

    pub(in crate::services) fn shutdown_alert_ingress_impl(&mut self) -> Result<(), ControlError> {
        self.alert_ingress.close_all()?;
        self.drain_alerts(AlertDrainTarget::All)
    }

    fn drain_alerts(&mut self, target: AlertDrainTarget<'_>) -> Result<(), ControlError> {
        let timeout = self.alert_ingress.drain_timeout();
        let started_at = Instant::now();
        loop {
            self.drain_alert_ingress_impl()?;
            if !target.has_outstanding_writes(&self.alert_ingress)? {
                return Ok(());
            }
            if started_at.elapsed() >= timeout {
                return Err(ControlError::new(
                    "alert_ingress_drain_timeout",
                    format!(
                        "{} still has outstanding alert writes after {}ms",
                        target.label(),
                        timeout.as_millis()
                    ),
                ));
            }
            let remaining = timeout.saturating_sub(started_at.elapsed());
            let sleep_for = self.finalization_poll_interval.min(remaining);
            if sleep_for.is_zero() {
                std::thread::yield_now();
            } else {
                std::thread::sleep(sleep_for);
            }
        }
    }

    fn persist_alert_ingress_issue(
        &mut self,
        issue: AlertIngressIssue,
    ) -> Result<(), ControlError> {
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(issue.trace_id),
            DiagnosticKind::RuntimeDropped,
            DiagnosticSeverity::Warning,
            SystemTime::now(),
            issue.message,
        )
        .with_metadata("code", issue.code)
        .with_metadata("plugin_instance", issue.instance_id);
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }
}

#[derive(Clone, Copy)]
enum AlertDrainTarget<'a> {
    Instance(&'a str),
    All,
}

impl<'a> AlertDrainTarget<'a> {
    fn has_outstanding_writes(
        self,
        ingress: &super::broker::AlertIngress,
    ) -> Result<bool, ControlError> {
        match self {
            Self::Instance(instance_id) => ingress.has_outstanding_writes_for(instance_id),
            Self::All => ingress.has_outstanding_writes(),
        }
    }

    fn label(self) -> &'a str {
        match self {
            Self::Instance(instance_id) => instance_id,
            Self::All => "daemon shutdown",
        }
    }
}
