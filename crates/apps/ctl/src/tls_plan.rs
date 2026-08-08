//! ctl-side launch TLS plan query helpers.

use std::path::Path;

use control_contract::command::{ControlCommand, ResolveLaunchTlsPlanCommand};
use control_contract::reply::{
    ControlError, ControlReply, LaunchTlsPlanReply, LaunchTlsPlanStatus,
};
use model_core::ids::RequestId;
use tls_payload_sync::RuntimePlanDescriptor;

use crate::transport::ControlClientPort;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueriedLaunchTlsPlan {
    pub(crate) descriptors: Vec<RuntimePlanDescriptor>,
    pub(crate) sources: Vec<String>,
    pub(crate) cache_hit: bool,
    pub(crate) resolve_elapsed_micros: u64,
}

pub(crate) fn query_launch_tls_plan_reply(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    binary: &Path,
) -> Result<LaunchTlsPlanReply, ControlError> {
    let reply = client.send(ControlCommand::ResolveLaunchTlsPlan(
        ResolveLaunchTlsPlanCommand {
            request_id,
            binary: binary.to_path_buf(),
        },
    ))?;
    let ControlReply::LaunchTlsPlan(reply) = reply else {
        return Err(ControlError::new(
            "tls_plan_query",
            "resolve launch TLS plan returned unexpected reply",
        ));
    };
    Ok(reply)
}

pub(crate) fn queried_plan_from_reply(
    reply: LaunchTlsPlanReply,
) -> Result<Option<QueriedLaunchTlsPlan>, String> {
    match reply.status {
        LaunchTlsPlanStatus::Found(plans) if plans.is_empty() => Ok(None),
        LaunchTlsPlanStatus::Found(plans) => Ok(Some(QueriedLaunchTlsPlan {
            descriptors: plans
                .iter()
                .map(|plan| RuntimePlanDescriptor {
                    target: plan.target.clone(),
                    target_identity: plan.target_identity.clone(),
                    binary: plan.binary.clone(),
                    binary_identity: plan.binary_identity.clone(),
                    provider: plan.provider.clone(),
                    points: plan.points.clone(),
                })
                .collect(),
            sources: plans.iter().map(|plan| plan.source.clone()).collect(),
            cache_hit: reply.cache_hit,
            resolve_elapsed_micros: reply.resolve_elapsed_micros,
        })),
        LaunchTlsPlanStatus::Unsupported { reason } => {
            if reason.is_empty() {
                Ok(None)
            } else {
                Err(reason)
            }
        }
    }
}

pub(crate) fn query_launch_tls_plan(
    client: &mut impl ControlClientPort,
    request_id: RequestId,
    binary: &Path,
) -> Result<Option<QueriedLaunchTlsPlan>, String> {
    query_launch_tls_plan_reply(client, request_id, binary)
        .map_err(|error| format!("{}: {}", error.code, error.message))
        .and_then(queried_plan_from_reply)
}
