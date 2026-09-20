//! OpenSSL and BoringSSL share the SSL read/write calling convention.

use super::super::targets::TlsUprobeTarget;
use super::points::{AttachPoint, WirePoint};
use crate::loader::LoaderError;

pub(super) struct SslPlan;

impl SslPlan {
    pub(super) fn parse(
        value: &str,
        targets: &[TlsUprobeTarget],
    ) -> Result<Vec<AttachPoint>, LoaderError> {
        let mut points = Vec::new();
        let mut has_outbound = false;
        let mut has_inbound = false;
        for encoded in value.split(';').filter(|point| !point.is_empty()) {
            let point = WirePoint::parse(encoded)?;
            match (point.symbol, point.direction) {
                ("SSL_write" | "SSL_write_ex", "outbound")
                | ("SSL_read" | "SSL_read_ex", "inbound") => {}
                _ => {
                    return Err(WirePoint::invalid(
                        encoded,
                        "unsupported SSL symbol or direction",
                    ));
                }
            }
            if !targets.iter().any(|target| target.symbol == point.symbol) {
                return Err(WirePoint::invalid(
                    encoded,
                    "SSL symbol has no provider target",
                ));
            }
            has_outbound |= point.direction == "outbound";
            has_inbound |= point.direction == "inbound";
            for target in targets
                .iter()
                .filter(|target| target.symbol == point.symbol)
            {
                points.push(AttachPoint {
                    program: target.program,
                    symbol: point.symbol.to_string(),
                    offset: point.offset,
                    retprobe: target.retprobe,
                });
            }
        }
        if !has_outbound || !has_inbound {
            return Err(LoaderError::new(
                "attach_dynamic_tls_plan",
                "SSL direct probe plan must contain an outbound and an inbound point",
            ));
        }
        Ok(points)
    }
}
