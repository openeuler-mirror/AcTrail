//! Payload admission gates before durable persistence.

mod body_retention;
mod socket;

pub(in crate::services) use body_retention::{
    PayloadBodyRetention, PayloadBodyRetentionDecision, PayloadBodyRetentionGate,
};
pub(in crate::services) use socket::{
    SocketHttpPayloadGate, socket_payload_prefix_is_http_candidate,
};
