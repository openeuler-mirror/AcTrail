//! Compact codes for event fields stored on the hot row.

use model_core::event::{EventFlags, EventKind, EventPayload};
use model_core::ids::CollectorName;
use model_core::policy::PolicyVerdict;
use rusqlite::Error as SqlError;

const FLAG_BOOTSTRAP_OBSERVED: i64 = 1 << 0;
const FLAG_METADATA_PARTIAL: i64 = 1 << 1;
const FLAG_POLICY_MODIFIED: i64 = 1 << 2;
const FLAG_PAYLOAD_BLOCKS: i64 = 1 << 3;
const FLAG_POLICY_DETAILS: i64 = 1 << 4;
const KNOWN_FLAGS: i64 = FLAG_BOOTSTRAP_OBSERVED
    | FLAG_METADATA_PARTIAL
    | FLAG_POLICY_MODIFIED
    | FLAG_PAYLOAD_BLOCKS
    | FLAG_POLICY_DETAILS;

const COLLECTOR_MASK: i64 = 0x0f;
const FLAGS_SHIFT: u32 = 4;
const POLICY_SHIFT: u32 = 9;
const POLICY_MASK: i64 = 0x03;

#[derive(Clone, Copy)]
pub struct EventMeta(i64);

impl EventMeta {
    pub fn encode(
        collector: &CollectorName,
        flags: &EventFlags,
        policy: PolicyVerdict,
        has_payload_blocks: bool,
        has_policy_details: bool,
    ) -> Option<Self> {
        let collector_code = match collector.as_str() {
            "ebpf" => 1,
            "process-seccomp" => 2,
            "process-reconcile" => 3,
            "resource-sampler" => 4,
            "application-protocol-analyzer" => 5,
            "network-control" => 6,
            "fanotify-enforcement" => 7,
            "tls-sync" => 8,
            _ => return None,
        };
        let flags_code = encode_event_flags(flags, has_payload_blocks, has_policy_details);
        let policy_code = encode_policy_verdict(policy);
        Some(Self(
            collector_code | (flags_code << FLAGS_SHIFT) | (policy_code << POLICY_SHIFT),
        ))
    }

    pub fn decode(code: i64) -> Result<Self, SqlError> {
        let meta = Self(code);
        meta.collector()?;
        decode_event_flags(meta.flags_code())?;
        decode_policy_verdict(meta.policy_code())?;
        if code & !meta.known_mask() != 0 {
            return Err(SqlError::InvalidQuery);
        }
        Ok(meta)
    }

    pub const fn code(self) -> i64 {
        self.0
    }

    pub fn collector(self) -> Result<CollectorName, SqlError> {
        let name = match self.0 & COLLECTOR_MASK {
            1 => "ebpf",
            2 => "process-seccomp",
            3 => "process-reconcile",
            4 => "resource-sampler",
            5 => "application-protocol-analyzer",
            6 => "network-control",
            7 => "fanotify-enforcement",
            8 => "tls-sync",
            _ => return Err(SqlError::InvalidQuery),
        };
        Ok(CollectorName::new(name))
    }

    pub fn flags(self) -> Result<EventFlags, SqlError> {
        decode_event_flags(self.flags_code())
    }

    pub fn policy(self) -> Result<PolicyVerdict, SqlError> {
        decode_policy_verdict(self.policy_code())
    }

    pub const fn has_payload_blocks(self) -> bool {
        self.flags_code() & FLAG_PAYLOAD_BLOCKS != 0
    }

    pub const fn has_policy_details(self) -> bool {
        self.flags_code() & FLAG_POLICY_DETAILS != 0
    }

    const fn flags_code(self) -> i64 {
        (self.0 >> FLAGS_SHIFT) & KNOWN_FLAGS
    }

    const fn policy_code(self) -> i64 {
        (self.0 >> POLICY_SHIFT) & POLICY_MASK
    }

    const fn known_mask(self) -> i64 {
        COLLECTOR_MASK | (KNOWN_FLAGS << FLAGS_SHIFT) | (POLICY_MASK << POLICY_SHIFT)
    }
}

pub const fn encode_event_kind(value: EventKind) -> i64 {
    match value {
        EventKind::Process => 0,
        EventKind::File => 1,
        EventKind::Net => 2,
        EventKind::Ipc => 3,
        EventKind::Stdio => 4,
        EventKind::Application => 5,
        EventKind::Resource => 6,
        EventKind::Control => 7,
        EventKind::Loss => 8,
        EventKind::Label => 9,
        EventKind::Enforcement => 10,
    }
}

pub fn decode_event_kind(code: i64) -> Result<EventKind, SqlError> {
    match code {
        0 => Ok(EventKind::Process),
        1 => Ok(EventKind::File),
        2 => Ok(EventKind::Net),
        3 => Ok(EventKind::Ipc),
        4 => Ok(EventKind::Stdio),
        5 => Ok(EventKind::Application),
        6 => Ok(EventKind::Resource),
        7 => Ok(EventKind::Control),
        8 => Ok(EventKind::Loss),
        9 => Ok(EventKind::Label),
        10 => Ok(EventKind::Enforcement),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub const fn event_kind_name(value: EventKind) -> &'static str {
    match value {
        EventKind::Process => "process",
        EventKind::File => "file",
        EventKind::Net => "net",
        EventKind::Ipc => "ipc",
        EventKind::Stdio => "stdio",
        EventKind::Application => "application",
        EventKind::Resource => "resource",
        EventKind::Control => "control",
        EventKind::Loss => "loss",
        EventKind::Label => "label",
        EventKind::Enforcement => "enforcement",
    }
}

pub const fn payload_kind(payload: &EventPayload) -> EventKind {
    match payload {
        EventPayload::Process(_) => EventKind::Process,
        EventPayload::File(_) => EventKind::File,
        EventPayload::Net(_) => EventKind::Net,
        EventPayload::Ipc(_) => EventKind::Ipc,
        EventPayload::Stdio(_) => EventKind::Stdio,
        EventPayload::Application(_) => EventKind::Application,
        EventPayload::Resource(_) => EventKind::Resource,
        EventPayload::Control(_) => EventKind::Control,
        EventPayload::Loss(_) => EventKind::Loss,
        EventPayload::Label(_) => EventKind::Label,
        EventPayload::Enforcement(_) => EventKind::Enforcement,
    }
}

const fn encode_event_flags(
    flags: &EventFlags,
    has_payload_blocks: bool,
    has_policy_details: bool,
) -> i64 {
    (if flags.bootstrap_observed {
        FLAG_BOOTSTRAP_OBSERVED
    } else {
        0
    }) | (if flags.metadata_partial {
        FLAG_METADATA_PARTIAL
    } else {
        0
    }) | (if flags.policy_modified {
        FLAG_POLICY_MODIFIED
    } else {
        0
    }) | (if has_payload_blocks {
        FLAG_PAYLOAD_BLOCKS
    } else {
        0
    }) | (if has_policy_details {
        FLAG_POLICY_DETAILS
    } else {
        0
    })
}

fn decode_event_flags(code: i64) -> Result<EventFlags, SqlError> {
    if code & !KNOWN_FLAGS != 0 {
        return Err(SqlError::InvalidQuery);
    }
    Ok(EventFlags {
        bootstrap_observed: code & FLAG_BOOTSTRAP_OBSERVED != 0,
        metadata_partial: code & FLAG_METADATA_PARTIAL != 0,
        policy_modified: code & FLAG_POLICY_MODIFIED != 0,
    })
}

const fn encode_policy_verdict(value: PolicyVerdict) -> i64 {
    match value {
        PolicyVerdict::Allow => 0,
        PolicyVerdict::Redact => 1,
        PolicyVerdict::Drop => 2,
        PolicyVerdict::Fatal => 3,
    }
}

fn decode_policy_verdict(code: i64) -> Result<PolicyVerdict, SqlError> {
    match code {
        0 => Ok(PolicyVerdict::Allow),
        1 => Ok(PolicyVerdict::Redact),
        2 => Ok(PolicyVerdict::Drop),
        3 => Ok(PolicyVerdict::Fatal),
        _ => Err(SqlError::InvalidQuery),
    }
}
