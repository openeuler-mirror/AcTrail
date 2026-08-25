use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{DeliverySeverity, DeliverySource, SandboxDeliverySource, SandboxProcessMarker};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum HandshakeAction {
    #[serde(rename = "handshake")]
    Handshake,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum SubscribeAction {
    #[serde(rename = "subscribe")]
    Subscribe,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum PongAction {
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
enum PingAction {
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
enum SuccessStatus {
    #[serde(rename = "success")]
    Success,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
enum AcceptedStatus {
    #[serde(rename = "accepted")]
    Accepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
enum ErrorStatus {
    #[serde(rename = "error")]
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeAuth {
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeRequest {
    action: HandshakeAction,
    pub version: String,
    pub auth: HandshakeAuth,
    pub client_id: String,
}

impl HandshakeRequest {
    pub fn new(
        version: impl Into<String>,
        token: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            action: HandshakeAction::Handshake,
            version: version.into(),
            auth: HandshakeAuth {
                token: token.into(),
            },
            client_id: client_id.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionFilter {
    #[serde(default)]
    pub severity: Vec<DeliverySeverity>,
    #[serde(default)]
    pub tags: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    pub id: String,
    action: SubscribeAction,
    pub topics: Vec<String>,
    pub filter: SubscriptionFilter,
}

impl SubscribeRequest {
    pub fn new(id: impl Into<String>, topics: Vec<String>, filter: SubscriptionFilter) -> Self {
        Self {
            id: id.into(),
            action: SubscribeAction::Subscribe,
            topics,
            filter,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PongRequest {
    action: PongAction,
    pub nonce: u64,
    pub ts: u64,
}

impl PongRequest {
    pub fn new(nonce: u64, ts: u64) -> Self {
        Self {
            action: PongAction::Pong,
            nonce,
            ts,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SubscriberRequest {
    Handshake(HandshakeRequest),
    Subscribe(SubscribeRequest),
    Pong(PongRequest),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandshakeResponse {
    status: SuccessStatus,
    pub session_id: String,
    pub heartbeat_interval: u64,
}

impl HandshakeResponse {
    pub fn new(session_id: impl Into<String>, heartbeat_interval_seconds: u64) -> Self {
        Self {
            status: SuccessStatus::Success,
            session_id: session_id.into(),
            heartbeat_interval: heartbeat_interval_seconds,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubscribeResponse {
    pub id: String,
    status: AcceptedStatus,
    pub subscribed_topics: Vec<String>,
}

impl SubscribeResponse {
    pub fn new(id: impl Into<String>, subscribed_topics: Vec<String>) -> Self {
        Self {
            id: id.into(),
            status: AcceptedStatus::Accepted,
            subscribed_topics,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PingMessage {
    action: PingAction,
    pub nonce: u64,
    pub ts: u64,
}

impl PingMessage {
    pub const fn new(nonce: u64, ts: u64) -> Self {
        Self {
            action: PingAction::Ping,
            nonce,
            ts,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubscriberErrorResponse {
    status: ErrorStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub code: String,
    pub message: String,
}

impl SubscriberErrorResponse {
    pub fn handshake(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: ErrorStatus::Error,
            id: None,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn request(
        id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: ErrorStatus::Error,
            id: Some(id.into()),
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AlertSource {
    Trace { trid: String },
    Sandbox { sandbox: SandboxAlertSource },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SandboxAlertSource {
    pub gateway_id: u32,
    pub sb_id: u32,
    pub boot_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<SandboxProcessSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SandboxProcessSource {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub executable_name_hex: String,
}

impl From<DeliverySource> for AlertSource {
    fn from(source: DeliverySource) -> Self {
        match source {
            DeliverySource::Trace { trid } => Self::Trace { trid },
            DeliverySource::Sandbox(source) => Self::Sandbox {
                sandbox: SandboxAlertSource::from(source),
            },
        }
    }
}

impl From<SandboxDeliverySource> for SandboxAlertSource {
    fn from(source: SandboxDeliverySource) -> Self {
        Self {
            gateway_id: source.gateway_id,
            sb_id: source.sb_id,
            boot_id: format_boot_id(source.boot_id),
            process: source.process.map(SandboxProcessSource::from),
        }
    }
}

impl From<SandboxProcessMarker> for SandboxProcessSource {
    fn from(process: SandboxProcessMarker) -> Self {
        Self {
            pid: process.pid,
            start_time_ticks: process.start_time_ticks,
            executable_name_hex: format_hex(process.executable_name),
        }
    }
}

fn format_boot_id(bytes: [u8; 16]) -> String {
    let hex = format_hex(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn format_hex<const N: usize>(bytes: [u8; N]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(N * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExternalAlert {
    pub id: String,
    pub ts: u64,
    pub source: AlertSource,
    pub s: DeliverySeverity,
    pub cat: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub labels: Map<String, Value>,
    pub extras: Map<String, Value>,
}
