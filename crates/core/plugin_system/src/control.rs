use std::sync::Arc;

use actrail_plugin_abi::control as control_abi;

use crate::{PluginHostcallMetricsSource, PluginRuntimeError, PluginRuntimeKind};

pub const CONTROL_CURRENT_CONTEXT_TOKEN: &str = control_abi::context::CURRENT_DECISION;
pub const FILE_POLICY_CURRENT_CONTEXT_TOKEN: &str = control_abi::context::CURRENT_FILE_POLICY;
pub const CONTROL_DECISION_SUMMARY_QUERY: &str = control_abi::query::DECISION_SUMMARY;
pub const FILE_POLICY_MATCHED_RULE_QUERY: &str = control_abi::query::MATCHED_RULE;
pub const FILE_POLICY_WRITE_SCHEMA_VERSION: &str = control_abi::file_policy_write::SCHEMA_VERSION;

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
    pub file_policy_updates: Vec<FilePolicyWriteUpdate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePolicyWriteUpdate {
    pub rule_id: String,
    pub decision: String,
    pub operation: String,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControlDecisionBudget {
    pub timeout_ms: Option<u64>,
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
}
