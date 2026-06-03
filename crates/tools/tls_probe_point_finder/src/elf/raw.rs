use crate::{ToolError, ToolResult};

pub(super) fn bounded(data: &[u8], offset: u64, size: u64) -> ToolResult<&[u8]> {
    let offset =
        usize::try_from(offset).map_err(|_| ToolError::new("ELF offset overflows usize"))?;
    let size = usize::try_from(size).map_err(|_| ToolError::new("ELF size overflows usize"))?;
    bounded_usize(data, offset, size)
}

pub(super) fn bounded_usize(data: &[u8], offset: usize, size: usize) -> ToolResult<&[u8]> {
    offset
        .checked_add(size)
        .filter(|end| *end <= data.len())
        .map(|end| &data[offset..end])
        .ok_or_else(|| ToolError::new("ELF data is truncated"))
}

pub(super) fn checked_table_offset(
    table_offset: u64,
    entry_size: u64,
    index: u64,
    message: &'static str,
) -> ToolResult<u64> {
    table_offset
        .checked_add(
            index
                .checked_mul(entry_size)
                .ok_or_else(|| ToolError::new(message))?,
        )
        .ok_or_else(|| ToolError::new(message))
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub(super) fn read_u8(data: &[u8], offset: usize) -> ToolResult<u8> {
    bounded_usize(data, offset, std::mem::size_of::<u8>()).map(|bytes| bytes[0])
}

pub(super) fn read_u16(data: &[u8], offset: usize) -> ToolResult<u16> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u16>())?;
    Ok(u16::from_le_bytes(bytes.try_into().expect("bounded u16")))
}

pub(super) fn read_u32(data: &[u8], offset: usize) -> ToolResult<u32> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u32>())?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("bounded u32")))
}

pub(super) fn read_i64(data: &[u8], offset: usize) -> ToolResult<i64> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<i64>())?;
    Ok(i64::from_le_bytes(bytes.try_into().expect("bounded i64")))
}

pub(super) fn read_u64(data: &[u8], offset: usize) -> ToolResult<u64> {
    let bytes = bounded_usize(data, offset, std::mem::size_of::<u64>())?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("bounded u64")))
}

pub(super) fn string_at(data: &[u8], offset: u32) -> ToolResult<Option<&str>> {
    let offset =
        usize::try_from(offset).map_err(|_| ToolError::new("ELF string offset overflow"))?;
    if offset == 0 {
        return Ok(None);
    }
    string_at_usize(data, offset).map(Some)
}

pub(super) fn string_at_usize(data: &[u8], offset: usize) -> ToolResult<&str> {
    let rest = data
        .get(offset..)
        .ok_or_else(|| ToolError::new("ELF string offset is out of range"))?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| ToolError::new("ELF string is unterminated"))?;
    std::str::from_utf8(&rest[..end]).map_err(|error| ToolError::new(error.to_string()))
}
