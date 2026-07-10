pub(super) fn body_looks_text_api(bytes: &[u8]) -> bool {
    let bytes = trim_ascii(bytes);
    if bytes.is_empty() {
        return true;
    }
    bytes.starts_with(b"{")
        || bytes.starts_with(b"[")
        || bytes.starts_with(b"data:")
        || bytes.starts_with(b"event:")
        || text_ratio(bytes) > 900
}

pub(super) fn body_looks_binary(bytes: &[u8]) -> bool {
    let bytes = trim_ascii(bytes);
    if bytes.is_empty() {
        return false;
    }
    if bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"\x1f\x8b")
        || bytes.starts_with(b"\x28\xb5\x2f\xfd")
        || bytes.starts_with(b"\0asm")
        || bytes.starts_with(b"%PDF")
    {
        return true;
    }
    text_ratio(bytes) < 700
}

pub(super) fn text_ratio(bytes: &[u8]) -> u16 {
    let sample = &bytes[..bytes.len().min(4096)];
    if sample.is_empty() {
        return 1000;
    }
    let text = sample
        .iter()
        .filter(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace())
        .count();
    ((text * 1000) / sample.len()) as u16
}

pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(super) fn find_byte(bytes: &[u8], needle: u8) -> Option<usize> {
    bytes.iter().position(|byte| *byte == needle)
}

pub(super) fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

pub(super) fn split_once_space(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|byte| byte.is_ascii_whitespace())?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

pub(super) fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

pub(super) fn ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}
