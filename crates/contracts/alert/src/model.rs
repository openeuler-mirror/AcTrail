//! Normalized alert definition and occurrence records.

use std::fmt;
use std::num::NonZeroUsize;
use std::time::SystemTime;

use model_core::ids::TraceId;
use serde::{Deserialize, Serialize};

macro_rules! define_alert_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}-{}", $label, self.0)
            }
        }
    };
}

define_alert_id!(AlertId, "alert");
define_alert_id!(AlertDefinitionId, "alert-definition");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertSubmitOutcome {
    Stored(AlertId),
    RejectedTraceToken,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlertSeverity {
    Informational,
    Low,
    Medium,
    High,
    Critical,
}

impl AlertSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Informational => "informational",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "informational" => Some(Self::Informational),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertDefinition {
    pub producer_plugin_id: String,
    pub definition_key: String,
    pub kind: String,
    pub title: String,
    pub severity: AlertSeverity,
    pub payload_schema_id: String,
}

impl AlertDefinition {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.producer_plugin_id.trim().is_empty() {
            return Err("producer_plugin_id must not be empty");
        }
        if self.definition_key.trim().is_empty() {
            return Err("definition_key must not be empty");
        }
        if self.kind.trim().is_empty() {
            return Err("kind must not be empty");
        }
        if self.title.trim().is_empty() {
            return Err("title must not be empty");
        }
        if self.payload_schema_id.trim().is_empty() {
            return Err("payload_schema_id must not be empty");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertDraft {
    pub definition_key: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertRecord {
    pub alert_id: AlertId,
    pub trace_id: TraceId,
    pub alert_definition_id: AlertDefinitionId,
    pub created_at: SystemTime,
    pub payload_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertView {
    pub record: AlertRecord,
    pub definition: AlertDefinition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlertListLimit(NonZeroUsize);

impl AlertListLimit {
    pub fn new(raw: usize) -> Option<Self> {
        NonZeroUsize::new(raw).map(Self)
    }

    pub const fn get(self) -> usize {
        self.0.get()
    }
}
