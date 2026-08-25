use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliverySeverity {
    Info,
    Warning,
    Critical,
}

impl DeliverySeverity {
    pub const fn code(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Critical => 3,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Info),
            2 => Some(Self::Warning),
            3 => Some(Self::Critical),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForwardAlert {
    pub detected_at_ms: u64,
    pub severity: DeliverySeverity,
    pub source: DeliverySource,
    pub category: String,
    pub description: Option<String>,
    pub extras: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliverySource {
    Trace { trid: String },
    Sandbox(SandboxDeliverySource),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxDeliverySource {
    pub gateway_id: u32,
    pub sb_id: u32,
    pub boot_id: [u8; 16],
    pub process: Option<SandboxProcessMarker>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxProcessMarker {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub executable_name: [u8; 16],
}
