//! Rustls internal plaintext points emit immediately and have no return probe.

use super::points::{AttachPoint, WirePoint};
use crate::loader::LoaderError;

pub(super) struct RustlsPlan;

impl RustlsPlan {
    pub(super) fn parse(value: &str) -> Result<Vec<AttachPoint>, LoaderError> {
        let mut points = Vec::new();
        let mut has_outbound = false;
        let mut has_inbound = false;
        for encoded in value.split(';').filter(|point| !point.is_empty()) {
            let point = WirePoint::parse(encoded)?;
            let program = match (point.symbol, point.direction) {
                ("rustls_buffer_plaintext", "outbound") => "handle_rustls_buffer_plaintext",
                ("rustls_take_received_plaintext", "inbound") => {
                    "handle_rustls_take_received_plaintext"
                }
                _ => {
                    return Err(WirePoint::invalid(
                        encoded,
                        "unsupported rustls symbol or direction",
                    ));
                }
            };
            has_outbound |= point.direction == "outbound";
            has_inbound |= point.direction == "inbound";
            points.push(AttachPoint {
                program,
                symbol: point.symbol.to_string(),
                offset: point.offset,
                retprobe: false,
            });
        }
        if !has_outbound || !has_inbound {
            return Err(LoaderError::new(
                "attach_dynamic_tls_plan",
                "rustls direct probe plan must contain an outbound and an inbound point",
            ));
        }
        Ok(points)
    }
}
