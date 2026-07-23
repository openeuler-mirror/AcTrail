use std::collections::BTreeMap;

use alert_contract::AlertSeverity;
use serde::{Deserialize, Serialize};

use super::{
    PluginCapability, PluginHostcallLimits, PluginObservationDelivery, PluginPostTraceTrigger,
    PluginPurpose, PluginRuntimeKind,
};
use crate::ObservationEventFamily;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub general: PluginGeneralDeclaration,
    #[serde(default)]
    pub host: PluginHostDeclaration,
    #[serde(default)]
    pub runtime: PluginRuntimeDeclaration,
    #[serde(default)]
    pub role: PluginRoleDeclaration,
    #[serde(default)]
    pub hostcall_limits: PluginHostcallLimits,
    #[serde(default)]
    pub outputs: PluginOutputsDeclaration,
    pub plugin_config: PluginConfigDeclaration,
    #[serde(default)]
    pub manifest_policy: PluginManifestPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginGeneralDeclaration {
    pub id: String,
    pub api_version: String,
    pub role: PluginPurpose,
    pub runtime: PluginRuntimeKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHostDeclaration {
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeDeclaration {
    #[serde(default)]
    pub wasm: Option<PluginWasmDeclaration>,
    #[serde(default)]
    pub builtin: Option<PluginBuiltinDeclaration>,
    #[serde(default, rename = "native-dylib")]
    pub native_dylib: Option<PluginNativeDylibDeclaration>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginBuiltinDeclaration {}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginNativeDylibDeclaration {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginWasmDeclaration {
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub abi: PluginWasmAbi,
    #[serde(default)]
    pub resources: PluginWasmResourceLimits,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginWasmResourceLimits {
    pub fuel_per_call: Option<u64>,
    pub memory_max_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginWasmAbi {
    #[default]
    LegacyModule,
    WitComponent,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRoleDeclaration {
    #[serde(default, rename = "observation-consumer")]
    pub observation_consumer: Option<PluginObservationConsumerDeclaration>,
    #[serde(default, rename = "control-decider")]
    pub control_decider: Option<PluginControlDeciderDeclaration>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginObservationConsumerDeclaration {
    #[serde(default)]
    pub delivery: PluginObservationDelivery,
    #[serde(default)]
    pub subscriptions: PluginSubscriptionDeclaration,
    #[serde(default)]
    pub resources: PluginObservationConsumerResources,
    #[serde(default, rename = "post-trace")]
    pub post_trace: Option<PluginPostTraceDeclaration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPostTraceDeclaration {
    pub trigger: PluginPostTraceTrigger,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginObservationConsumerResources {
    pub queue_capacity: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginControlDeciderDeclaration {
    #[serde(default)]
    pub resources: PluginControlDeciderResources,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginControlDeciderResources {
    pub concurrency_limit: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSubscriptionDeclaration {
    #[serde(default)]
    pub event_families: Option<Vec<ObservationEventFamily>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginOutputsDeclaration {
    #[serde(default)]
    pub alerts: BTreeMap<String, PluginAlertDefinitionDeclaration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAlertDefinitionDeclaration {
    pub kind: String,
    pub title: String,
    pub severity: AlertSeverity,
    pub payload_schema_id: String,
    pub payload_schema_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigDeclaration {
    pub format: String,
    pub schema_ref: Option<String>,
    pub required: bool,
    #[serde(default)]
    pub runtime_managed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestPolicy {
    #[serde(default)]
    pub unused_runtime_sections: PluginUnusedRuntimeSectionsPolicy,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginUnusedRuntimeSectionsPolicy {
    #[default]
    Deny,
    Warn,
    Ignore,
}
