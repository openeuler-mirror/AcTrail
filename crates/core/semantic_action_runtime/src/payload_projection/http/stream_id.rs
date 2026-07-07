use super::{
    HTTP1_REQUEST_METHODS, HTTP2_CONNECTION_PREFACE, HTTP2_FRAME_HEADER_BYTES,
    http2_stream_id_is_valid, split_http1_request,
};

pub(crate) fn request_stream_id_hint(bytes: &[u8]) -> Option<Option<u32>> {
    if split_http1_request(bytes).is_some() || looks_like_http1_request_prefix(bytes) {
        return Some(None);
    }
    http2_stream_id_hint(bytes).map(Some)
}

fn looks_like_http1_request_prefix(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some(first_line) = text.split("\r\n").next() else {
        return false;
    };
    HTTP1_REQUEST_METHODS.iter().any(|method| {
        first_line == *method
            || first_line
                .strip_prefix(method)
                .is_some_and(|tail| tail.starts_with(' '))
    })
}

fn http2_stream_id_hint(bytes: &[u8]) -> Option<u32> {
    let mut cursor = if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        HTTP2_CONNECTION_PREFACE.len()
    } else {
        0
    };
    loop {
        let header = bytes.get(cursor..cursor.checked_add(HTTP2_FRAME_HEADER_BYTES)?)?;
        let length =
            (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
        let frame_type = header[3];
        let stream_id = (u32::from(header[5] & 0x7f) << 24)
            | (u32::from(header[6]) << 16)
            | (u32::from(header[7]) << 8)
            | u32::from(header[8]);
        if !http2_stream_id_is_valid(frame_type, stream_id) {
            return None;
        }
        if stream_id != 0 {
            return Some(stream_id);
        }
        let encoded_len = HTTP2_FRAME_HEADER_BYTES.checked_add(length)?;
        let next_cursor = cursor.checked_add(encoded_len)?;
        if bytes.len() < next_cursor {
            return None;
        }
        cursor = next_cursor;
    }
}
