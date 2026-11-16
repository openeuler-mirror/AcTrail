use std::fmt::Write as _;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::llm_pipeline::utils::CanonicalJsonWriter;

pub(super) fn bytes(value: &Value) -> Vec<u8> {
    CanonicalJsonWriter::new(u64::MAX)
        .serialize_bytes(value)
        .expect("unbounded canonical JSON serialization cannot exceed its limit")
}

pub(super) fn string(value: &Value) -> String {
    CanonicalJsonWriter::new(u64::MAX)
        .serialize(value)
        .expect("unbounded canonical JSON serialization cannot exceed its limit")
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity("sha256:".len() + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}
