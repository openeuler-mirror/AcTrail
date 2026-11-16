//! Shared plugin purpose and runtime identifiers.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPurpose {
    ObservationConsumer,
    SandboxObservationConsumer,
    AlertConsumer,
    ControlDecider,
    LlmCodec,
}

impl PluginPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObservationConsumer => "observation-consumer",
            Self::SandboxObservationConsumer => "sandbox-observation-consumer",
            Self::AlertConsumer => "alert-consumer",
            Self::ControlDecider => "control-decider",
            Self::LlmCodec => "llm-codec",
        }
    }

    pub fn from_wire(value: &str) -> Result<Self, String> {
        match value {
            "observation-consumer" => Ok(Self::ObservationConsumer),
            "sandbox-observation-consumer" => Ok(Self::SandboxObservationConsumer),
            "alert-consumer" => Ok(Self::AlertConsumer),
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
