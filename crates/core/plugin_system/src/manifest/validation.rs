use std::collections::BTreeSet;

use super::{
    PluginAlertDefinitionDeclaration, PluginCapability, PluginManifest, PluginObservationDelivery,
    PluginPurpose, PluginRuntimeKind, PluginUnusedRuntimeSectionsPolicy, PluginWasmAbi,
    PluginWasmDeclaration,
};
use crate::{ObservationEventFamily, DEFAULT_OBSERVATION_EVENT_FAMILIES};

pub const SUPPORTED_PLUGIN_API_VERSION: &str = "actrail.plugin.v2";

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
        self.validate_plugin_config()?;
        self.validate_alert_outputs()?;
        self.validate_resource_limits()?;
        Ok(self.unused_runtime_section_warnings())
    }

    fn validate_plugin_config(&self) -> Result<(), String> {
        if !self.plugin_config.runtime_managed {
            return Ok(());
        }
        if self.role() != PluginPurpose::ControlDecider
            || self.runtime_kind() != PluginRuntimeKind::Wasm
            || self.selected_wasm().map(|wasm| wasm.abi) != Some(PluginWasmAbi::WitComponent)
        {
            return Err(
                "plugin_config.runtime_managed requires a WIT control-decider component"
                    .to_string(),
            );
        }
        if self.plugin_config.schema_ref.is_none() {
            return Err("plugin_config.runtime_managed requires schema_ref".to_string());
        }
        if self.plugin_config.format != "json" {
            return Err("plugin_config.runtime_managed requires format = \"json\"".to_string());
        }
        if self.control_decision_concurrency_limit() != Some(1) {
            return Err(
                "plugin_config.runtime_managed requires control-decider concurrency_limit = 1"
                    .to_string(),
            );
        }
        Ok(())
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
        (self.runtime_kind() == PluginRuntimeKind::Wasm)
            .then_some(self.runtime.wasm.as_ref())
            .flatten()
    }

    pub fn selected_wasm_mut(&mut self) -> Option<&mut PluginWasmDeclaration> {
        (self.runtime_kind() == PluginRuntimeKind::Wasm)
            .then_some(self.runtime.wasm.as_mut())
            .flatten()
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

    pub fn has_post_trace_analyzer(&self) -> bool {
        self.role
            .observation_consumer
            .as_ref()
            .and_then(|role| role.post_trace.as_ref())
            .is_some()
    }

    pub fn alert_outputs(&self) -> impl Iterator<Item = (&str, &PluginAlertDefinitionDeclaration)> {
        self.outputs
            .alerts
            .iter()
            .map(|(key, definition)| (key.as_str(), definition))
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
        if self.manifest_policy.unused_runtime_sections == PluginUnusedRuntimeSectionsPolicy::Deny
            && !self.unused_runtime_sections().is_empty()
        {
            return Err(format!(
                "manifest declares unused runtime sections for general.runtime = {}: {}",
                self.runtime_kind().as_str(),
                self.unused_runtime_sections().join(", ")
            ));
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
                    let event_families = role.subscriptions.event_families.as_ref();
                    if event_families.is_some_and(Vec::is_empty) {
                        return Err(
                            "role.observation-consumer.subscriptions.event_families must not be empty"
                                .to_string(),
                        );
                    }
                    if let Some(event_families) = event_families {
                        let unique = event_families.iter().collect::<BTreeSet<_>>();
                        if unique.len() != event_families.len() {
                            return Err(
                                "role.observation-consumer.subscriptions.event_families must not contain duplicates"
                                    .to_string(),
                            );
                        }
                    }
                    if role.post_trace.is_some() {
                        if role.delivery != PluginObservationDelivery::TraceConsistent {
                            return Err(
                                "role.observation-consumer.post-trace requires delivery = \"trace-consistent\""
                                    .to_string(),
                            );
                        }
                        if self.runtime_kind() != PluginRuntimeKind::Wasm
                            || self.selected_wasm().map(|wasm| wasm.abi)
                                != Some(PluginWasmAbi::WitComponent)
                        {
                            return Err(
                                "role.observation-consumer.post-trace requires a WIT component runtime"
                                    .to_string(),
                            );
                        }
                        let families = self.observation_event_families();
                        for required in [
                            ObservationEventFamily::SemanticAction,
                            ObservationEventFamily::TraceLifecycle,
                        ] {
                            if !families.contains(&required) {
                                return Err(format!(
                                    "role.observation-consumer.post-trace requires {} subscription",
                                    observation_family_name(required)
                                ));
                            }
                        }
                    }
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
            PluginPurpose::LlmCodec => {
                if self.role.observation_consumer.is_some() {
                    return Err(
                        "role.observation-consumer is unused when general.role = \"llm-codec\""
                            .to_string(),
                    );
                }
                if self.role.control_decider.is_some() {
                    return Err(
                        "role.control-decider is unused when general.role = \"llm-codec\""
                            .to_string(),
                    );
                }
                if self.runtime_kind() != PluginRuntimeKind::Wasm {
                    return Err(
                        "general.role = \"llm-codec\" requires general.runtime = \"wasm\""
                            .to_string(),
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), String> {
        let unique = self.host.capabilities.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.host.capabilities.len() {
            return Err("host.capabilities must not contain duplicates".to_string());
        }
        Ok(())
    }

    fn validate_alert_outputs(&self) -> Result<(), String> {
        if self.outputs.alerts.is_empty() {
            return Ok(());
        }
        if !self.capabilities().contains(&PluginCapability::AlertWrite) {
            return Err("outputs.alerts requires alert-write capability".to_string());
        }
        if self.role() != PluginPurpose::ObservationConsumer
            || self.runtime_kind() != PluginRuntimeKind::Wasm
            || self.selected_wasm().map(|wasm| wasm.abi) != Some(PluginWasmAbi::WitComponent)
        {
            return Err("outputs.alerts requires a WIT observation component runtime".to_string());
        }
        for (key, definition) in &self.outputs.alerts {
            validate_non_empty(key, "outputs.alerts definition key")?;
            validate_non_empty(&definition.kind, &format!("outputs.alerts.{key}.kind"))?;
            validate_non_empty(&definition.title, &format!("outputs.alerts.{key}.title"))?;
            validate_non_empty(
                &definition.payload_schema_id,
                &format!("outputs.alerts.{key}.payload_schema_id"),
            )?;
            validate_non_empty(
                &definition.payload_schema_ref,
                &format!("outputs.alerts.{key}.payload_schema_ref"),
            )?;
        }
        Ok(())
    }

    fn validate_resource_limits(&self) -> Result<(), String> {
        let wasm = self.runtime.wasm.as_ref();
        validate_positive_u64(
            wasm.and_then(|runtime| runtime.resources.fuel_per_call),
            "runtime.wasm.resources.fuel_per_call",
        )?;
        validate_positive_u64(
            wasm.and_then(|runtime| runtime.resources.memory_max_bytes),
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
        for (value, field) in [
            (
                self.hostcall_limits.env.name_max_bytes,
                "hostcall_limits.env.name_max_bytes",
            ),
            (
                self.hostcall_limits.env.value_max_bytes,
                "hostcall_limits.env.value_max_bytes",
            ),
            (
                self.hostcall_limits.payload.segment_max_count,
                "hostcall_limits.payload.segment_max_count",
            ),
            (
                self.hostcall_limits.payload.ref_max_bytes,
                "hostcall_limits.payload.ref_max_bytes",
            ),
            (
                self.hostcall_limits.payload.read_max_bytes,
                "hostcall_limits.payload.read_max_bytes",
            ),
            (
                self.hostcall_limits.context.ref_max_bytes,
                "hostcall_limits.context.ref_max_bytes",
            ),
            (
                self.hostcall_limits.context.query_max_bytes,
                "hostcall_limits.context.query_max_bytes",
            ),
            (
                self.hostcall_limits.context.read_max_bytes,
                "hostcall_limits.context.read_max_bytes",
            ),
            (
                self.hostcall_limits.file_policy.context_ref_max_bytes,
                "hostcall_limits.file_policy.context_ref_max_bytes",
            ),
            (
                self.hostcall_limits.file_policy.query_max_bytes,
                "hostcall_limits.file_policy.query_max_bytes",
            ),
            (
                self.hostcall_limits.file_policy.read_max_bytes,
                "hostcall_limits.file_policy.read_max_bytes",
            ),
            (
                self.hostcall_limits.plugin_config.read_max_bytes,
                "hostcall_limits.plugin_config.read_max_bytes",
            ),
            (
                self.hostcall_limits.plugin_command.argv_max_count,
                "hostcall_limits.plugin_command.argv_max_count",
            ),
            (
                self.hostcall_limits.plugin_command.arg_max_bytes,
                "hostcall_limits.plugin_command.arg_max_bytes",
            ),
            (
                self.hostcall_limits.plugin_command.output_max_bytes,
                "hostcall_limits.plugin_command.output_max_bytes",
            ),
            (
                self.hostcall_limits.trace_analysis.action_page_max_count,
                "hostcall_limits.trace_analysis.action_page_max_count",
            ),
            (
                self.hostcall_limits.trace_analysis.action_total_max_count,
                "hostcall_limits.trace_analysis.action_total_max_count",
            ),
            (
                self.hostcall_limits.trace_file_state.query_max_count,
                "hostcall_limits.trace_file_state.query_max_count",
            ),
            (
                self.hostcall_limits.alert.payload_max_bytes,
                "hostcall_limits.alert.payload_max_bytes",
            ),
        ] {
            validate_positive_u32(value, field)?;
        }
        validate_positive_u64(
            self.hostcall_limits.plugin_command.timeout_ms,
            "hostcall_limits.plugin_command.timeout_ms",
        )?;
        validate_positive_u64(
            self.hostcall_limits.trace_file_state.timeout_ms,
            "hostcall_limits.trace_file_state.timeout_ms",
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

fn validate_non_empty(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(())
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

fn observation_family_name(family: ObservationEventFamily) -> &'static str {
    match family {
        ObservationEventFamily::SemanticAction => "semantic-action",
        ObservationEventFamily::SemanticActionLink => "semantic-action-link",
        ObservationEventFamily::Diagnostic => "diagnostic",
        ObservationEventFamily::TraceLifecycle => "trace-lifecycle",
        ObservationEventFamily::ResourceMetric => "resource-metric",
        ObservationEventFamily::PayloadMetadata => "payload-metadata",
    }
}
