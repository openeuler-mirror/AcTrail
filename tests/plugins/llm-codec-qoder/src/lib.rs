use std::slice;

const QODER_ALPHABET: &[u8; 64] =
    b"_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!";
const NO_MATCH: &[u8] = br#"{"status":"no_match"}"#;
const REQUEST_PREFIX: &[u8] = br#"{"status":"decoded","classifier_id":"qoder-infer","protocol_id":"qoder-infer","model":"auto","body":["#;
const SSE_PREFIX: &[u8] = br#"{"status":"decoded","provider_id":"qoder-infer","body":["#;
const SUFFIX: &[u8] = b"]}";

#[unsafe(no_mangle)]
pub extern "C" fn actrail_alloc(len: usize) -> *mut u8 {
    let mut bytes = Vec::<u8>::with_capacity(len);
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn actrail_llm_codec_decode_request(ptr: *const u8, len: usize) -> u64 {
    let input = unsafe { slice::from_raw_parts(ptr, len) };
    match decode_qoder_request(input) {
        Some(decoded) if looks_like_qoder_request(&decoded) => {
            pack_vec(decoded_output(REQUEST_PREFIX, &decoded))
        }
        _ => pack_static(NO_MATCH),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn actrail_llm_codec_decode_sse_event(ptr: *const u8, len: usize) -> u64 {
    let input = unsafe { slice::from_raw_parts(ptr, len) };
    match unwrap_qoder_sse(input) {
        Some(decoded) => pack_vec(decoded_output(SSE_PREFIX, &decoded)),
        None => pack_static(NO_MATCH),
    }
}

fn decode_qoder_request(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() || input.len() % 4 != 0 {
        return None;
    }
    let n = input.len();
    let first_len = n / 3;
    let second_end = (n * 2).div_ceil(3);
    let third_len = n - second_end;
    let second_len = second_end - first_len;
    let mut custom = Vec::with_capacity(n);
    custom.extend_from_slice(&input[third_len + second_len..]);
    custom.extend_from_slice(&input[third_len..third_len + second_len]);
    custom.extend_from_slice(&input[..third_len]);
    base64_decode_qoder(&custom)
}

fn base64_decode_qoder(input: &[u8]) -> Option<Vec<u8>> {
    let alphabet = alphabet_map();
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    for chunk in input.chunks_exact(4) {
        let pad = usize::from(chunk[2] == b'$') + usize::from(chunk[3] == b'$');
        if pad > 0 && (chunk[3] != b'$' || (pad == 2 && chunk[2] != b'$')) {
            return None;
        }
        let a = sextet(&alphabet, chunk[0])?;
        let b = sextet(&alphabet, chunk[1])?;
        let c = if chunk[2] == b'$' {
            0
        } else {
            sextet(&alphabet, chunk[2])?
        };
        let d = if chunk[3] == b'$' {
            0
        } else {
            sextet(&alphabet, chunk[3])?
        };
        out.push((a << 2) | (b >> 4));
        if pad < 2 {
            out.push((b << 4) | (c >> 2));
        }
        if pad == 0 {
            out.push((c << 6) | d);
        }
    }
    Some(out)
}

fn alphabet_map() -> [u8; 256] {
    let mut map = [255u8; 256];
    for (index, byte) in QODER_ALPHABET.iter().enumerate() {
        map[*byte as usize] = index as u8;
    }
    map
}

fn sextet(alphabet: &[u8; 256], byte: u8) -> Option<u8> {
    let value = alphabet[byte as usize];
    (value != 255).then_some(value)
}

fn looks_like_qoder_request(decoded: &[u8]) -> bool {
    contains(decoded, br#""model_config""#) && contains(decoded, br#""messages""#)
}

fn unwrap_qoder_sse(input: &[u8]) -> Option<Vec<u8>> {
    if input == b"[DONE]" {
        return Some(input.to_vec());
    }
    if !contains(input, br#""statusCodeValue":200"#) {
        return None;
    }
    let body_key = br#""body":"#;
    let body_start = find(input, body_key)? + body_key.len();
    if input.get(body_start) != Some(&b'"') {
        return None;
    }
    let mut cursor = body_start + 1;
    let mut out = Vec::new();
    while cursor < input.len() {
        let byte = input[cursor];
        if byte == b'"' {
            return Some(out);
        }
        if byte != b'\\' {
            out.push(byte);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let escaped = *input.get(cursor)?;
        match escaped {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'/' => out.push(b'/'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => return None,
            other => out.push(other),
        }
        cursor += 1;
    }
    None
}

fn decoded_output(prefix: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + body.len() * 4 + SUFFIX.len());
    out.extend_from_slice(prefix);
    for (index, byte) in body.iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        push_decimal(&mut out, *byte);
    }
    out.extend_from_slice(SUFFIX);
    out
}

fn push_decimal(out: &mut Vec<u8>, byte: u8) {
    if byte >= 100 {
        out.push(b'0' + byte / 100);
        out.push(b'0' + (byte / 10) % 10);
        out.push(b'0' + byte % 10);
    } else if byte >= 10 {
        out.push(b'0' + byte / 10);
        out.push(b'0' + byte % 10);
    } else {
        out.push(b'0' + byte);
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn pack_static(bytes: &'static [u8]) -> u64 {
    ((bytes.as_ptr() as u64) << 32) | bytes.len() as u64
}

fn pack_vec(mut bytes: Vec<u8>) -> u64 {
    let ptr = bytes.as_mut_ptr();
    let len = bytes.len();
    std::mem::forget(bytes);
    ((ptr as u64) << 32) | len as u64
}
