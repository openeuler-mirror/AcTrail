//! HTTP/2 helpers for payload body retention.

const CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const FRAME_HEADER_BYTES: usize = 9;
const DATA_FRAME_TYPE: u8 = 0x0;
const HEADERS_FRAME_TYPE: u8 = 0x1;
const CONTINUATION_FRAME_TYPE: u8 = 0x9;
const FLAG_PADDED: u8 = 0x8;

pub(super) struct ClassifiedMessage {
    pub(super) stream_id: Option<u32>,
    pub(super) llm: bool,
}

pub(super) fn classify_request(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let frames = frames(bytes)?;
    if frames.body.is_empty() {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: frames.stream_id,
        llm: body_looks_like_llm_request(&frames.body),
    })
}

pub(super) fn classify_response(bytes: &[u8]) -> Option<ClassifiedMessage> {
    let frames = frames(bytes)?;
    if frames.body.is_empty() && !frames.saw_http_frame {
        return None;
    }
    Some(ClassifiedMessage {
        stream_id: frames.stream_id,
        llm: body_looks_like_llm_response(&frames.body),
    })
}

pub(super) fn candidate_stream_id(bytes: &[u8]) -> Option<Option<u32>> {
    if bytes.starts_with(CONNECTION_PREFACE) {
        return Some(None);
    }
    known_frame_stream_id(bytes).map(|stream_id| {
        if stream_id == 0 {
            None
        } else {
            Some(stream_id)
        }
    })
}

pub(super) fn body_looks_like_llm_request(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    text.contains("\"model\"")
        && (text.contains("\"messages\"")
            || text.contains("\"prompt\"")
            || text.contains("\"input\""))
}

pub(super) fn body_looks_like_llm_response(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    text.contains("message_stop")
        || text.contains("[done]")
        || (text.contains("\"model\"")
            && (text.contains("\"choices\"")
                || text.contains("\"content\"")
                || text.contains("\"output\"")))
}

struct FrameBodies {
    stream_id: Option<u32>,
    body: Vec<u8>,
    saw_http_frame: bool,
}

fn frames(bytes: &[u8]) -> Option<FrameBodies> {
    let mut cursor = if bytes.starts_with(CONNECTION_PREFACE) {
        CONNECTION_PREFACE.len()
    } else {
        0
    };
    let mut body = Vec::new();
    let mut stream_id = None;
    let mut saw_http_frame = false;
    while cursor + FRAME_HEADER_BYTES <= bytes.len() {
        let Some(frame) = decode_frame(&bytes[cursor..]) else {
            return None;
        };
        if frame.stream_id != 0 && stream_id.is_none() {
            stream_id = Some(frame.stream_id);
        }
        if matches!(
            frame.frame_type,
            DATA_FRAME_TYPE | HEADERS_FRAME_TYPE | CONTINUATION_FRAME_TYPE
        ) {
            saw_http_frame = true;
        }
        if frame.frame_type == DATA_FRAME_TYPE
            && let Some(data) = data_payload(frame.flags, frame.payload)
        {
            body.extend_from_slice(data);
        }
        cursor += frame.encoded_len;
    }
    (cursor > 0 || bytes.starts_with(CONNECTION_PREFACE)).then_some(FrameBodies {
        stream_id,
        body,
        saw_http_frame,
    })
}

struct Frame<'a> {
    frame_type: u8,
    flags: u8,
    stream_id: u32,
    payload: &'a [u8],
    encoded_len: usize,
}

fn decode_frame(bytes: &[u8]) -> Option<Frame<'_>> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return None;
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    let encoded_len = FRAME_HEADER_BYTES.checked_add(length)?;
    if bytes.len() < encoded_len {
        return None;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    if !stream_id_is_valid(frame_type, stream_id) {
        return None;
    }
    Some(Frame {
        frame_type,
        flags: bytes[4],
        stream_id,
        payload: &bytes[FRAME_HEADER_BYTES..encoded_len],
        encoded_len,
    })
}

fn known_frame_stream_id(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return None;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    stream_id_is_valid(frame_type, stream_id).then_some(stream_id)
}

fn stream_id_is_valid(frame_type: u8, stream_id: u32) -> bool {
    match frame_type {
        0x0 | 0x1 | 0x2 | 0x3 | 0x5 | 0x9 => stream_id != 0,
        0x4 | 0x6 | 0x7 => stream_id == 0,
        0x8 => true,
        _ => false,
    }
}

fn data_payload(flags: u8, payload: &[u8]) -> Option<&[u8]> {
    let mut cursor = 0usize;
    let mut data_end = payload.len();
    if flags & FLAG_PADDED != 0 {
        let padding = usize::from(*payload.first()?);
        cursor = cursor.checked_add(1)?;
        data_end = data_end.checked_sub(padding)?;
    }
    if cursor <= data_end && data_end <= payload.len() {
        Some(&payload[cursor..data_end])
    } else {
        None
    }
}
