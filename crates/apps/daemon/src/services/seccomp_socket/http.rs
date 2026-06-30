//! HTTP/1.x socket fallback admission helpers.

pub(super) const HTTP1_PROTOCOL_HINT: &str = "http/1.x";

pub(super) struct HttpBodyAdmission {
    pub(super) content_length: u64,
    pub(super) header_len: u64,
    pub(super) body_bytes_in_buffer: u64,
}

pub(super) fn content_length_admission(bytes: &[u8]) -> Option<HttpBodyAdmission> {
    let header_len = header_len(bytes)?;
    let header = std::str::from_utf8(bytes.get(..header_len)?).ok()?;
    let mut lines = header.lines();
    let first_line = lines.next()?.trim_end_matches('\r').trim();
    if !is_request_line(first_line) {
        return None;
    }

    let mut content_length = None;
    for raw_line in lines {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return None;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return None;
            }
            let parsed = value.parse::<u64>().ok()?;
            if parsed == 0 {
                return None;
            }
            content_length = Some(parsed);
        }
    }

    let content_length = content_length?;
    let header_len_u64 = u64::try_from(header_len).ok()?;
    let buffer_len_u64 = u64::try_from(bytes.len()).ok()?;
    let body_bytes_in_buffer = buffer_len_u64
        .saturating_sub(header_len_u64)
        .min(content_length);
    Some(HttpBodyAdmission {
        content_length,
        header_len: header_len_u64,
        body_bytes_in_buffer,
    })
}

fn header_len(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn is_request_line(line: &str) -> bool {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    parts.len() == 3 && parts[2].starts_with("HTTP/")
}
