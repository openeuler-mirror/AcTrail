use super::{
    read_bool, read_option_u64, read_u8, read_varint, write_bool, write_option_u64, write_varint,
};
use model_core::event::{FileIoDirection, FileIoSummary, FileIoTargetKind, FileSummaryPathState};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn encode(out: &mut Vec<u8>, summary: &Option<FileIoSummary>) {
    let Some(summary) = summary else {
        out.push(0);
        return;
    };
    out.push(1);
    write_varint(out, summary.file_token);
    out.push(match summary.direction {
        FileIoDirection::Read => 1,
        FileIoDirection::Write => 2,
    });
    out.push(match summary.target_kind {
        FileIoTargetKind::RegularFile => 1,
        FileIoTargetKind::CharacterDevice => 2,
    });
    write_varint(out, u64::from(summary.errno));
    encode_time(out, summary.interval_start);
    encode_time(out, summary.interval_end);
    write_bool(out, summary.interval_complete);
    write_option_u64(out, summary.operations);
    write_option_u64(out, summary.bytes);
    out.push(match summary.path_state {
        FileSummaryPathState::Resolved => 1,
        FileSummaryPathState::Truncated => 2,
        FileSummaryPathState::Unavailable => 3,
    });
}
pub(super) fn decode(bytes: &[u8], c: &mut usize) -> Result<Option<FileIoSummary>, String> {
    if !read_bool(bytes, c)? {
        return Ok(None);
    }
    let file_token = read_varint(bytes, c)?;
    let direction = match read_u8(bytes, c)? {
        1 => FileIoDirection::Read,
        2 => FileIoDirection::Write,
        _ => return Err("invalid file I/O direction".into()),
    };
    let target_kind = match read_u8(bytes, c)? {
        1 => FileIoTargetKind::RegularFile,
        2 => FileIoTargetKind::CharacterDevice,
        _ => return Err("invalid file I/O target".into()),
    };
    let errno = u32::try_from(read_varint(bytes, c)?).map_err(|_| "invalid file I/O errno")?;
    let interval_start = decode_time(bytes, c)?;
    let interval_end = decode_time(bytes, c)?;
    let interval_complete = read_bool(bytes, c)?;
    let operations = read_option_u64(bytes, c)?;
    let size = read_option_u64(bytes, c)?;
    let path_state = match read_u8(bytes, c)? {
        1 => FileSummaryPathState::Resolved,
        2 => FileSummaryPathState::Truncated,
        3 => FileSummaryPathState::Unavailable,
        _ => return Err("invalid file path state".into()),
    };
    Ok(Some(FileIoSummary {
        file_token,
        direction,
        target_kind,
        errno,
        interval_start,
        interval_end,
        interval_complete,
        operations,
        bytes: size,
        path_state,
    }))
}
fn encode_time(out: &mut Vec<u8>, value: SystemTime) {
    let (negative, duration) = match value.duration_since(UNIX_EPOCH) {
        Ok(d) => (false, d),
        Err(e) => (true, e.duration()),
    };
    write_bool(out, negative);
    write_varint(out, duration.as_secs());
    write_varint(out, u64::from(duration.subsec_nanos()));
}
fn decode_time(bytes: &[u8], c: &mut usize) -> Result<SystemTime, String> {
    let negative = read_bool(bytes, c)?;
    let seconds = read_varint(bytes, c)?;
    let nanos = read_varint(bytes, c)?;
    if nanos >= 1_000_000_000 {
        return Err("invalid summary timestamp nanoseconds".into());
    }
    let duration = Duration::new(seconds, nanos as u32);
    if negative {
        UNIX_EPOCH.checked_sub(duration)
    } else {
        UNIX_EPOCH.checked_add(duration)
    }
    .ok_or_else(|| "summary timestamp overflow".into())
}
