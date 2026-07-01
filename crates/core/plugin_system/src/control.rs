use std::sync::Arc;

use actrail_plugin_abi::control as control_abi;

use crate::{PluginHostcallMetricsSource, PluginRuntimeError, PluginRuntimeKind};

pub const CONTROL_CURRENT_CONTEXT_TOKEN: &str = control_abi::context::CURRENT_DECISION;
pub const FILE_POLICY_CURRENT_CONTEXT_TOKEN: &str = control_abi::context::CURRENT_FILE_POLICY;
pub const CONTROL_DECISION_SUMMARY_QUERY: &str = control_abi::query::DECISION_SUMMARY;
pub const FILE_POLICY_MATCHED_RULE_QUERY: &str = control_abi::query::MATCHED_RULE;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FilePolicyDecision {
    Default,
    Allow,
    Deny,
    Gray,
}

impl FilePolicyDecision {
    pub fn code(self) -> u8 {
        match self {
            Self::Default => control_abi::file_policy::decision_code::DEFAULT,
            Self::Allow => control_abi::file_policy::decision_code::ALLOW,
            Self::Deny => control_abi::file_policy::decision_code::DENY,
            Self::Gray => control_abi::file_policy::decision_code::GRAY,
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
            control_abi::file_policy::decision_code::DEFAULT => Ok(Self::Default),
            control_abi::file_policy::decision_code::ALLOW => Ok(Self::Allow),
            control_abi::file_policy::decision_code::DENY => Ok(Self::Deny),
            control_abi::file_policy::decision_code::GRAY => Ok(Self::Gray),
            _ => Err(format!("unsupported file policy decision code {code}")),
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "gray" => Ok(Self::Gray),
            other => Err(format!(
                "unsupported file policy decision {other}; expected default, allow, deny, or gray"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FilePolicyOperation {
    Open,
}

impl FilePolicyOperation {
    pub fn code(self) -> u8 {
        match self {
            Self::Open => control_abi::file_policy::operation_code::OPEN,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            control_abi::file_policy::operation_code::OPEN => Ok(Self::Open),
            _ => Err(format!("unsupported file policy operation code {code}")),
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "open" => Ok(Self::Open),
            other => Err(format!(
                "unsupported file policy operation {other}; expected open"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePolicyPatchOp {
    Upsert,
    Delete,
    Enable,
    Disable,
}

impl FilePolicyPatchOp {
    pub fn code(self) -> u8 {
        match self {
            Self::Upsert => control_abi::file_policy::patch_op_code::UPSERT,
            Self::Delete => control_abi::file_policy::patch_op_code::DELETE,
            Self::Enable => control_abi::file_policy::patch_op_code::ENABLE,
            Self::Disable => control_abi::file_policy::patch_op_code::DISABLE,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            control_abi::file_policy::patch_op_code::UPSERT => Ok(Self::Upsert),
            control_abi::file_policy::patch_op_code::DELETE => Ok(Self::Delete),
            control_abi::file_policy::patch_op_code::ENABLE => Ok(Self::Enable),
            control_abi::file_policy::patch_op_code::DISABLE => Ok(Self::Disable),
            _ => Err(format!("unsupported file policy patch op code {code}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilePolicyApplyMode {
    #[default]
    Partial,
    Aon,
}

impl FilePolicyApplyMode {
    pub fn code(self) -> u8 {
        match self {
            Self::Partial => control_abi::file_policy::apply_mode_code::PARTIAL,
            Self::Aon => control_abi::file_policy::apply_mode_code::AON,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, String> {
        match code {
            control_abi::file_policy::apply_mode_code::PARTIAL => Ok(Self::Partial),
            control_abi::file_policy::apply_mode_code::AON => Ok(Self::Aon),
            _ => Err(format!("unsupported file policy apply mode code {code}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePolicyApplyStatus {
    Accepted,
    Rejected,
}

impl FilePolicyApplyStatus {
    pub fn code(self) -> u8 {
        match self {
            Self::Accepted => control_abi::file_policy::apply_status_code::ACCEPTED,
            Self::Rejected => control_abi::file_policy::apply_status_code::REJECTED,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyRuleDraft {
    pub rule_id: Option<String>,
    pub decision: FilePolicyDecision,
    pub operation: FilePolicyOperation,
    pub path: String,
    pub gray_target: Option<u64>,
    pub priority: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyPatchItem {
    pub op: FilePolicyPatchOp,
    pub rule_id: Option<String>,
    pub rule: Option<FilePolicyRuleDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyApplyPrecondition {
    pub base_revision: u64,
    pub mutation_id: String,
    pub reason: Option<String>,
    pub correlation_id: Option<String>,
    pub apply_mode: FilePolicyApplyMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyApplyRequest {
    pub items: Vec<FilePolicyPatchItem>,
    pub precondition: FilePolicyApplyPrecondition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyApplyError {
    pub item_index: u32,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyApplyResult {
    pub status: FilePolicyApplyStatus,
    pub new_revision: u64,
    pub applied_count: u32,
    pub rejected_count: u32,
    pub errors: Vec<FilePolicyApplyError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyRuleView {
    pub rule_id: String,
    pub owner_instance_id: String,
    pub decision: FilePolicyDecision,
    pub operation: FilePolicyOperation,
    pub path: String,
    pub gray_target: Option<u64>,
    pub priority: i32,
    pub enabled: bool,
    pub updated_sequence: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilePolicyListFilter {
    pub decision: Option<FilePolicyDecision>,
    pub path_prefix: Option<String>,
    pub operation: Option<FilePolicyOperation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyListResult {
    pub rules: Vec<FilePolicyRuleView>,
    pub next_cursor: Option<String>,
    pub source_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyMatchDryRunRequest {
    pub path: String,
    pub operation: FilePolicyOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyMatchDryRunResult {
    pub matched: bool,
    pub decision: FilePolicyDecision,
    pub rule_id: Option<String>,
    pub operation: FilePolicyOperation,
    pub canonical_path: String,
    pub source_revision: u64,
}

pub trait FilePolicyHost: Send + Sync {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError>;

    fn rules_list(
        &self,
        filter: FilePolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<FilePolicyListResult, PluginRuntimeError>;

    fn rules_match_dry_run(
        &self,
        request: FilePolicyMatchDryRunRequest,
    ) -> Result<FilePolicyMatchDryRunResult, PluginRuntimeError>;

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[crate::FilePolicyRulesApplyGrant],
        request: &FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError>;

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[crate::FilePolicyRulesApplyGrant],
        request: FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlSubject {
    FileAccess,
    CommandExecution,
    NetworkAction,
}

impl ControlSubject {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileAccess => "file-access",
            Self::CommandExecution => "command-execution",
            Self::NetworkAction => "network-action",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::FileAccess => control_abi::subject_code::FILE_ACCESS,
            Self::CommandExecution => control_abi::subject_code::COMMAND_EXECUTION,
            Self::NetworkAction => control_abi::subject_code::NETWORK_ACTION,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlVerdict {
    Allow,
    Deny,
}

impl ControlVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionScope {
    Once,
    Reusable,
}

impl DecisionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Reusable => "reusable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlActorProcessIdentity {
    pub pid: u32,
    pub task_id: Option<u32>,
    pub generation: u64,
    pub namespace: Option<String>,
}

impl ControlActorProcessIdentity {
    pub fn summary(&self) -> String {
        format!(
            "pid={} task_id={} generation={} namespace={}",
            self.pid,
            self.task_id
                .map(|task_id| task_id.to_string())
                .unwrap_or_default(),
            self.generation,
            self.namespace.as_deref().unwrap_or_default(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlDecisionRequest {
    pub decision_id: String,
    pub trace_id: String,
    pub subject: ControlSubject,
    pub actor_process_identity: ControlActorProcessIdentity,
    pub operation: String,
    pub target_summary: String,
    pub context_ref: Option<String>,
    pub file_policy_context: Option<FilePolicyReadContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyReadContext {
    pub context_ref: String,
    pub matched_rule: FilePolicyMatchedRule,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyMatchedRule {
    pub rule_id: String,
    pub decision: String,
    pub operation: String,
    pub path: String,
    pub plugin_instance: Option<String>,
    pub timeout_ms: Option<u64>,
    pub concurrency_limit: Option<u32>,
    pub fallback: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlDecisionResponse {
    pub verdict: ControlVerdict,
    pub scope: DecisionScope,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlDecisionBudget {
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCommandRequest {
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginCommandResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PluginCommandBudget {
    pub timeout_ms: Option<u64>,
    pub output_max_bytes: Option<usize>,
}

pub trait ControlDecider: Send + Sync {
    fn instance_id(&self) -> &str;

    fn plugin_id(&self) -> &str;

    fn runtime_kind(&self) -> PluginRuntimeKind;

    fn host_grants(&self) -> Vec<String> {
        Vec::new()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        None
    }

    fn instance_concurrency_limit(&self) -> u32 {
        1
    }

    fn decide(
        &self,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlDecisionResponse, PluginRuntimeError>;

    fn handle_command(
        &self,
        _request: PluginCommandRequest,
        _budget: PluginCommandBudget,
    ) -> Result<PluginCommandResponse, PluginRuntimeError> {
        Err(PluginRuntimeError::new(
            "plugin_command",
            "plugin does not support management commands",
        ))
    }
}
