//! Typed grants for policy publication.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyRulesApplyGrant {
    pub decision: FilePolicyDecision,
    pub path_scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyRulesApplyGrant {
    pub decision: CommandPolicyDecision,
    pub path_scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyRulesApplyGrant {
    pub decision: NetworkPolicyDecision,
    pub operation: NetworkPolicyOperation,
    pub protocol: NetworkPolicyProtocol,
    pub endpoint_scope: NetworkPolicyEndpointGrantScope,
}
