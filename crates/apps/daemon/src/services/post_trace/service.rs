use std::time::{Instant, SystemTime};

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use recording_runtime::RecordingWriter;

use crate::services::attach::StorageAttachService;

use super::coordinator::PostTraceIssue;

impl StorageAttachService {
    pub(in crate::services) fn drain_post_trace_runtime_impl(
        &mut self,
    ) -> Result<(), ControlError> {
        self.post_trace_broker
            .drain_requests(self.storage.as_mut())?;
        let outcomes = self
            .post_trace_coordinator
            .drain_completions(&self.export_runtime, self.storage.as_mut())?;
        for outcome in outcomes {
            match outcome.result {
                Ok(()) => tracing::info!(
                    trace_id = %outcome.trace_id,
                    plugin_instance = %outcome.instance_id,
                    "post-trace analysis completed"
                ),
                Err(error) => self.persist_post_trace_issue(PostTraceIssue {
                    trace_id: outcome.trace_id,
                    instance_id: outcome.instance_id,
                    code: error.code,
                    message: error.message,
                })?,
            }
        }
        Ok(())
    }

    pub(in crate::services) fn persist_post_trace_issues(
        &mut self,
        issues: Vec<PostTraceIssue>,
    ) -> Result<(), ControlError> {
        for issue in issues {
            self.persist_post_trace_issue(issue)?;
        }
        Ok(())
    }

    pub(in crate::services) fn drain_post_trace_instance_for_unload_impl(
        &mut self,
        instance_id: &str,
    ) -> Result<(), ControlError> {
        self.drain_post_trace_tasks(PostTraceDrainTarget::Instance(instance_id))?;
        self.post_trace_coordinator.forget_instance(instance_id)
    }

    pub(in crate::services) fn shutdown_post_trace_runtime_impl(
        &mut self,
    ) -> Result<(), ControlError> {
        self.post_trace_coordinator.close_admission();
        self.drain_post_trace_tasks(PostTraceDrainTarget::All)
    }

    fn drain_post_trace_tasks(
        &mut self,
        target: PostTraceDrainTarget<'_>,
    ) -> Result<(), ControlError> {
        let drain_timeout = self.post_trace_coordinator.shutdown_drain_timeout();
        let cancel_after = drain_timeout
            .checked_sub(self.post_trace_coordinator.cancellation_grace())
            .ok_or_else(|| {
                ControlError::new(
                    "post_trace_config",
                    "post-trace drain timeout does not reserve cancellation time",
                )
            })?;
        let started_at = Instant::now();
        loop {
            self.drain_post_trace_runtime_impl()?;
            if !target.has_running_tasks(&self.post_trace_coordinator) {
                return Ok(());
            }
            let elapsed = started_at.elapsed();
            if elapsed >= cancel_after {
                for instance_id in target.running_instance_ids(&self.post_trace_coordinator) {
                    self.export_runtime
                        .cancel_post_trace(&instance_id)
                        .map_err(|error| ControlError::new(error.code, error.message))?;
                }
            }
            if elapsed >= drain_timeout {
                let issues = self
                    .post_trace_coordinator
                    .diagnose_drain_timeout(target.instance_id());
                self.persist_post_trace_issues(issues)?;
                return Err(ControlError::new(
                    "post_trace_drain_timeout",
                    format!(
                        "{} still has running post-trace tasks after {}ms",
                        target.label(),
                        drain_timeout.as_millis()
                    ),
                ));
            }
            let remaining = drain_timeout.saturating_sub(elapsed);
            let mut sleep_for = self.finalization_poll_interval.min(remaining);
            if elapsed < cancel_after {
                sleep_for = sleep_for.min(cancel_after.saturating_sub(elapsed));
            }
            if sleep_for.is_zero() {
                std::thread::yield_now();
            } else {
                std::thread::sleep(sleep_for);
            }
        }
    }

    fn persist_post_trace_issue(&mut self, issue: PostTraceIssue) -> Result<(), ControlError> {
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
enum PostTraceDrainTarget<'a> {
    Instance(&'a str),
    All,
}

impl<'a> PostTraceDrainTarget<'a> {
    fn has_running_tasks(self, coordinator: &super::coordinator::PostTraceCoordinator) -> bool {
        match self {
            Self::Instance(instance_id) => coordinator.has_running_tasks_for(instance_id),
            Self::All => coordinator.has_running_tasks(),
        }
    }

    fn running_instance_ids(
        self,
        coordinator: &super::coordinator::PostTraceCoordinator,
    ) -> Vec<String> {
        match self {
            Self::Instance(instance_id) if coordinator.has_running_tasks_for(instance_id) => {
                vec![instance_id.to_string()]
            }
            Self::Instance(_) => Vec::new(),
            Self::All => coordinator.running_instance_ids(),
        }
    }

    fn instance_id(self) -> Option<&'a str> {
        match self {
            Self::Instance(instance_id) => Some(instance_id),
            Self::All => None,
        }
    }

    fn label(self) -> &'a str {
        match self {
            Self::Instance(instance_id) => instance_id,
            Self::All => "daemon shutdown",
        }
    }
}
