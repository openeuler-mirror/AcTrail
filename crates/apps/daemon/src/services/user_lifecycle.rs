//! Routing of low-frequency agent lifecycle observations and periodic alerts.

use agent_lifecycle_contract::{
    SessionClosedEvent, TurnLifecycleEvent, UserInteractionEvent, UserLifecycleEvent,
    WorkLifecycleEvent,
};
use control_contract::reply::ControlError;

use crate::services::attach::StorageAttachService;

impl StorageAttachService {
    pub(in crate::services) fn report_turn_lifecycle_impl(
        &mut self,
        event: TurnLifecycleEvent,
    ) -> Result<(), ControlError> {
        self.observe_agent_execution(UserLifecycleEvent::Turn(event))
    }

    pub(in crate::services) fn report_user_interaction_impl(
        &mut self,
        event: UserInteractionEvent,
    ) -> Result<(), ControlError> {
        self.observe_agent_execution(UserLifecycleEvent::Interaction(event))
    }

    pub(in crate::services) fn report_session_closed_impl(
        &mut self,
        event: SessionClosedEvent,
    ) -> Result<(), ControlError> {
        self.observe_agent_execution(UserLifecycleEvent::SessionClosed(event))
    }

    pub(in crate::services) fn report_work_lifecycle_impl(
        &mut self,
        event: WorkLifecycleEvent,
    ) -> Result<(), ControlError> {
        self.observe_agent_execution(UserLifecycleEvent::Work(event))
    }

    fn observe_agent_execution(&mut self, event: UserLifecycleEvent) -> Result<(), ControlError> {
        self.agent_executions
            .observe(&event)
            .map_err(|message| ControlError::new("agent_execution_state", message))
    }

    pub(in crate::services) fn drain_idle_detection_alerts(
        &mut self,
        traces: &trace_runtime::TraceRuntime,
    ) {
        for alert in self.idle_detection.poll(&self.agent_executions) {
            let Some(host) = self.idle_detection_alert_host.as_ref() else {
                continue;
            };
            let Some(entry) = traces.get_trace(alert.trace_id) else {
                continue;
            };
            if entry.trace.lifecycle_state.is_terminal() {
                continue;
            }
            if let Err(error) = host.submit_observation_alert(
                alert.trace_id,
                entry.trace.alert_token.clone(),
                alert.draft,
            ) {
                tracing::warn!(trace_id = alert.trace_id.get(), code = %error.code, message = %error.message, "idle detection alert admission failed locally");
            }
        }
    }
}
