//! Dynamic INET-connect policy contracts exposed to control plugins.

use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr};

use actrail_plugin_abi::control::network_policy as network_policy_abi;

use crate::{ControlVerdict, PluginRuntimeError};

pub const NETWORK_ACTION_CURRENT_CONTEXT_TOKEN: &str =
    actrail_plugin_abi::control::context::CURRENT_NETWORK_ACTION;
pub const NETWORK_ACTION_CONTEXT_QUERY: &str =
    actrail_plugin_abi::control::query::NETWORK_ACTION_CONTEXT;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkPolicyDecision {
    Default,
    Allow,
    Deny,
    Gray,
}

impl NetworkPolicyDecision {
    pub fn code(self) -> u8 {
        match self {
            Self::Default => network_policy_abi::decision_code::DEFAULT,
            Self::Allow => network_policy_abi::decision_code::ALLOW,
            Self::Deny => network_policy_abi::decision_code::DENY,
            Self::Gray => network_policy_abi::decision_code::GRAY,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Gray => "gray",
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            network_policy_abi::decision_code::DEFAULT => Ok(Self::Default),
            network_policy_abi::decision_code::ALLOW => Ok(Self::Allow),
            network_policy_abi::decision_code::DENY => Ok(Self::Deny),
            network_policy_abi::decision_code::GRAY => Ok(Self::Gray),
            _ => Err(format!("unsupported network policy decision code {code}")),
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "gray" => Ok(Self::Gray),
            other => Err(format!(
                "unsupported network policy decision {other}; expected default, allow, deny, or gray"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkPolicyRemoteSelector {
    Endpoint(SocketAddr),
    AnyPort(IpAddr),
}

impl NetworkPolicyRemoteSelector {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if let Some(raw_ip) = raw.strip_suffix(":*") {
            let (raw_ip, bracketed) = match raw_ip
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                Some(value) => (value, true),
                None => (raw_ip, false),
            };
            let ip = raw_ip.parse::<IpAddr>().map_err(|error| {
                format!("network remote {raw} must contain a numeric IP before :*: {error}")
            })?;
            match (ip, bracketed) {
                (IpAddr::V4(_), true) => {
                    return Err(format!(
                        "network remote {raw} must not bracket an IPv4 address"
                    ));
                }
                (IpAddr::V6(_), false) => {
                    return Err(format!(
                        "network remote {raw} must bracket an IPv6 address as [ip]:*"
                    ));
                }
                _ => {}
            }
            return Ok(Self::AnyPort(ip));
        }
        raw.parse::<SocketAddr>()
            .map(Self::Endpoint)
            .map_err(|error| {
                format!(
                    "network remote {raw} must be a numeric IP endpoint or IP any-port selector: {error}"
                )
            })
    }
}

impl Display for NetworkPolicyRemoteSelector {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Endpoint(endpoint) => endpoint.fmt(formatter),
            Self::AnyPort(IpAddr::V4(ip)) => write!(formatter, "{ip}:*"),
            Self::AnyPort(IpAddr::V6(ip)) => write!(formatter, "[{ip}]:*"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkPolicyRemoteGrantScope {
    All,
    Selector(NetworkPolicyRemoteSelector),
}

impl NetworkPolicyRemoteGrantScope {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "*" {
            return Ok(Self::All);
        }
        NetworkPolicyRemoteSelector::parse(raw).map(Self::Selector)
    }

    pub fn covers(self, selector: NetworkPolicyRemoteSelector) -> bool {
        match self {
            Self::All => true,
            Self::Selector(NetworkPolicyRemoteSelector::AnyPort(ip)) => {
                ip == match selector {
                    NetworkPolicyRemoteSelector::Endpoint(endpoint) => endpoint.ip(),
                    NetworkPolicyRemoteSelector::AnyPort(ip) => ip,
                }
            }
            Self::Selector(NetworkPolicyRemoteSelector::Endpoint(endpoint)) => {
                selector == NetworkPolicyRemoteSelector::Endpoint(endpoint)
            }
        }
    }
}

impl Display for NetworkPolicyRemoteGrantScope {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => formatter.write_str("*"),
            Self::Selector(selector) => selector.fmt(formatter),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPolicyPatchOp {
    Upsert,
    Delete,
}

impl NetworkPolicyPatchOp {
    pub fn code(self) -> u8 {
        match self {
            Self::Upsert => network_policy_abi::patch_op_code::UPSERT,
            Self::Delete => network_policy_abi::patch_op_code::DELETE,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            network_policy_abi::patch_op_code::UPSERT => Ok(Self::Upsert),
            network_policy_abi::patch_op_code::DELETE => Ok(Self::Delete),
            _ => Err(format!("unsupported network policy patch op code {code}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPolicyApplyStatus {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyRuleDraft {
    pub rule_id: Option<String>,
    pub decision: NetworkPolicyDecision,
    pub remote: String,
    pub gray_target: Option<String>,
    pub timeout_ms: Option<u64>,
    pub concurrency_limit: Option<u32>,
    pub fallback: Option<ControlVerdict>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyPatchItem {
    pub op: NetworkPolicyPatchOp,
    pub rule_id: Option<String>,
    pub rule: Option<NetworkPolicyRuleDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyApplyRequest {
    pub base_revision: u64,
    pub mutation_id: String,
    pub reason: Option<String>,
    pub items: Vec<NetworkPolicyPatchItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyApplyError {
    pub item_index: u32,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyApplyResult {
    pub status: NetworkPolicyApplyStatus,
    pub new_revision: u64,
    pub applied_count: u32,
    pub rejected_count: u32,
    pub errors: Vec<NetworkPolicyApplyError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyRuleView {
    pub rule_id: String,
    pub owner_instance_id: String,
    pub decision: NetworkPolicyDecision,
    pub remote: String,
    pub gray_target: Option<String>,
    pub timeout_ms: Option<u64>,
    pub concurrency_limit: Option<u32>,
    pub fallback: Option<ControlVerdict>,
    pub rule_revision: u64,
    pub updated_sequence: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkPolicyListFilter {
    pub decision: Option<NetworkPolicyDecision>,
    pub remote: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyListResult {
    pub rules: Vec<NetworkPolicyRuleView>,
    pub next_cursor: Option<String>,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyMatchDryRunRequest {
    pub remote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkPolicyMatchDryRunResult {
    pub matched: bool,
    pub decision: NetworkPolicyDecision,
    pub rule_id: Option<String>,
    pub owner_instance_id: Option<String>,
    pub resolved_remote: String,
    pub rule_revision: Option<u64>,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkActionContext {
    pub syscall: String,
    pub fd: u64,
    pub address_family: String,
    pub remote_address: String,
    pub remote_port: u16,
    pub ipv6_scope_id: u32,
}

pub trait NetworkPolicyHost: Send + Sync {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError>;

    fn rules_list(
        &self,
        filter: NetworkPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<NetworkPolicyListResult, PluginRuntimeError>;

    fn rules_match_dry_run(
        &self,
        request: NetworkPolicyMatchDryRunRequest,
    ) -> Result<NetworkPolicyMatchDryRunResult, PluginRuntimeError>;

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[crate::NetworkPolicyRulesApplyGrant],
        request: &NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError>;

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[crate::NetworkPolicyRulesApplyGrant],
        request: NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError>;
}
