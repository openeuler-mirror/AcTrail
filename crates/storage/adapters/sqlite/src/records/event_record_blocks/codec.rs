use std::io::{Read, Write};

use model_core::event::{DomainEvent, EventEnvelope};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;

use crate::config::{
    SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS, SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES,
};
use crate::records::{
    BlockKind, EventMeta, PayloadBlock, decode_event_kind, decode_event_payload,
    decode_policy_record, decode_time, encode_event_kind, encode_event_payload,
    encode_policy_record, encode_time, restore_shared_path,
};

pub(super) const CODEC_VERSION: i64 = 3;
pub(super) const KIND_COUNT: usize = 11;

pub(super) struct EncodedBlockEvent {
    pub(super) event_id: u64,
    pub(super) observed_at: i64,
    pub(super) kind_index: usize,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn encode_event(
    mut event: DomainEvent,
    payload_path_id: Option<i64>,
) -> Result<EncodedBlockEvent, rusqlite::Error> {
    let encoded_payload = encode_event_payload(&mut event.payload)?;
    let (redactions, truncations) = encode_policy_record(&event.policy);
    let has_details =
        event.policy.note.is_some() || !redactions.is_empty() || !truncations.is_empty();
    let event_meta = EventMeta::encode(
        &event.envelope.collector,
        &event.envelope.flags,
        event.policy.verdict,
        !encoded_payload.blocks.is_empty(),
        has_details,
    )
    .ok_or(rusqlite::Error::InvalidQuery)?;
    let kind_code = encode_event_kind(event.envelope.kind);
    let mut bytes = Vec::with_capacity(encoded_payload.fields.len().saturating_add(64));
    write_varint(&mut bytes, event.envelope.event_id.get());
    write_i64(&mut bytes, encode_time(event.envelope.observed_at));
    write_varint(&mut bytes, event.envelope.process.get());
    write_varint(&mut bytes, event_meta.code() as u64);
    write_varint(&mut bytes, kind_code as u64);
    write_option_i64(&mut bytes, payload_path_id);
    write_bytes(&mut bytes, &encoded_payload.fields);
    write_varint(&mut bytes, encoded_payload.blocks.len() as u64);
    for block in encoded_payload.blocks {
        write_varint(&mut bytes, block.kind.to_i64() as u64);
        write_bytes(&mut bytes, &block.bytes);
    }
    if has_details {
        write_option_string(&mut bytes, event.policy.note.as_deref());
        write_string(&mut bytes, &redactions);
        write_string(&mut bytes, &truncations);
    }
    Ok(EncodedBlockEvent {
        event_id: event.envelope.event_id.get(),
        observed_at: encode_time(event.envelope.observed_at),
        kind_index: kind_code as usize,
        bytes,
    })
}

pub(super) fn compress_block(frames: &[Vec<u8>], level: i32) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), level)?;
    encoder.write_all(&[CODEC_VERSION as u8])?;
    let mut prefix = Vec::with_capacity(10);
    write_varint(&mut prefix, frames.len() as u64);
    encoder.write_all(&prefix)?;
    for frame in frames {
        prefix.clear();
        write_varint(&mut prefix, frame.len() as u64);
        encoder.write_all(&prefix)?;
        encoder.write_all(frame)?;
    }
    encoder.finish()
}

pub(super) fn encode_kind_counts(counts: &[u64; KIND_COUNT]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(KIND_COUNT);
    for count in counts {
        write_varint(&mut bytes, *count);
    }
    bytes
}

pub(crate) fn decode_kind_counts(bytes: &[u8]) -> Result<[u64; KIND_COUNT], rusqlite::Error> {
    let mut cursor = 0usize;
    let mut counts = [0u64; KIND_COUNT];
    for count in &mut counts {
        *count = read_varint(bytes, &mut cursor)?;
    }
    if cursor != bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(counts)
}

pub(crate) fn decode_block<F>(
    trace_id: u64,
    codec_version: i64,
    expected_event_count: usize,
    uncompressed_bytes: usize,
    encoded_bytes: &[u8],
    resolve_path: &mut F,
) -> Result<Vec<DomainEvent>, rusqlite::Error>
where
    F: FnMut(i64) -> Result<String, rusqlite::Error>,
{
    if codec_version != CODEC_VERSION {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if expected_event_count == 0
        || expected_event_count > SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS
        || uncompressed_bytes == 0
        || uncompressed_bytes > SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut decoder = zstd::stream::Decoder::new(std::io::Cursor::new(encoded_bytes))
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(uncompressed_bytes)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    bytes.resize(uncompressed_bytes, 0);
    decoder
        .read_exact(&mut bytes)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let mut trailing = [0u8; 1];
    if decoder
        .read(&mut trailing)
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        != 0
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut cursor = 0usize;
    if read_u8(&bytes, &mut cursor)? != CODEC_VERSION as u8 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let count = read_usize(&bytes, &mut cursor)?;
    if count != expected_event_count || count > bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        let frame = read_bytes_slice(&bytes, &mut cursor)?;
        events.push(decode_event(trace_id, frame, resolve_path)?);
    }
    if cursor != bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(events)
}

pub(crate) fn decode_event<F>(
    trace_id: u64,
    bytes: &[u8],
    resolve_path: &mut F,
) -> Result<DomainEvent, rusqlite::Error>
where
    F: FnMut(i64) -> Result<String, rusqlite::Error>,
{
    let mut cursor = 0usize;
    let event_id = read_varint(bytes, &mut cursor)?;
    let observed_at = read_i64(bytes, &mut cursor)?;
    let process_id = read_varint(bytes, &mut cursor)?;
    let event_meta = EventMeta::decode(read_i64_varint(bytes, &mut cursor)?)?;
    let kind_code = read_i64_varint(bytes, &mut cursor)?;
    let payload_path_id = read_option_i64(bytes, &mut cursor)?;
    let fields = read_bytes_slice(bytes, &mut cursor)?;
    let block_count = read_usize(bytes, &mut cursor)?;
    if block_count > 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut payload_blocks = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        let kind = BlockKind::from_i64(read_i64_varint(bytes, &mut cursor)?)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        payload_blocks.push(PayloadBlock {
            kind,
            bytes: read_bytes_slice(bytes, &mut cursor)?.to_vec(),
        });
    }
    let (note, redactions, truncations) = if event_meta.has_policy_details() {
        (
            read_option_string(bytes, &mut cursor)?,
            read_string(bytes, &mut cursor)?,
            read_string(bytes, &mut cursor)?,
        )
    } else {
        (None, String::new(), String::new())
    };
    if cursor != bytes.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut payload = decode_event_payload(fields, &payload_blocks)?;
    let path = payload_path_id.map(resolve_path).transpose()?;
    restore_shared_path(&mut payload, path)?;
    Ok(DomainEvent {
        envelope: EventEnvelope {
            event_id: EventId::new(event_id),
            trace_id: TraceId::new(trace_id),
            observed_at: decode_time(observed_at),
            process: ProcessIdentity::new(process_id),
            collector: event_meta.collector()?,
            kind: decode_event_kind(kind_code)?,
            flags: event_meta.flags()?,
        },
        payload,
        policy: decode_policy_record(event_meta.policy()?, note, &redactions, &truncations)?,
    })
}

fn write_varint(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            bytes.push(byte);
            return;
        }
        bytes.push(byte | 0x80);
    }
}

fn write_i64(bytes: &mut Vec<u8>, value: i64) {
    write_varint(bytes, ((value as u64) << 1) ^ ((value >> 63) as u64));
}

fn write_option_i64(bytes: &mut Vec<u8>, value: Option<i64>) {
    match value {
        Some(value) => {
            bytes.push(1);
            write_i64(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn write_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
    write_varint(bytes, value.len() as u64);
    bytes.extend_from_slice(value);
}

fn write_string(bytes: &mut Vec<u8>, value: &str) {
    write_bytes(bytes, value.as_bytes());
}

fn write_option_string(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            write_string(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, rusqlite::Error> {
    let byte = bytes
        .get(*cursor)
        .copied()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    *cursor += 1;
    Ok(byte)
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, rusqlite::Error> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = read_u8(bytes, cursor)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= u64::BITS {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
}

fn read_i64(bytes: &[u8], cursor: &mut usize) -> Result<i64, rusqlite::Error> {
    let raw = read_varint(bytes, cursor)?;
    Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
}

fn read_i64_varint(bytes: &[u8], cursor: &mut usize) -> Result<i64, rusqlite::Error> {
    i64::try_from(read_varint(bytes, cursor)?).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn read_usize(bytes: &[u8], cursor: &mut usize) -> Result<usize, rusqlite::Error> {
    usize::try_from(read_varint(bytes, cursor)?).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn read_option_i64(bytes: &[u8], cursor: &mut usize) -> Result<Option<i64>, rusqlite::Error> {
    match read_u8(bytes, cursor)? {
        0 => Ok(None),
        1 => read_i64(bytes, cursor).map(Some),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn read_bytes_slice<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], rusqlite::Error> {
    let length = read_usize(bytes, cursor)?;
    let end = cursor
        .checked_add(length)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    *cursor = end;
    Ok(value)
}

fn read_string(bytes: &[u8], cursor: &mut usize) -> Result<String, rusqlite::Error> {
    std::str::from_utf8(read_bytes_slice(bytes, cursor)?)
        .map(str::to_owned)
        .map_err(|_| rusqlite::Error::InvalidQuery)
}

fn read_option_string(bytes: &[u8], cursor: &mut usize) -> Result<Option<String>, rusqlite::Error> {
    match read_u8(bytes, cursor)? {
        0 => Ok(None),
        1 => read_string(bytes, cursor).map(Some),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
