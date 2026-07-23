//! Trace lifecycle and health transition ownership.

use std::time::SystemTime;

use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateTransitionError {
    pub from: TraceLifecycleState,
    pub to: TraceLifecycleState,
}

pub fn start_trace(
    trace: &mut TraceRecord,
    started_at: SystemTime,
) -> Result<(), StateTransitionError> {
    require_transition(trace.lifecycle_state, TraceLifecycleState::Active)?;
    trace.lifecycle_state = TraceLifecycleState::Active;
    trace.timings.started_at = Some(started_at);
    Ok(())
}

pub fn begin_draining(
    trace: &mut TraceRecord,
    _draining_at: SystemTime,
) -> Result<(), StateTransitionError> {
    require_transition(trace.lifecycle_state, TraceLifecycleState::Draining)?;
    trace.lifecycle_state = TraceLifecycleState::Draining;
    Ok(())
}

pub fn complete_trace(
    trace: &mut TraceRecord,
    completed_at: SystemTime,
) -> Result<(), StateTransitionError> {
    require_transition(trace.lifecycle_state, TraceLifecycleState::Completed)?;
    trace.lifecycle_state = TraceLifecycleState::Completed;
    trace.timings.completed_at = Some(completed_at);
    Ok(())
}

pub fn exit_trace(
    trace: &mut TraceRecord,
    exited_at: SystemTime,
) -> Result<(), StateTransitionError> {
    require_transition(trace.lifecycle_state, TraceLifecycleState::Exited)?;
    trace.lifecycle_state = TraceLifecycleState::Exited;
    trace.timings.exited_at = Some(exited_at);
    Ok(())
}

pub fn fail_trace(
    trace: &mut TraceRecord,
    failed_at: SystemTime,
) -> Result<(), StateTransitionError> {
    require_transition(trace.lifecycle_state, TraceLifecycleState::Failed)?;
    trace.lifecycle_state = TraceLifecycleState::Failed;
    trace.timings.failed_at = Some(failed_at);
    Ok(())
}

pub fn degrade_trace(trace: &mut TraceRecord) {
    trace.health = TraceHealth::Degraded;
}

fn require_transition(
    from: TraceLifecycleState,
    to: TraceLifecycleState,
) -> Result<(), StateTransitionError> {
    let allowed = matches!(
        (from, to),
        (TraceLifecycleState::Starting, TraceLifecycleState::Active)
            | (TraceLifecycleState::Starting, TraceLifecycleState::Failed)
            | (TraceLifecycleState::Active, TraceLifecycleState::Draining)
            | (TraceLifecycleState::Active, TraceLifecycleState::Completed)
            | (TraceLifecycleState::Active, TraceLifecycleState::Exited)
            | (TraceLifecycleState::Active, TraceLifecycleState::Failed)
            | (
                TraceLifecycleState::Draining,
                TraceLifecycleState::Completed
            )
            | (TraceLifecycleState::Draining, TraceLifecycleState::Exited)
            | (TraceLifecycleState::Draining, TraceLifecycleState::Failed)
    );

    if allowed {
        Ok(())
    } else {
        Err(StateTransitionError { from, to })
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use model_core::ids::{ProfileName, TraceId, TraceName};
    use model_core::process::ProcessIdentity;
    use model_core::trace::{TraceAlertToken, TraceLifecycleState, TraceRecord};

    use super::{begin_draining, complete_trace, start_trace};

    fn sample_trace() -> TraceRecord {
        TraceRecord::new(
            TraceId::new(1),
            TraceAlertToken::new([1; 32]),
            ProcessIdentity::new(1),
            TraceName::new("agent"),
            ProfileName::new("default"),
            SystemTime::UNIX_EPOCH,
        )
    }

    #[test]
    fn completed_trace_cannot_restart() {
        let mut trace = sample_trace();
        start_trace(&mut trace, SystemTime::UNIX_EPOCH).unwrap();
        complete_trace(&mut trace, SystemTime::UNIX_EPOCH).unwrap();

        let err = begin_draining(&mut trace, SystemTime::UNIX_EPOCH).unwrap_err();
        assert_eq!(err.from, TraceLifecycleState::Completed);
    }
}
