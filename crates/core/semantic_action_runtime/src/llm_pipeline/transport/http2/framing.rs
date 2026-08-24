//! HTTP/2 frame-boundary decoding used before logical-stream demultiplexing.

pub(crate) const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
pub(crate) const HTTP2_DATA_FRAME_TYPE: u8 = 0x0;
pub(crate) const HTTP2_HEADERS_FRAME_TYPE: u8 = 0x1;
pub(crate) const HTTP2_RST_STREAM_FRAME_TYPE: u8 = 0x3;
pub(crate) const HTTP2_CONTINUATION_FRAME_TYPE: u8 = 0x9;
pub(crate) const HTTP2_FLAG_END_STREAM: u8 = 0x1;
pub(crate) const HTTP2_FLAG_PADDED: u8 = 0x8;
pub(crate) const HTTP2_FRAME_HEADER_BYTES: usize = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Http2Frame<'a> {
    pub(crate) frame_type: u8,
    pub(crate) flags: u8,
    pub(crate) stream_id: u32,
    pub(crate) payload: &'a [u8],
    pub(crate) encoded_len: usize,
}

pub(in crate::llm_pipeline) enum Http2FrameDecode<'a> {
    NeedMore,
    Invalid { encoded_len: usize, stream_id: u32 },
    Frame(Http2Frame<'a>),
}

pub(crate) fn decode_http2_frame(bytes: &[u8]) -> Option<Http2Frame<'_>> {
    match decode_http2_frame_state(bytes) {
        Http2FrameDecode::Frame(frame) => Some(frame),
        Http2FrameDecode::NeedMore | Http2FrameDecode::Invalid { .. } => None,
    }
}

pub(in crate::llm_pipeline) fn decode_http2_frame_state(bytes: &[u8]) -> Http2FrameDecode<'_> {
    if bytes.len() < HTTP2_FRAME_HEADER_BYTES {
        return Http2FrameDecode::NeedMore;
    }
    let length =
        (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
    let encoded_len = HTTP2_FRAME_HEADER_BYTES + length;
    if bytes.len() < encoded_len {
        return Http2FrameDecode::NeedMore;
    }
    let frame_type = bytes[3];
    let stream_id = (u32::from(bytes[5] & 0x7f) << 24)
        | (u32::from(bytes[6]) << 16)
        | (u32::from(bytes[7]) << 8)
        | u32::from(bytes[8]);
    if !stream_id_is_valid(frame_type, stream_id) {
        return Http2FrameDecode::Invalid {
            encoded_len,
            stream_id,
        };
    }
    Http2FrameDecode::Frame(Http2Frame {
        frame_type,
        flags: bytes[4],
        stream_id,
        payload: &bytes[HTTP2_FRAME_HEADER_BYTES..encoded_len],
        encoded_len,
    })
}

pub(crate) fn data_payload(flags: u8, payload: &[u8]) -> Option<&[u8]> {
    strip_padding(flags, payload, 0)
}

fn stream_id_is_valid(frame_type: u8, stream_id: u32) -> bool {
    match frame_type {
        0x0 | 0x1 | 0x2 | 0x3 | 0x5 | 0x9 => stream_id != 0,
        0x4 | 0x6 | 0x7 => stream_id == 0,
        0x8 => true,
        // Extension frames define their own stream semantics. Their complete
        // payload can be skipped without blocking later standard frames.
        _ => true,
    }
}

fn strip_padding(flags: u8, payload: &[u8], prefix_without_padding: usize) -> Option<&[u8]> {
    let mut start = 0;
    let mut end = payload.len();
    if flags & HTTP2_FLAG_PADDED != 0 {
        let padding = usize::from(*payload.first()?);
        start = 1;
        if padding > end.saturating_sub(start) {
            return None;
        }
        end -= padding;
    }
    start = start.checked_add(prefix_without_padding)?;
    (start <= end).then(|| &payload[start..end])
}
