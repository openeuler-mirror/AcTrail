//! TLS payload diagnostic counter snapshots.

use libbpf_rs::{MapCore, MapFlags, MapHandle};

use crate::loader::LoaderError;

const COUNTER_NAMES: &[&str] = &[
    "enter_total",
    "namespace_fallback",
    "trace_lookup_miss",
    "trace_lookup_host_fallback",
    "empty_buffer",
    "direct_copy_attempt",
    "direct_copy_too_large",
    "direct_reserve_fail",
    "direct_read_fail",
    "direct_submit_ok",
    "pending_update_fail",
    "pending_update_ok",
    "capture_request_reserve_fail",
    "capture_request_signal_fail",
    "capture_request_submit_ok",
    "completion_total",
    "completion_missing_pending",
    "completion_reserve_fail",
    "completion_submit_ok",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsPayloadDiagnosticCounter {
    pub name: &'static str,
    pub value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsPayloadDiagnostics {
    pub counters: Vec<TlsPayloadDiagnosticCounter>,
}

impl TlsPayloadDiagnostics {
    pub fn nonzero_summary(&self) -> String {
        let entries = self
            .counters
            .iter()
            .filter(|counter| counter.value != 0)
            .map(|counter| format!("{}={}", counter.name, counter.value))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            "all counters are zero".to_string()
        } else {
            entries.join(" ")
        }
    }
}

pub(crate) fn read_tls_payload_diagnostics(
    map: &MapHandle,
) -> Result<TlsPayloadDiagnostics, LoaderError> {
    let mut counters = Vec::with_capacity(COUNTER_NAMES.len());
    for (index, name) in COUNTER_NAMES.iter().copied().enumerate() {
        let key = (index as u32).to_ne_bytes();
        let value = map
            .lookup(&key, MapFlags::ANY)
            .map_err(|error| LoaderError::new("payload_tls_diagnostics", error.to_string()))?
            .ok_or_else(|| {
                LoaderError::new(
                    "payload_tls_diagnostics",
                    format!("missing diagnostic counter {name}"),
                )
            })?;
        counters.push(TlsPayloadDiagnosticCounter {
            name,
            value: read_u64_value(&value)?,
        });
    }
    Ok(TlsPayloadDiagnostics { counters })
}

fn read_u64_value(value: &[u8]) -> Result<u64, LoaderError> {
    value
        .get(..std::mem::size_of::<u64>())
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_ne_bytes)
        .ok_or_else(|| {
            LoaderError::new(
                "payload_tls_diagnostics",
                format!("unexpected diagnostic counter size {}", value.len()),
            )
        })
}
