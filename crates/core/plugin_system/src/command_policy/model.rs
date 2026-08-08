//! Dynamic command-execution policy contracts exposed to control plugins.

use actrail_plugin_abi::control::command_policy as command_policy_abi;

use crate::PluginRuntimeError;

pub const COMMAND_EXECUTION_CURRENT_CONTEXT_TOKEN: &str =
    actrail_plugin_abi::control::context::CURRENT_COMMAND_EXECUTION;
pub const COMMAND_EXECUTION_CONTEXT_QUERY: &str =
    actrail_plugin_abi::control::query::COMMAND_EXECUTION_CONTEXT;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CommandPolicyDecision {
    Default,
    Allow,
    Deny,
    Gray,
}

impl CommandPolicyDecision {
    pub fn code(self) -> u8 {
        match self {
            Self::Default => command_policy_abi::decision_code::DEFAULT,
            Self::Allow => command_policy_abi::decision_code::ALLOW,
            Self::Deny => command_policy_abi::decision_code::DENY,
            Self::Gray => command_policy_abi::decision_code::GRAY,
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
            command_policy_abi::decision_code::DEFAULT => Ok(Self::Default),
            command_policy_abi::decision_code::ALLOW => Ok(Self::Allow),
            command_policy_abi::decision_code::DENY => Ok(Self::Deny),
            command_policy_abi::decision_code::GRAY => Ok(Self::Gray),
            _ => Err(format!("unsupported command policy decision code {code}")),
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "gray" => Ok(Self::Gray),
            other => Err(format!(
                "unsupported command policy decision {other}; expected default, allow, deny, or gray"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPolicyPatchOp {
    Upsert,
    Delete,
}

impl CommandPolicyPatchOp {
    pub fn code(self) -> u8 {
        match self {
            Self::Upsert => command_policy_abi::patch_op_code::UPSERT,
            Self::Delete => command_policy_abi::patch_op_code::DELETE,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            command_policy_abi::patch_op_code::UPSERT => Ok(Self::Upsert),
            command_policy_abi::patch_op_code::DELETE => Ok(Self::Delete),
            _ => Err(format!("unsupported command policy patch op code {code}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPolicyApplyStatus {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyRuleDraft {
    pub rule_id: Option<String>,
    pub decision: CommandPolicyDecision,
    pub executable: String,
    pub args: Option<Vec<String>>,
    pub gray_target: Option<String>,
    pub priority: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyPatchItem {
    pub op: CommandPolicyPatchOp,
    pub rule_id: Option<String>,
    pub rule: Option<CommandPolicyRuleDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyApplyRequest {
    pub base_revision: u64,
    pub mutation_id: String,
    pub reason: Option<String>,
    pub items: Vec<CommandPolicyPatchItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyApplyError {
    pub item_index: u32,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyApplyResult {
    pub status: CommandPolicyApplyStatus,
    pub new_revision: u64,
    pub applied_count: u32,
    pub rejected_count: u32,
    pub errors: Vec<CommandPolicyApplyError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyRuleView {
    pub rule_id: String,
    pub owner_instance_id: String,
    pub decision: CommandPolicyDecision,
    pub executable: String,
    pub args: Option<Vec<String>>,
    pub gray_target: Option<String>,
    pub priority: i32,
    pub rule_revision: u64,
    pub updated_sequence: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandPolicyListFilter {
    pub decision: Option<CommandPolicyDecision>,
    pub executable_prefix: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyListResult {
    pub rules: Vec<CommandPolicyRuleView>,
    pub next_cursor: Option<String>,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyMatchDryRunRequest {
    pub executable: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPolicyMatchDryRunResult {
    pub matched: bool,
    pub decision: CommandPolicyDecision,
    pub rule_id: Option<String>,
    pub owner_instance_id: Option<String>,
    pub resolved_executable: String,
    pub rule_revision: Option<u64>,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandExecutionContext {
    pub syscall: String,
    pub requested_path: String,
    pub resolved_path: String,
    pub argv: Vec<String>,
    pub execveat_dirfd: Option<i32>,
    pub execveat_flags: Option<u64>,
}

pub trait CommandPolicyHost: Send + Sync {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError>;

    fn rules_list(
        &self,
        filter: CommandPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<CommandPolicyListResult, PluginRuntimeError>;

    fn rules_match_dry_run(
        &self,
        request: CommandPolicyMatchDryRunRequest,
    ) -> Result<CommandPolicyMatchDryRunResult, PluginRuntimeError>;

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[crate::CommandPolicyRulesApplyGrant],
        request: &CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError>;

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[crate::CommandPolicyRulesApplyGrant],
        request: CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError>;
}
