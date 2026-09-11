//! Durable trace-to-resource-boundary relation.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::event::ResourceAccountingMethod;
use crate::ids::{EventId, TraceId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceScopeLifecycleState {
    Active,
    WaitingForEmpty,
    Finalized,
    Orphaned,
}

impl ResourceScopeLifecycleState {
    pub const fn as_storage_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::WaitingForEmpty => "waiting_for_empty",
            Self::Finalized => "finalized",
            Self::Orphaned => "orphaned",
        }
    }

    pub fn from_storage_str(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "waiting_for_empty" => Some(Self::WaitingForEmpty),
            "finalized" => Some(Self::Finalized),
            "orphaned" => Some(Self::Orphaned),
            _ => None,
        }
    }

    pub const fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::WaitingForEmpty)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceResourceScope {
    pub trace_id: TraceId,
    pub nonce: String,
    pub relative_path: PathBuf,
    pub accounting_method: ResourceAccountingMethod,
    pub lifecycle_state: ResourceScopeLifecycleState,
    pub created_at: SystemTime,
    pub final_event_id: Option<EventId>,
    pub updated_at: SystemTime,
}

impl TraceResourceScope {
    pub fn active(
        trace_id: TraceId,
        nonce: impl Into<String>,
        relative_path: impl Into<PathBuf>,
        accounting_method: ResourceAccountingMethod,
        created_at: SystemTime,
    ) -> Self {
        Self {
            trace_id,
            nonce: nonce.into(),
            relative_path: relative_path.into(),
            accounting_method,
            lifecycle_state: ResourceScopeLifecycleState::Active,
            created_at,
            final_event_id: None,
            updated_at: created_at,
        }
    }
}
