use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPurpose {
    ObservationConsumer,
    ControlDecider,
    LlmCodec,
}

impl PluginPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObservationConsumer => "observation-consumer",
            Self::ControlDecider => "control-decider",
            Self::LlmCodec => "llm-codec",
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "observation-consumer" => Ok(Self::ObservationConsumer),
            "control-decider" => Ok(Self::ControlDecider),
            "llm-codec" => Ok(Self::LlmCodec),
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginObservationDelivery {
    #[default]
    BestEffort,
    TraceConsistent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPostTraceTrigger {
    TraceTerminal,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapability {
    PayloadRead,
    ContextQuery,
    TraceAnalysisRead,
    TraceFileStateRead,
    AlertWrite,
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
            Self::TraceAnalysisRead => "trace-analysis-read",
            Self::TraceFileStateRead => "trace-file-state-read",
            Self::AlertWrite => "alert-write",
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
