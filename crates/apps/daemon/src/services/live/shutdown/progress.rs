//! Shutdown stage deadlines and progress diagnostics.

use super::*;

impl StorageAttachService {
    pub(super) fn begin_shutdown_stage_probe(
        &mut self,
        stage: ShutdownDrainStage,
        budget: Duration,
        remaining_count: usize,
        shutdown_started_at: Instant,
    ) -> ShutdownStageProbe {
        let mut probe = ShutdownStageProbe::new(stage, budget, shutdown_started_at);
        self.persist_shutdown_stage_diagnostic_fail_local(
            &probe,
            ShutdownStageStatus::Started,
            remaining_count,
        );
        probe.mark_stage_started();
        probe
    }

    pub(super) fn finish_shutdown_stage_probe(
        &mut self,
        probe: ShutdownStageProbe,
        remaining_count: usize,
        error: Option<&ControlError>,
    ) {
        let status = if error.is_some() {
            ShutdownStageStatus::Failed
        } else {
            ShutdownStageStatus::Completed
        };
        self.persist_shutdown_stage_diagnostic_fail_local(&probe, status, remaining_count);
    }

    fn persist_shutdown_stage_diagnostic_fail_local(
        &mut self,
        probe: &ShutdownStageProbe,
        status: ShutdownStageStatus,
        remaining_count: usize,
    ) {
        let stage_elapsed = probe.stage_elapsed();
        let shutdown_elapsed = probe.shutdown_elapsed();
        let slow = stage_elapsed >= probe.slow_threshold();
        let diagnostic_id = match self.next_diagnostic_id() {
            Ok(diagnostic_id) => diagnostic_id,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    stage = probe.stage.as_str(),
                    "shutdown drain diagnostic id allocation failed; continuing shutdown"
                );
                return;
            }
        };
        let diagnostic = DiagnosticRecord::new(
            diagnostic_id,
            None,
            DiagnosticKind::RuntimeFailure,
            status.severity(slow),
            SystemTime::now(),
            status.message(),
        )
        .with_metadata("component", "daemon_shutdown")
        .with_metadata("code", status.code())
        .with_metadata("stage", probe.stage.as_str())
        .with_metadata("status", status.as_str())
        .with_metadata("slow", slow.to_string())
        .with_metadata("remaining_unit", probe.stage.remaining_unit())
        .with_metadata(
            "elapsed_ms",
            ShutdownStageProbe::duration_millis(stage_elapsed).to_string(),
        )
        .with_metadata(
            "shutdown_elapsed_ms",
            ShutdownStageProbe::duration_millis(shutdown_elapsed).to_string(),
        )
        .with_metadata(
            "budget_ms",
            ShutdownStageProbe::duration_millis(probe.budget).to_string(),
        )
        .with_metadata(
            "remaining_count",
            u64::try_from(remaining_count)
                .unwrap_or(u64::MAX)
                .to_string(),
        );
        if let Err(error) =
            RecordingWriter::new(self.storage.as_mut()).persist_diagnostic(diagnostic)
        {
            tracing::warn!(
                error = ?error,
                stage = probe.stage.as_str(),
                "shutdown drain diagnostic persistence failed; continuing shutdown"
            );
        }
    }
}

pub(super) struct ShutdownDeadline {
    deadline: Option<Instant>,
}

impl ShutdownDeadline {
    pub(super) fn new(started_at: Instant, timeout: Duration) -> Self {
        Self {
            deadline: started_at.checked_add(timeout),
        }
    }

    pub(super) fn remaining_budget(&self, stage_limit: Duration) -> Duration {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
            .min(stage_limit)
    }

    pub(super) fn is_expired(&self) -> bool {
        self.deadline
            .is_none_or(|deadline| Instant::now() >= deadline)
    }
}

#[derive(Clone, Copy)]
pub(super) enum ShutdownDrainStage {
    TerminalFinalization,
    UnsettledSemantics,
    PostTrace,
    Export,
    Alert,
}

pub(super) fn shutdown_deadline_exhausted(stage: ShutdownDrainStage) -> ControlError {
    ControlError::new(
        "daemon_shutdown_deadline",
        format!(
            "global shutdown deadline exhausted before {} completed",
            stage.as_str()
        ),
    )
}

impl ShutdownDrainStage {
    const COUNT: u32 = 5;

    fn as_str(self) -> &'static str {
        match self {
            Self::TerminalFinalization => "terminal_finalization",
            Self::UnsettledSemantics => "unsettled_semantics",
            Self::PostTrace => "post_trace",
            Self::Export => "export",
            Self::Alert => "alert",
        }
    }

    fn remaining_unit(self) -> &'static str {
        match self {
            Self::TerminalFinalization | Self::UnsettledSemantics => "traces",
            Self::PostTrace => "plugin_instances",
            Self::Export => "observation_consumers",
            Self::Alert => "has_outstanding_writes",
        }
    }
}

pub(super) struct ShutdownStageProbe {
    stage: ShutdownDrainStage,
    budget: Duration,
    shutdown_started_at: Instant,
    stage_started_at: Instant,
}

impl ShutdownStageProbe {
    fn new(stage: ShutdownDrainStage, budget: Duration, shutdown_started_at: Instant) -> Self {
        Self {
            stage,
            budget,
            shutdown_started_at,
            stage_started_at: Instant::now(),
        }
    }

    fn stage_elapsed(&self) -> Duration {
        self.stage_started_at.elapsed()
    }

    fn mark_stage_started(&mut self) {
        self.stage_started_at = Instant::now();
    }

    fn shutdown_elapsed(&self) -> Duration {
        self.shutdown_started_at.elapsed()
    }

    fn slow_threshold(&self) -> Duration {
        self.budget / ShutdownDrainStage::COUNT
    }

    fn duration_millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone, Copy)]
enum ShutdownStageStatus {
    Started,
    Completed,
    Failed,
}

impl ShutdownStageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Started => "daemon_shutdown_stage_started",
            Self::Completed => "daemon_shutdown_stage_completed",
            Self::Failed => "daemon_shutdown_stage_failed",
        }
    }

    fn severity(self, slow: bool) -> DiagnosticSeverity {
        match self {
            Self::Started => DiagnosticSeverity::Info,
            Self::Completed if slow => DiagnosticSeverity::Warning,
            Self::Completed => DiagnosticSeverity::Info,
            Self::Failed => DiagnosticSeverity::Error,
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Started => "daemon shutdown drain stage entered",
            Self::Completed => "daemon shutdown drain stage completed",
            Self::Failed => "daemon shutdown drain stage failed",
        }
    }
}
