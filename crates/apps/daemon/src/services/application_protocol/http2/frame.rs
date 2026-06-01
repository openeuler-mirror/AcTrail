//! Minimal HTTP/2 frame decoding.

pub(super) const CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
pub(super) const FRAME_HEADER_BYTES: usize = 9;
pub(super) const DATA_FRAME_TYPE: u8 = 0x0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Frame {
    pub length: usize,
    pub frame_type: u8,
    pub flags: u8,
    pub stream_id: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    pub(super) fn type_name(&self) -> &'static str {
        frame_type_name(self.frame_type)
    }

    pub(super) fn flags_hex(&self) -> String {
        format!("0x{:02x}", self.flags)
    }
}

#[derive(Debug)]
pub(super) enum DecodeStatus {
    NeedMore,
    Frame(Frame),
}

pub(super) fn decode_next(buffer: &[u8], max_frame_bytes: u64) -> Result<DecodeStatus, String> {
    if buffer.len() < FRAME_HEADER_BYTES {
        return Ok(DecodeStatus::NeedMore);
    }
    let length =
        (usize::from(buffer[0]) << 16) | (usize::from(buffer[1]) << 8) | usize::from(buffer[2]);
    if u64::try_from(length).map_err(|error| error.to_string())? > max_frame_bytes {
        return Err(format!(
            "HTTP/2 frame length {length} exceeds configured maximum {max_frame_bytes}"
        ));
    }
    let frame_end = FRAME_HEADER_BYTES
        .checked_add(length)
        .ok_or_else(|| "HTTP/2 frame length overflow".to_string())?;
    if buffer.len() < frame_end {
        return Ok(DecodeStatus::NeedMore);
    }
    let stream_id = (u32::from(buffer[5] & 0x7f) << 24)
        | (u32::from(buffer[6]) << 16)
        | (u32::from(buffer[7]) << 8)
        | u32::from(buffer[8]);
    Ok(DecodeStatus::Frame(Frame {
        length,
        frame_type: buffer[3],
        flags: buffer[4],
        stream_id,
        payload: buffer[FRAME_HEADER_BYTES..frame_end].to_vec(),
    }))
}

pub(super) fn encoded_len(frame: &Frame) -> usize {
    FRAME_HEADER_BYTES + frame.length
}

pub(super) fn frame_type_name(frame_type: u8) -> &'static str {
    match frame_type {
        0x0 => "DATA",
        0x1 => "HEADERS",
        0x2 => "PRIORITY",
        0x3 => "RST_STREAM",
        0x4 => "SETTINGS",
        0x5 => "PUSH_PROMISE",
        0x6 => "PING",
        0x7 => "GOAWAY",
        0x8 => "WINDOW_UPDATE",
        0x9 => "CONTINUATION",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodeStatus, decode_next};

    #[test]
    fn decodes_data_frame_header() {
        let raw = [0, 0, 3, 0, 1, 0, 0, 0, 5, b'a', b'b', b'c'];
        let status = decode_next(&raw, 16).unwrap();
        let DecodeStatus::Frame(frame) = status else {
            panic!("expected frame");
        };
        assert_eq!(frame.length, 3);
        assert_eq!(frame.type_name(), "DATA");
        assert_eq!(frame.flags_hex(), "0x01");
        assert_eq!(frame.stream_id, 5);
        assert_eq!(frame.payload, b"abc");
    }

    #[test]
    fn waits_for_partial_frame() {
        let raw = [0, 0, 3, 0, 1, 0, 0, 0, 5, b'a'];
        assert!(matches!(
            decode_next(&raw, 16).unwrap(),
            DecodeStatus::NeedMore
        ));
    }

    #[test]
    fn rejects_oversized_frame() {
        let raw = [0, 0, 17, 0, 1, 0, 0, 0, 5];
        let error = decode_next(&raw, 16).unwrap_err();
        assert!(error.contains("exceeds configured maximum"));
    }
}
