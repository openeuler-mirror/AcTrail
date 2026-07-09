//! AcTrail plugin-system protocol and runtime boundaries.

mod control;
mod diagnostics;
mod grants;
mod llm_codec;
mod manifest;
mod observation;
mod runtime;
mod status;

pub use control::{
    ControlActorProcessIdentity, ControlDecider, ControlDecisionBudget, ControlDecisionRequest,
    ControlDecisionResponse, ControlSubject, ControlVerdict, DecisionScope, FilePolicyApplyError,
    FilePolicyApplyMode, FilePolicyApplyPrecondition, FilePolicyApplyRequest,
    FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision, FilePolicyHost,
    FilePolicyListFilter, FilePolicyListResult, FilePolicyMatchDryRunRequest,
    FilePolicyMatchDryRunResult, FilePolicyMatchedRule, FilePolicyOperation, FilePolicyPatchItem,
    FilePolicyPatchOp, FilePolicyReadContext, FilePolicyRuleDraft, FilePolicyRuleView,
    PluginCommandBudget, PluginCommandRequest, PluginCommandResponse,
    CONTROL_CURRENT_CONTEXT_TOKEN, CONTROL_DECISION_SUMMARY_QUERY,
    FILE_POLICY_CURRENT_CONTEXT_TOKEN, FILE_POLICY_MATCHED_RULE_QUERY,
};
pub use diagnostics::{PluginDroppedRecord, PluginRuntimeError};
pub use grants::{FilePolicyRulesApplyGrant, PluginHostGrant, PluginHostGrants};
pub use llm_codec::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
pub use manifest::{
    PluginBuiltinDeclaration, PluginCapability, PluginCommandHostcallLimits,
    PluginConfigDeclaration, PluginConfigHostcallLimits, PluginContextHostcallLimits,
    PluginControlDeciderDeclaration, PluginControlDeciderResources, PluginEnvHostcallLimits,
    PluginFilePolicyHostcallLimits, PluginGeneralDeclaration, PluginHostDeclaration,
    PluginHostcallLimits, PluginManifest, PluginManifestPolicy, PluginNativeDylibDeclaration,
    PluginObservationConsumerDeclaration, PluginObservationConsumerResources,
    PluginPayloadHostcallLimits, PluginPurpose, PluginRoleDeclaration, PluginRuntimeDeclaration,
    PluginRuntimeKind, PluginSubscriptionDeclaration, PluginUnusedRuntimeSectionsPolicy,
    PluginWasmAbi, PluginWasmDeclaration, PluginWasmResourceLimits, SUPPORTED_PLUGIN_API_VERSION,
};
pub use observation::{
    ObservationBatch, ObservationConsumeReport, ObservationConsumer, ObservationEventFamily,
    DEFAULT_OBSERVATION_EVENT_FAMILIES, DEFAULT_OBSERVATION_QUEUE_CAPACITY,
};
pub use runtime::{BuiltinPluginInstance, PluginInstanceId};
pub use status::{
    PluginHostcallMetrics, PluginHostcallMetricsSource, PluginInstanceStatus, PluginLifecycleState,
    PluginPayloadReadMetrics,
};
