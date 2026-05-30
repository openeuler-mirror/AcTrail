//! Matching observations to traces and process identities.

use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership};

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
        })
}

pub fn match_by_pid(
    trace_id: TraceId,
    observed: &ProcessIdentity,
    root_pid: u32,
) -> Option<IngestMatch> {
    (observed.pid == root_pid).then(|| IngestMatch {
        trace_id,
        process: observed.clone(),
    })
}
