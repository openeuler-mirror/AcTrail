use serde::{Deserialize, Serialize};

use crate::{ObservationEventFamily, DEFAULT_OBSERVATION_EVENT_FAMILIES};

pub const SUPPORTED_PLUGIN_API_VERSION: &str = "actrail.plugin.v1";

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
    pub plugin_config: PluginConfigDeclaration,
    #[serde(default)]
    pub manifest_policy: PluginManifestPolicy,
}

impl PluginManifest {
    pub fn validate_loadable(&self) -> Result<Vec<String>, String> {
        if self.general.id.trim().is_empty() {
            return Err("plugin manifest general.id must not be empty".to_string());
        }
        if self.general.api_version != SUPPORTED_PLUGIN_API_VERSION {
            return Err(format!(
                "plugin manifest general.api_version {} is unsupported; expected {}",
                self.general.api_version, SUPPORTED_PLUGIN_API_VERSION
            ));
        }
        self.validate_capabilities()?;
        self.validate_role_sections()?;
        self.validate_runtime_selection()?;
        self.validate_resource_limits()?;
        Ok(self.unused_runtime_section_warnings())
    }

    pub fn id(&self) -> &str {
        &self.general.id
    }

    pub fn role(&self) -> PluginPurpose {
        self.general.role
    }

    pub fn runtime_kind(&self) -> PluginRuntimeKind {
        self.general.runtime
    }

    pub fn capabilities(&self) -> &[PluginCapability] {
        &self.host.capabilities
    }

    pub fn selected_wasm(&self) -> Option<&PluginWasmDeclaration> {
        if self.runtime_kind() == PluginRuntimeKind::Wasm {
            self.runtime.wasm.as_ref()
        } else {
            None
        }
    }

    pub fn selected_wasm_mut(&mut self) -> Option<&mut PluginWasmDeclaration> {
        if self.runtime_kind() == PluginRuntimeKind::Wasm {
            self.runtime.wasm.as_mut()
        } else {
            None
        }
    }

    pub fn observation_event_families(&self) -> Vec<ObservationEventFamily> {
        self.role
            .observation_consumer
            .as_ref()
            .and_then(|role| role.subscriptions.event_families.clone())
            .unwrap_or_else(|| DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec())
    }

    pub fn observation_queue_capacity(&self) -> Option<u32> {
        self.role
            .observation_consumer
            .as_ref()
            .and_then(|role| role.resources.queue_capacity)
    }

    pub fn control_decision_concurrency_limit(&self) -> Option<u32> {
        self.role
            .control_decider
            .as_ref()
            .and_then(|role| role.resources.concurrency_limit)
    }

    fn validate_runtime_selection(&self) -> Result<(), String> {
        match self.runtime_kind() {
            PluginRuntimeKind::Wasm => {
                let Some(wasm) = self.runtime.wasm.as_ref() else {
                    return Err("general.runtime = \"wasm\" requires [runtime.wasm]".to_string());
                };
                if !wasm
                    .artifact_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty())
                {
                    return Err(
                        "general.runtime = \"wasm\" requires non-empty runtime.wasm.artifact_path"
                            .to_string(),
                    );
                }
            }
            PluginRuntimeKind::Builtin => {}
            PluginRuntimeKind::NativeDylib => {
                if self.runtime.native_dylib.is_none() {
                    return Err(
                        "general.runtime = \"native-dylib\" requires [runtime.native-dylib]"
                            .to_string(),
                    );
                }
            }
        }
        match self.manifest_policy.unused_runtime_sections {
            PluginUnusedRuntimeSectionsPolicy::Deny
                if !self.unused_runtime_sections().is_empty() =>
            {
                return Err(format!(
                    "manifest declares unused runtime sections for general.runtime = {}: {}",
                    self.runtime_kind().as_str(),
                    self.unused_runtime_sections().join(", ")
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_role_sections(&self) -> Result<(), String> {
        match self.role() {
            PluginPurpose::ObservationConsumer => {
                if self.role.control_decider.is_some() {
                    return Err(
                        "role.control-decider is unused when general.role = \"observation-consumer\""
                            .to_string(),
                    );
                }
                if let Some(role) = self.role.observation_consumer.as_ref() {
                    role.validate()?;
                }
            }
            PluginPurpose::ControlDecider => {
                if self.role.observation_consumer.is_some() {
                    return Err(
                        "role.observation-consumer is unused when general.role = \"control-decider\""
                            .to_string(),
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), String> {
        let unique = self
            .host
            .capabilities
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != self.host.capabilities.len() {
            return Err("host.capabilities must not contain duplicates".to_string());
        }
        Ok(())
    }

    fn validate_resource_limits(&self) -> Result<(), String> {
        validate_positive_u64(
            self.runtime
                .wasm
                .as_ref()
                .and_then(|wasm| wasm.resources.fuel_per_call),
            "runtime.wasm.resources.fuel_per_call",
        )?;
        validate_positive_u64(
            self.runtime
                .wasm
                .as_ref()
                .and_then(|wasm| wasm.resources.memory_max_bytes),
            "runtime.wasm.resources.memory_max_bytes",
        )?;
        validate_positive_u32(
            self.control_decision_concurrency_limit(),
            "role.control-decider.resources.concurrency_limit",
        )?;
        validate_positive_u32(
            self.observation_queue_capacity(),
            "role.observation-consumer.resources.queue_capacity",
        )?;
        validate_positive_u32(
            self.hostcall_limits.env.name_max_bytes,
            "hostcall_limits.env.name_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.env.value_max_bytes,
            "hostcall_limits.env.value_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.payload.segment_max_count,
            "hostcall_limits.payload.segment_max_count",
        )?;
        validate_positive_u32(
            self.hostcall_limits.payload.ref_max_bytes,
            "hostcall_limits.payload.ref_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.payload.read_max_bytes,
            "hostcall_limits.payload.read_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.context.ref_max_bytes,
            "hostcall_limits.context.ref_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.context.query_max_bytes,
            "hostcall_limits.context.query_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.context.read_max_bytes,
            "hostcall_limits.context.read_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.file_policy.context_ref_max_bytes,
            "hostcall_limits.file_policy.context_ref_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.file_policy.query_max_bytes,
            "hostcall_limits.file_policy.query_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.file_policy.read_max_bytes,
            "hostcall_limits.file_policy.read_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.plugin_config.read_max_bytes,
            "hostcall_limits.plugin_config.read_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.plugin_command.argv_max_count,
            "hostcall_limits.plugin_command.argv_max_count",
        )?;
        validate_positive_u32(
            self.hostcall_limits.plugin_command.arg_max_bytes,
            "hostcall_limits.plugin_command.arg_max_bytes",
        )?;
        validate_positive_u32(
            self.hostcall_limits.plugin_command.output_max_bytes,
            "hostcall_limits.plugin_command.output_max_bytes",
        )?;
        validate_positive_u64(
            self.hostcall_limits.plugin_command.timeout_ms,
            "hostcall_limits.plugin_command.timeout_ms",
        )?;
        Ok(())
    }

    fn unused_runtime_sections(&self) -> Vec<&'static str> {
        let selected = self.runtime_kind();
        let mut sections = Vec::new();
        if self.runtime.wasm.is_some() && selected != PluginRuntimeKind::Wasm {
            sections.push("runtime.wasm");
        }
        if self.runtime.builtin.is_some() && selected != PluginRuntimeKind::Builtin {
            sections.push("runtime.builtin");
        }
        if self.runtime.native_dylib.is_some() && selected != PluginRuntimeKind::NativeDylib {
            sections.push("runtime.native-dylib");
        }
        sections
    }

    fn unused_runtime_section_warnings(&self) -> Vec<String> {
        if self.manifest_policy.unused_runtime_sections != PluginUnusedRuntimeSectionsPolicy::Warn {
            return Vec::new();
        }
        self.unused_runtime_sections()
            .into_iter()
            .map(|section| {
                format!(
                    "{section} ignored because general.runtime = {}",
                    self.runtime_kind().as_str()
                )
            })
            .collect()
    }
}

fn validate_positive_u32(value: Option<u32>, field: &str) -> Result<(), String> {
    if value.is_some_and(|value| value == 0) {
        return Err(format!("{field} must be greater than zero"));
    }
    Ok(())
}

fn validate_positive_u64(value: Option<u64>, field: &str) -> Result<(), String> {
    if value.is_some_and(|value| value == 0) {
        return Err(format!("{field} must be greater than zero"));
    }
    Ok(())
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
    pub subscriptions: PluginSubscriptionDeclaration,
    #[serde(default)]
    pub resources: PluginObservationConsumerResources,
}

impl PluginObservationConsumerDeclaration {
    fn validate(&self) -> Result<(), String> {
        let Some(event_families) = self.subscriptions.event_families.as_ref() else {
            return Ok(());
        };
        if event_families.is_empty() {
            return Err(
                "role.observation-consumer.subscriptions.event_families must not be empty"
                    .to_string(),
            );
        }
        let unique = event_families
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != event_families.len() {
            return Err(
                "role.observation-consumer.subscriptions.event_families must not contain duplicates"
                    .to_string(),
            );
        }
        Ok(())
    }
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
pub struct PluginHostcallLimits {
    #[serde(default)]
    pub env: PluginEnvHostcallLimits,
    #[serde(default)]
    pub payload: PluginPayloadHostcallLimits,
    #[serde(default)]
    pub context: PluginContextHostcallLimits,
    #[serde(default)]
    pub file_policy: PluginFilePolicyHostcallLimits,
    #[serde(default)]
    pub plugin_config: PluginConfigHostcallLimits,
    #[serde(default)]
    pub plugin_command: PluginCommandHostcallLimits,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEnvHostcallLimits {
    pub name_max_bytes: Option<u32>,
    pub value_max_bytes: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPayloadHostcallLimits {
    pub segment_max_count: Option<u32>,
    pub ref_max_bytes: Option<u32>,
    pub read_max_bytes: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginContextHostcallLimits {
    pub ref_max_bytes: Option<u32>,
    pub query_max_bytes: Option<u32>,
    pub read_max_bytes: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginFilePolicyHostcallLimits {
    pub context_ref_max_bytes: Option<u32>,
    pub query_max_bytes: Option<u32>,
    pub read_max_bytes: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCommandHostcallLimits {
    pub argv_max_count: Option<u32>,
    pub arg_max_bytes: Option<u32>,
    pub output_max_bytes: Option<u32>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigHostcallLimits {
    pub read_max_bytes: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSubscriptionDeclaration {
    #[serde(default)]
    pub event_families: Option<Vec<ObservationEventFamily>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPurpose {
    ObservationConsumer,
    ControlDecider,
}

impl PluginPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObservationConsumer => "observation-consumer",
            Self::ControlDecider => "control-decider",
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "observation-consumer" => Ok(Self::ObservationConsumer),
            "control-decider" => Ok(Self::ControlDecider),
            _ => Err(format!("unknown plugin role {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRuntimeKind {
    Builtin,
    Wasm,
    NativeDylib,
}

impl PluginRuntimeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Wasm => "wasm",
            Self::NativeDylib => "native-dylib",
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "builtin" => Ok(Self::Builtin),
            "wasm" => Ok(Self::Wasm),
            "native-dylib" => Ok(Self::NativeDylib),
            _ => Err(format!("unknown plugin runtime {value}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigDeclaration {
    pub format: String,
    pub schema_ref: Option<String>,
    pub required: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapability {
    PayloadRead,
    ContextQuery,
    #[serde(rename = "file-access.current-match-get")]
    FileAccessCurrentMatchGet,
    #[serde(rename = "file-access.current-context-query")]
    FileAccessCurrentContextQuery,
    #[serde(rename = "file-policy.rules.read")]
    FilePolicyRulesRead,
    #[serde(rename = "file-policy.rules.match-dry-run")]
    FilePolicyRulesMatchDryRun,
    #[serde(rename = "file-policy.rules.validate")]
    FilePolicyRulesValidate,
    #[serde(rename = "file-policy.rules.apply")]
    FilePolicyRulesApply,
    NetworkEgress,
    EnvRead,
}

impl PluginCapability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PayloadRead => "payload-read",
            Self::ContextQuery => "context-query",
            Self::FileAccessCurrentMatchGet => "file-access.current-match-get",
            Self::FileAccessCurrentContextQuery => "file-access.current-context-query",
            Self::FilePolicyRulesRead => "file-policy.rules.read",
            Self::FilePolicyRulesMatchDryRun => "file-policy.rules.match-dry-run",
            Self::FilePolicyRulesValidate => "file-policy.rules.validate",
            Self::FilePolicyRulesApply => "file-policy.rules.apply",
            Self::NetworkEgress => "network-egress",
            Self::EnvRead => "env-read",
        }
    }
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
