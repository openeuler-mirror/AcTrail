use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub trace_analysis: PluginTraceAnalysisHostcallLimits,
    #[serde(default)]
    pub trace_file_state: PluginTraceFileStateHostcallLimits,
    #[serde(default)]
    pub alert: PluginAlertHostcallLimits,
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
pub struct PluginTraceAnalysisHostcallLimits {
    pub action_page_max_count: Option<u32>,
    pub action_total_max_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTraceFileStateHostcallLimits {
    pub query_max_count: Option<u32>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAlertHostcallLimits {
    pub payload_max_bytes: Option<u32>,
}
