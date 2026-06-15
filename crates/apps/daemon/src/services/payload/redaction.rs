use config_core::daemon::PayloadRedactionPolicy;
use model_core::payload::PayloadRedactionState;

pub(super) fn redact_payload_bytes(
    policy: PayloadRedactionPolicy,
    bytes: Vec<u8>,
) -> (Vec<u8>, PayloadRedactionState) {
    match policy {
        PayloadRedactionPolicy::Disabled => (bytes, PayloadRedactionState::Unredacted),
        PayloadRedactionPolicy::AuthorizationHeader => redact_authorization_header(bytes),
    }
}

fn redact_authorization_header(bytes: Vec<u8>) -> (Vec<u8>, PayloadRedactionState) {
    let mut output = Vec::with_capacity(bytes.len());
    let mut changed = false;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let without_newline = line.strip_suffix(b"\n").unwrap_or(line);
        let without_crlf = without_newline
            .strip_suffix(b"\r")
            .unwrap_or(without_newline);
        if starts_with_ignore_ascii_case(without_crlf, b"authorization:") {
            output.extend_from_slice(b"Authorization: <redacted>");
            if line.ends_with(b"\r\n") {
                output.extend_from_slice(b"\r\n");
            } else if line.ends_with(b"\n") {
                output.push(b'\n');
            }
            changed = true;
        } else {
            output.extend_from_slice(line);
        }
    }

    if changed {
        (output, PayloadRedactionState::Redacted)
    } else {
        (output, PayloadRedactionState::Unredacted)
    }
}

fn starts_with_ignore_ascii_case(value: &[u8], prefix: &[u8]) -> bool {
    value.len() >= prefix.len()
        && value[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}
