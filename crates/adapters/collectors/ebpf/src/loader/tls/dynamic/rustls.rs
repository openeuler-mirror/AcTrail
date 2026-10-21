//! Rustls outbound capture follows the accepted length returned by buffer_plaintext.

use super::points::{AttachPoint, WirePoint};
use crate::loader::LoaderError;

pub(super) struct RustlsPlan;

impl RustlsPlan {
    pub(super) fn parse(value: &str) -> Result<Vec<AttachPoint>, LoaderError> {
        let mut points = Vec::new();
        let mut has_outbound = false;
        let mut has_inbound = false;
        let mut lifecycle_cookie = None;
        let mut payload_cookies = Vec::new();
        for encoded in value.split(';').filter(|point| !point.is_empty()) {
            let point = WirePoint::parse(encoded)?;
            if point.cookie > 32752 || point.cookie % 8 != 0 {
                return Err(WirePoint::invalid(
                    encoded,
                    "invalid rustls certificate layout",
                ));
            }
            let program = match (point.symbol, point.direction) {
                ("rustls_start_outgoing_traffic", "control") if point.cookie != 0 => {
                    if lifecycle_cookie.replace(point.cookie).is_some() {
                        return Err(WirePoint::invalid(
                            encoded,
                            "duplicate rustls lifecycle point",
                        ));
                    }
                    "handle_rustls_start_outgoing_traffic"
                }
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
            if point.direction != "control" {
                payload_cookies.push(point.cookie);
            }
            has_outbound |= point.direction == "outbound";
            has_inbound |= point.direction == "inbound";
            points.push(AttachPoint {
                program,
                symbol: point.symbol.to_string(),
                offset: point.offset,
                cookie: point.cookie,
                retprobe: false,
            });
            if point.direction == "outbound" {
                points.push(AttachPoint {
                    program: "handle_rustls_buffer_plaintext_exit",
                    symbol: point.symbol.to_string(),
                    offset: point.offset,
                    cookie: point.cookie,
                    retprobe: true,
                });
            }
        }
        if payload_cookies
            .iter()
            .any(|cookie| *cookie != lifecycle_cookie.unwrap_or(0))
        {
            return Err(LoaderError::new(
                "attach_dynamic_tls_plan",
                "rustls lifecycle and payload layouts differ",
            ));
        }
        // Register connection identities before any plaintext probe can run.
        points.sort_by_key(|point| point.symbol != "rustls_start_outgoing_traffic");
        if !has_outbound || !has_inbound {
            return Err(LoaderError::new(
                "attach_dynamic_tls_plan",
                "rustls direct probe plan must contain an outbound and an inbound point",
            ));
        }
        Ok(points)
    }
}
