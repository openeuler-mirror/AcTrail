use plugin_system::{
    PluginHostcallMetrics, PluginInstanceStatus, PluginLifecycleState, PluginPayloadReadMetrics,
    PluginPurpose, PluginRuntimeKind,
};

use super::{ControlCodecError, field, parse_usize};

const HOSTCALL_METRIC_FIELDS: usize = 9;
const WARNINGS_FIELD: &str = "warnings";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostcallMetricsMode {
    Absent,
    Present,
}

pub(super) fn encode_plugin_statuses_v2(
    fields: &mut Vec<String>,
    statuses: &[PluginInstanceStatus],
) {
    fields.push(statuses.len().to_string());
    for status in statuses {
        encode_plugin_status_v2(fields, status);
    }
}

pub(super) fn encode_plugin_status_v2(fields: &mut Vec<String>, status: &PluginInstanceStatus) {
    let mut status_fields = Vec::new();
    encode_plugin_status_fields(&mut status_fields, status);
    fields.push(status_fields.len().to_string());
    fields.extend(status_fields);
}

fn encode_plugin_status_fields(fields: &mut Vec<String>, status: &PluginInstanceStatus) {
    fields.push(status.instance_id.clone());
    fields.push(status.plugin_id.clone());
    fields.push(status.purpose.as_str().to_string());
    fields.push(status.runtime.as_str().to_string());
    fields.push(status.state.as_str().to_string());
    fields.push(status.host_grants.len().to_string());
    fields.extend(status.host_grants.iter().cloned());
    fields.push(optional_u64(status.queue_depth));
    fields.push(optional_u32(status.queue_capacity));
    fields.push(status.observed_records.to_string());
    fields.push(status.dropped_records.to_string());
    fields.push(
        status
            .last_error
            .clone()
            .unwrap_or_else(|| "none".to_string()),
    );
    let payload_read = status.hostcall_metrics.payload_read;
    fields.push(payload_read.calls.to_string());
    fields.push(payload_read.bytes.to_string());
    fields.push(payload_read.denied.to_string());
    fields.push(payload_read.not_found.to_string());
    fields.push(payload_read.invalid.to_string());
    fields.push(payload_read.too_large.to_string());
    fields.push(payload_read.truncated.to_string());
    fields.push(payload_read.latency_total_ns.to_string());
    fields.push(payload_read.latency_max_ns.to_string());
    if !status.warnings.is_empty() {
        fields.push(WARNINGS_FIELD.to_string());
        fields.push(status.warnings.len().to_string());
        fields.extend(status.warnings.iter().cloned());
    }
}

pub(super) fn decode_plugin_statuses_v1(
    fields: &[String],
    offset: usize,
) -> Result<(Vec<PluginInstanceStatus>, usize), ControlCodecError> {
    if let Ok((statuses, cursor)) =
        decode_plugin_statuses_v1_with_mode(fields, offset, HostcallMetricsMode::Absent)
        && cursor == fields.len()
    {
        return Ok((statuses, cursor));
    }
    let (statuses, cursor) =
        decode_plugin_statuses_v1_with_mode(fields, offset, HostcallMetricsMode::Present)?;
    if cursor != fields.len() {
        return Err(ControlCodecError::new(
            "decode",
            "legacy plugin status frame has trailing fields",
        ));
    }
    Ok((statuses, cursor))
}

fn decode_plugin_statuses_v1_with_mode(
    fields: &[String],
    offset: usize,
    hostcall_metrics: HostcallMetricsMode,
) -> Result<(Vec<PluginInstanceStatus>, usize), ControlCodecError> {
    let count = parse_usize(field(fields, offset)?, "plugin_count")?;
    let mut cursor = offset + 1;
    let mut statuses = Vec::new();
    for _ in 0..count {
        let (status, next_cursor) = decode_plugin_status_fields(fields, cursor, hostcall_metrics)?;
        statuses.push(status);
        cursor = next_cursor;
    }
    Ok((statuses, cursor))
}

pub(super) fn decode_plugin_status_v1(
    fields: &[String],
    offset: usize,
) -> Result<(PluginInstanceStatus, usize), ControlCodecError> {
    if let Ok((status, cursor)) =
        decode_plugin_status_fields(fields, offset, HostcallMetricsMode::Absent)
        && cursor == fields.len()
    {
        return Ok((status, cursor));
    }
    decode_plugin_status_fields(fields, offset, HostcallMetricsMode::Present)
}

pub(super) fn decode_plugin_statuses_v2(
    fields: &[String],
    offset: usize,
) -> Result<(Vec<PluginInstanceStatus>, usize), ControlCodecError> {
    let count = parse_usize(field(fields, offset)?, "plugin_count")?;
    let mut cursor = offset + 1;
    let mut statuses = Vec::new();
    for _ in 0..count {
        let (status, next_cursor) = decode_plugin_status_v2(fields, cursor)?;
        statuses.push(status);
        cursor = next_cursor;
    }
    Ok((statuses, cursor))
}

pub(super) fn decode_plugin_status_v2(
    fields: &[String],
    offset: usize,
) -> Result<(PluginInstanceStatus, usize), ControlCodecError> {
    let field_count = parse_usize(field(fields, offset)?, "plugin_status_field_count")?;
    let start = offset + 1;
    let end = start
        .checked_add(field_count)
        .ok_or_else(|| ControlCodecError::new("decode", "plugin status field count overflow"))?;
    if end > fields.len() {
        return Err(ControlCodecError::new(
            "decode",
            "plugin status frame field count exceeds reply fields",
        ));
    }
    let (status, _) =
        decode_plugin_status_fields(&fields[start..end], 0, HostcallMetricsMode::Present)?;
    Ok((status, end))
}

fn decode_plugin_status_fields(
    fields: &[String],
    offset: usize,
    hostcall_metrics_mode: HostcallMetricsMode,
) -> Result<(PluginInstanceStatus, usize), ControlCodecError> {
    let purpose = PluginPurpose::from_wire(field(fields, offset + 2)?)
        .map_err(|error| ControlCodecError::new("decode", error))?;
    let runtime = PluginRuntimeKind::from_wire(field(fields, offset + 3)?)
        .map_err(|error| ControlCodecError::new("decode", error))?;
    let state = PluginLifecycleState::from_wire(field(fields, offset + 4)?)
        .map_err(|error| ControlCodecError::new("decode", error))?;
    let host_grant_count = parse_usize(field(fields, offset + 5)?, "host_grant_count")?;
    let host_grants_start = offset + 6;
    let mut host_grants = Vec::new();
    for grant_offset in 0..host_grant_count {
        host_grants.push(field(fields, host_grants_start + grant_offset)?.clone());
    }
    let metrics_offset = host_grants_start + host_grant_count;
    let base_end = metrics_offset + 5;
    let hostcall_metrics = match hostcall_metrics_mode {
        HostcallMetricsMode::Absent => PluginHostcallMetrics::default(),
        HostcallMetricsMode::Present => PluginHostcallMetrics {
            payload_read: PluginPayloadReadMetrics {
                calls: parse_u64(field(fields, base_end)?, "payload_read_calls")?,
                bytes: parse_u64(field(fields, base_end + 1)?, "payload_read_bytes")?,
                denied: parse_u64(field(fields, base_end + 2)?, "payload_read_denied")?,
                not_found: parse_u64(field(fields, base_end + 3)?, "payload_read_not_found")?,
                invalid: parse_u64(field(fields, base_end + 4)?, "payload_read_invalid")?,
                too_large: parse_u64(field(fields, base_end + 5)?, "payload_read_too_large")?,
                truncated: parse_u64(field(fields, base_end + 6)?, "payload_read_truncated")?,
                latency_total_ns: parse_u64(
                    field(fields, base_end + 7)?,
                    "payload_read_latency_total_ns",
                )?,
                latency_max_ns: parse_u64(
                    field(fields, base_end + 8)?,
                    "payload_read_latency_max_ns",
                )?,
            },
        },
    };
    let warning_count_offset = match hostcall_metrics_mode {
        HostcallMetricsMode::Absent => base_end,
        HostcallMetricsMode::Present => base_end + HOSTCALL_METRIC_FIELDS,
    };
    let mut warnings = Vec::new();
    let next_offset = if matches!(hostcall_metrics_mode, HostcallMetricsMode::Present)
        && warning_count_offset < fields.len()
        && field(fields, warning_count_offset)? == WARNINGS_FIELD
    {
        let warning_count = parse_usize(
            field(fields, warning_count_offset + 1)?,
            "plugin_warning_count",
        )?;
        let warning_start = warning_count_offset + 2;
        let warning_end = warning_start
            .checked_add(warning_count)
            .ok_or_else(|| ControlCodecError::new("decode", "plugin warning count overflow"))?;
        if warning_end > fields.len() {
            return Err(ControlCodecError::new(
                "decode",
                "plugin warning count exceeds status fields",
            ));
        }
        warnings.extend(fields[warning_start..warning_end].iter().cloned());
        warning_end
    } else {
        warning_count_offset
    };
    Ok((
        PluginInstanceStatus {
            instance_id: field(fields, offset)?.clone(),
            plugin_id: field(fields, offset + 1)?.clone(),
            purpose,
            runtime,
            state,
            host_grants,
            queue_depth: parse_optional_u64(field(fields, metrics_offset)?, "queue_depth")?,
            queue_capacity: parse_optional_u32(
                field(fields, metrics_offset + 1)?,
                "queue_capacity",
            )?,
            observed_records: parse_u64(field(fields, metrics_offset + 2)?, "observed_records")?,
            dropped_records: parse_u64(field(fields, metrics_offset + 3)?, "dropped_records")?,
            hostcall_metrics,
            last_error: match field(fields, metrics_offset + 4)?.as_str() {
                "none" => None,
                value => Some(value.to_string()),
            },
            warnings,
        },
        next_offset,
    ))
}

fn parse_u64(value: &str, field_name: &str) -> Result<u64, ControlCodecError> {
    value
        .parse::<u64>()
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {field_name}")))
}

fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn parse_optional_u64(value: &str, field_name: &str) -> Result<Option<u64>, ControlCodecError> {
    if value == "none" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {field_name}")))
}

fn parse_optional_u32(value: &str, field_name: &str) -> Result<Option<u32>, ControlCodecError> {
    if value == "none" {
        return Ok(None);
    }
    value
        .parse::<u32>()
        .map(Some)
        .map_err(|_| ControlCodecError::new("decode", format!("invalid {field_name}")))
}
