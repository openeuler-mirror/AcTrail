//! Runtime probe plan descriptors shared by launcher, daemon, and preload runtime.

use std::path::PathBuf;

use tls_probe_point_finder::{PayloadDirection as FinderDirection, ProbePointPlan};

use crate::{SyncError, SyncResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlanDescriptor {
    pub target: PathBuf,
    pub binary: PathBuf,
    pub provider: String,
    pub points: String,
}

pub fn runtime_plan_descriptor(plan: &ProbePointPlan) -> SyncResult<RuntimePlanDescriptor> {
    Ok(RuntimePlanDescriptor {
        target: plan.target.binary.clone(),
        binary: plan.binary.path.clone(),
        provider: plan.provider.as_str().to_string(),
        points: encode_points(plan)?,
    })
}

pub fn runtime_plan_bundle(plans: &[ProbePointPlan]) -> SyncResult<String> {
    let mut values = Vec::new();
    for plan in plans {
        values.push(encode_runtime_plan(&runtime_plan_descriptor(plan)?));
    }
    Ok(values.join("\n"))
}

pub fn encode_runtime_plan(plan: &RuntimePlanDescriptor) -> String {
    format!(
        "{}|{}|{}|{}",
        encode_hex(plan.target.display().to_string().as_bytes()),
        encode_hex(plan.binary.display().to_string().as_bytes()),
        encode_hex(plan.provider.as_bytes()),
        encode_hex(plan.points.as_bytes())
    )
}

pub fn decode_runtime_plan(value: &str) -> SyncResult<RuntimePlanDescriptor> {
    let mut parts = value.split('|');
    let target = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| SyncError::new(format!("invalid runtime plan item: {value}")))?,
    )?;
    let binary = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| SyncError::new(format!("invalid runtime plan item: {value}")))?,
    )?;
    let provider = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| SyncError::new(format!("invalid runtime plan item: {value}")))?,
    )?;
    let points = decode_hex_string(
        parts
            .next()
            .ok_or_else(|| SyncError::new(format!("invalid runtime plan item: {value}")))?,
    )?;
    if parts.next().is_some() {
        return Err(SyncError::new(format!(
            "invalid runtime plan item: {value}"
        )));
    }
    Ok(RuntimePlanDescriptor {
        target: PathBuf::from(target),
        binary: PathBuf::from(binary),
        provider,
        points,
    })
}

pub fn encode_points(plan: &ProbePointPlan) -> SyncResult<String> {
    let mut values = Vec::new();
    for point in &plan.points {
        let Some(direction) = direction_to_core(point.direction) else {
            continue;
        };
        values.push(format!(
            "{}:{}:{}",
            point.symbol, direction, point.file_offset
        ));
    }
    if values.is_empty() {
        return Err(SyncError::new("probe plan has no payload hook points"));
    }
    Ok(values.join(";"))
}

fn direction_to_core(direction: FinderDirection) -> Option<&'static str> {
    match direction {
        FinderDirection::Inbound => Some("inbound"),
        FinderDirection::Outbound => Some("outbound"),
        FinderDirection::Control => None,
    }
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

pub(crate) fn decode_hex(value: &str) -> SyncResult<Vec<u8>> {
    if value.len() % 2 != 0 {
        return Err(SyncError::new("hex value has odd length"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk)
            .map_err(|error| SyncError::new(format!("hex utf8: {error}")))?;
        bytes.push(
            u8::from_str_radix(text, 16)
                .map_err(|error| SyncError::new(format!("hex {text}: {error}")))?,
        );
    }
    Ok(bytes)
}

pub(crate) fn decode_hex_string(value: &str) -> SyncResult<String> {
    String::from_utf8(decode_hex(value)?)
        .map_err(|error| SyncError::new(format!("hex utf8: {error}")))
}
