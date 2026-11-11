//! Optional numbers, one field each: the number, or `none` when absent.

use super::{ControlCodecError, parse_u32, parse_u64};

pub(super) fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

pub(super) fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

pub(super) fn parse_optional_u64(
    value: &str,
    field_name: &str,
) -> Result<Option<u64>, ControlCodecError> {
    if value == "none" {
        return Ok(None);
    }
    parse_u64(value, field_name).map(Some)
}

pub(super) fn parse_optional_u32(
    value: &str,
    field_name: &str,
) -> Result<Option<u32>, ControlCodecError> {
    if value == "none" {
        return Ok(None);
    }
    parse_u32(value, field_name).map(Some)
}
