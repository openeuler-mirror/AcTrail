use serde_json::Value;
use sha2::{Digest, Sha256};

const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

pub(super) struct CanonicalJson;

impl CanonicalJson {
    pub(super) fn validate_jsonrpc(bytes: &[u8]) -> Result<(), String> {
        let value = serde_json::from_slice::<Value>(bytes)
            .map_err(|error| format!("canonical JSON-RPC is invalid JSON: {error}"))?;
        if !value.is_object() || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("canonical JSON-RPC must be a JSON-RPC 2.0 object".to_string());
        }
        let mut canonical = Vec::new();
        Self::encode_value(&value, &mut canonical);
        if canonical != bytes {
            return Err(
                "canonical JSON-RPC must use recursively sorted object keys and compact UTF-8 JSON"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn digest(bytes: &[u8]) -> Vec<u8> {
        Sha256::digest(bytes).to_vec()
    }

    pub(super) fn parse_hash(value: &str) -> Result<Vec<u8>, String> {
        let Some(hex) = value.strip_prefix(SHA256_PREFIX) else {
            return Err("SHA-256 hash must start with sha256:".to_string());
        };
        if hex.len() != SHA256_HEX_LEN || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("SHA-256 hash must contain exactly 64 hexadecimal digits".to_string());
        }
        (0..SHA256_HEX_LEN)
            .step_by(2)
            .map(|index| {
                u8::from_str_radix(&hex[index..index + 2], 16)
                    .map_err(|error| format!("invalid SHA-256 hash: {error}"))
            })
            .collect()
    }

    pub(super) fn hash_text(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(SHA256_PREFIX.len() + bytes.len() * 2);
        text.push_str(SHA256_PREFIX);
        for byte in bytes {
            use std::fmt::Write;
            write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
        }
        text
    }

    fn encode_value(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null => output.extend_from_slice(b"null"),
            Value::Bool(true) => output.extend_from_slice(b"true"),
            Value::Bool(false) => output.extend_from_slice(b"false"),
            Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
            Value::String(value) => output.extend_from_slice(
                serde_json::to_string(value)
                    .expect("serde_json strings are always serializable")
                    .as_bytes(),
            ),
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    Self::encode_value(value, output);
                }
                output.push(b']');
            }
            Value::Object(values) => {
                output.push(b'{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend_from_slice(
                        serde_json::to_string(key)
                            .expect("serde_json object keys are always serializable")
                            .as_bytes(),
                    );
                    output.push(b':');
                    Self::encode_value(value, output);
                }
                output.push(b'}');
            }
        }
    }
}
