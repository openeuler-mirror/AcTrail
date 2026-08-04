use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment};
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticEvidence};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct McpStdioSessionKey {
    pub(super) trace_id: TraceId,
    pub(super) stdin_channel_id: Arc<str>,
    pub(super) stdout_channel_id: Arc<str>,
}

impl McpStdioSessionKey {
    pub(super) fn action_component(&self) -> String {
        let stdin_len = u64::try_from(self.stdin_channel_id.len())
            .expect("MCP stdin channel identity length must fit u64");
        let stdout_len = u64::try_from(self.stdout_channel_id.len())
            .expect("MCP stdout channel identity length must fit u64");
        let mut hasher = Sha256::new();
        hasher.update(b"actrail:mcp-stdio-connection:v1\0");
        hasher.update(stdin_len.to_be_bytes());
        hasher.update(self.stdin_channel_id.as_bytes());
        hasher.update(stdout_len.to_be_bytes());
        hasher.update(self.stdout_channel_id.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpStdioStream {
    Stdin,
    Stdout,
    Stderr,
    Unknown,
}

impl McpStdioStream {
    pub(super) fn from_segment(segment: &PayloadSegment) -> Self {
        match segment.protocol_hint.as_deref() {
            Some("stdin") => return Self::Stdin,
            Some("stdout") => return Self::Stdout,
            Some("stderr") => return Self::Stderr,
            _ => {}
        }
        let stream_key = segment.stream_key.as_str();
        if stream_key.ends_with(":stdin") || stream_key.ends_with("_stdin") {
            Self::Stdin
        } else if stream_key.ends_with(":stdout") || stream_key.ends_with("_stdout") {
            Self::Stdout
        } else if stream_key.ends_with(":stderr") || stream_key.ends_with("_stderr") {
            Self::Stderr
        } else {
            Self::Unknown
        }
    }

    pub(super) const fn expected_payload_direction(self) -> Option<PayloadDirection> {
        match self {
            Self::Stdin => Some(PayloadDirection::Inbound),
            Self::Stdout => Some(PayloadDirection::Outbound),
            Self::Stderr | Self::Unknown => None,
        }
    }

    pub(super) const fn message_direction(self) -> Option<McpMessageDirection> {
        match self {
            Self::Stdin => Some(McpMessageDirection::ClientToServer),
            Self::Stdout => Some(McpMessageDirection::ServerToClient),
            Self::Stderr | Self::Unknown => None,
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Stdin => "stdin",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpMessageDirection {
    ClientToServer,
    ServerToClient,
}

impl McpMessageDirection {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::ClientToServer => "outbound",
            Self::ServerToClient => "inbound",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum McpJsonRpcId {
    String(String),
    Number(String),
}

impl McpJsonRpcId {
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        match value.get("id")? {
            Value::String(id) => Some(Self::String(id.clone())),
            Value::Number(id) => Some(Self::Number(id.to_string())),
            _ => None,
        }
    }

    pub(super) fn as_attribute(&self) -> &str {
        match self {
            Self::String(value) | Self::Number(value) => value,
        }
    }

    pub(super) fn action_component(&self) -> String {
        match self {
            Self::String(value) => format!("string:{value}"),
            Self::Number(value) => format!("number:{value}"),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct McpJsonRpcMessage {
    pub(super) value: Value,
    pub(super) observed_at: SystemTime,
    pub(super) evidence: Arc<[SemanticEvidence]>,
}

impl McpJsonRpcMessage {
    pub(super) fn split_complete_value(
        value: Value,
        observed_at: SystemTime,
        evidence: Vec<SemanticEvidence>,
    ) -> Vec<Self> {
        let evidence = Arc::<[SemanticEvidence]>::from(evidence);
        let values = match value {
            Value::Array(values) => values,
            value => vec![value],
        };
        values
            .into_iter()
            .filter(|value| {
                value
                    .get("jsonrpc")
                    .and_then(Value::as_str)
                    .is_some_and(|version| version == "2.0")
            })
            .map(|value| Self {
                value,
                observed_at,
                evidence: evidence.clone(),
            })
            .collect()
    }

    pub(super) fn method(&self) -> Option<&str> {
        self.value.get("method")?.as_str()
    }

    pub(super) fn id(&self) -> Option<McpJsonRpcId> {
        McpJsonRpcId::from_value(&self.value)
    }

    pub(super) fn tools_call_name(&self) -> Option<&str> {
        let params = self.value.get("params")?.as_object()?;
        let name = params
            .get("name")?
            .as_str()
            .filter(|name| !name.is_empty())?;
        match params.get("arguments") {
            None => Some(name),
            Some(arguments) if arguments.is_object() => Some(name),
            Some(_) => None,
        }
    }

    pub(super) fn is_tools_call_admission(&self, direction: McpMessageDirection) -> bool {
        direction == McpMessageDirection::ClientToServer
            && self.value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
            && self.method() == Some("tools/call")
            && self.id().is_some()
            && self.tools_call_name().is_some()
    }

    pub(super) fn response_status(&self) -> Option<semantic_action::SemanticActionStatus> {
        use semantic_action::SemanticActionStatus;

        if self.method().is_some() || self.id().is_none() {
            return None;
        }
        if self.value.get("error").is_some() {
            return Some(SemanticActionStatus::Error);
        }
        let result = self.value.get("result")?;
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Some(SemanticActionStatus::Error)
        } else {
            Some(SemanticActionStatus::Success)
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct McpBufferedStdioMessage {
    pub(super) direction: McpMessageDirection,
    pub(super) server_process: ProcessIdentity,
    pub(super) stream_key: String,
    pub(super) message: McpJsonRpcMessage,
}

impl McpBufferedStdioMessage {
    pub(super) fn is_tools_call_admission(&self) -> bool {
        self.message.is_tools_call_admission(self.direction)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct McpResponseKey {
    pub(super) session: McpStdioSessionKey,
    pub(super) request_id: McpJsonRpcId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct McpRequestKey {
    pub(super) response: McpResponseKey,
    pub(super) invocation_sequence: u64,
}

#[derive(Clone, Debug)]
pub(super) struct McpOpenCall {
    pub(super) action: SemanticAction,
}

#[derive(Clone, Debug, Default)]
pub(super) struct McpCorrelationState {
    pub(super) invocation_sequences: BTreeMap<McpResponseKey, u64>,
    pub(super) open_calls: BTreeMap<McpRequestKey, McpOpenCall>,
    pub(super) open_by_response: BTreeMap<McpResponseKey, VecDeque<McpRequestKey>>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct McpServerState {
    pub(super) name: Option<String>,
    pub(super) pending_initialize_id: Option<McpJsonRpcId>,
}
