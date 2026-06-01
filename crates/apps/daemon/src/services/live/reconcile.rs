//! Reconcile draining traces against procfs when lifecycle exit events were missed.

use std::collections::BTreeSet;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::process::{ExitStatus, MembershipState, ProcessIdentity};
use model_core::trace::TraceLifecycleState;
use process_identity_contract::lookup::{IdentityLookupError, ProcessIdentityReader};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;

impl SqliteAttachService {
    pub(super) fn reconcile_draining_memberships_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let trace_ids = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| trace.lifecycle_state == TraceLifecycleState::Draining)
            .map(|trace| trace.trace_id)
            .collect::<Vec<_>>();
        let mut touched_traces = BTreeSet::new();

        for trace_id in trace_ids {
            let candidates = trace_runtime
                .get_trace(trace_id)
                .map(|entry| {
                    entry
                        .memberships
                        .memberships()
                        .filter(|membership| {
                            membership.capture_enabled
                                && matches!(
                                    membership.state,
                                    MembershipState::Starting | MembershipState::Active
                                )
                        })
                        .map(|membership| membership.identity.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for identity in candidates {
                if !self.process_membership_is_gone(&identity) {
                    continue;
                }
                trace_runtime
                    .mark_process_exited(
                        trace_id,
                        &identity,
                        ExitStatus {
                            code: None,
                            observed_at: SystemTime::now(),
                        },
                    )
                    .map_err(|error| {
                        ControlError::new("reconcile_draining_membership", format!("{:?}", error))
                    })?;
                touched_traces.insert(trace_id);
            }
        }

        for trace_id in touched_traces {
            self.persist_trace_state(trace_runtime, trace_id)?;
        }
        Ok(())
    }

    fn process_membership_is_gone(&self, identity: &ProcessIdentity) -> bool {
        match self.identity_reader.read_identity(identity.pid) {
            Ok(current) => {
                identity.pid_namespace.is_some()
                    && (current.start_time_ticks != identity.start_time_ticks
                        || current.pid_namespace != identity.pid_namespace)
            }
            Err(IdentityLookupError::NotFound { .. }) => true,
            Err(IdentityLookupError::PermissionDenied { .. })
            | Err(IdentityLookupError::Incomplete { .. }) => false,
        }
    }
}
