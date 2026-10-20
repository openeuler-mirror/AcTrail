//! Common plan representation and wire-point validation.

use super::super::targets::{BORINGSSL_UPROBE_TARGETS, OPENSSL_UPROBE_TARGETS};
use super::{rustls::RustlsPlan, ssl::SslPlan};
use crate::loader::LoaderError;

pub(super) struct AttachPoint {
    pub(super) program: &'static str,
    pub(super) symbol: String,
    pub(super) offset: u64,
    pub(super) cookie: u64,
    pub(super) retprobe: bool,
}

pub(super) struct AttachmentPlan {
    pub(super) points: Vec<AttachPoint>,
}

impl AttachmentPlan {
    pub(super) fn resolve(provider: &str, encoded: &str) -> Result<Self, LoaderError> {
        let points = match provider {
            "openssl" => SslPlan::parse(encoded, OPENSSL_UPROBE_TARGETS)?,
            "boringssl" => SslPlan::parse(encoded, BORINGSSL_UPROBE_TARGETS)?,
            "rustls" => RustlsPlan::parse(encoded)?,
            _ => {
                return Err(LoaderError::new(
                    "attach_dynamic_tls",
                    format!(
                        "provider {provider} is not supported for direct detector-plan attachment"
                    ),
                ));
            }
        };
        Ok(Self { points })
    }
}

pub(super) struct WirePoint<'a> {
    pub(super) symbol: &'a str,
    pub(super) direction: &'a str,
    pub(super) offset: u64,
    pub(super) cookie: u64,
}

impl<'a> WirePoint<'a> {
    pub(super) fn parse(encoded: &'a str) -> Result<Self, LoaderError> {
        let mut fields = encoded.split(':');
        let symbol = fields.next().unwrap_or_default();
        let direction = fields.next().unwrap_or_default();
        let offset = fields
            .next()
            .ok_or_else(|| Self::invalid(encoded, "missing offset"))?
            .parse::<u64>()
            .map_err(|error| Self::invalid(encoded, &format!("invalid offset: {error}")))?;
        let cookie = fields
            .next()
            .map(|field| field.parse::<u64>())
            .transpose()
            .map_err(|error| Self::invalid(encoded, &format!("invalid cookie: {error}")))?
            .unwrap_or(0);
        if fields.next().is_some() {
            return Err(Self::invalid(encoded, "unexpected field"));
        }
        Ok(Self {
            symbol,
            direction,
            offset,
            cookie,
        })
    }

    pub(super) fn invalid(point: &str, reason: &str) -> LoaderError {
        LoaderError::new(
            "attach_dynamic_tls_plan",
            format!("invalid detector point {point:?}: {reason}"),
        )
    }
}
