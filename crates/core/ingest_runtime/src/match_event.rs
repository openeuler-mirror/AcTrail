//! Matching observations to traces and process identities.

use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership, ProcessObservation};

use crate::IngestMatch;

pub fn match_membership(
    memberships: impl IntoIterator<Item = ProcessMembership>,
    observed: &ProcessIdentity,
) -> Option<IngestMatch> {
    memberships
        .into_iter()
        .find(|membership| membership.identity == *observed && membership.capture_enabled)
        .map(|membership| IngestMatch {
            trace_id: membership.trace_id,
            process: membership.identity,
            parent: None,
        })
}

pub fn match_by_pid(
    trace_id: TraceId,
    observed: &ProcessObservation,
    root_pid: u32,
) -> Option<IngestMatch> {
    (observed
        .host
        .as_ref()
        .is_some_and(|host| host.pid == root_pid))
    .then(|| IngestMatch {
        trace_id,
        process: ProcessIdentity::new(u64::from(root_pid)),
        parent: None,
    })
}
