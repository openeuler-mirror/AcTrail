use std::fmt::Write as _;
use std::io::{self, Write as _};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn bytes(value: &Value) -> Vec<u8> {
    let mut writer = CanonicalJsonWriter::new(u64::MAX);
    writer
        .write_value(value)
        .expect("unbounded canonical JSON serialization cannot exceed its limit");
    writer.output
}

pub(super) fn string(value: &Value) -> String {
    CanonicalJsonWriter::new(u64::MAX)
        .serialize(value)
        .expect("unbounded canonical JSON serialization cannot exceed its limit")
}

pub(in crate::llm_pipeline) struct CanonicalJsonWriter {
    output: Vec<u8>,
    max_bytes: u64,
}

impl CanonicalJsonWriter {
    pub(in crate::llm_pipeline) fn new(max_bytes: u64) -> Self {
        Self {
            output: Vec::new(),
            max_bytes,
        }
    }

    pub(in crate::llm_pipeline) fn serialize(mut self, value: &Value) -> io::Result<String> {
        self.write_value(value)?;
        Ok(String::from_utf8(self.output).expect("canonical JSON serialization emits valid UTF-8"))
    }

    fn write_value(&mut self, value: &Value) -> io::Result<()> {
        match value {
            Value::Null => self.write_all(b"null"),
            Value::Bool(value) => self.write_all(if *value { b"true" } else { b"false" }),
            Value::Number(value) => self.write_all(value.to_string().as_bytes()),
            Value::String(value) => self.write_string(value),
            Value::Array(values) => {
                self.require_remaining((values.len() as u64).saturating_mul(2).saturating_add(1))?;
                self.write_all(b"[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    self.write_value(value)?;
                }
                self.write_all(b"]")
            }
            Value::Object(object) => {
                self.require_remaining((object.len() as u64).saturating_mul(5).saturating_add(1))?;
                self.write_all(b"{")?;
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    self.write_string(key)?;
                    self.write_all(b":")?;
                    self.write_value(&object[key])?;
                }
                self.write_all(b"}")
            }
        }
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        // Reject oversized raw strings before serde scans them for escaping.
        self.require_remaining((value.len() as u64).saturating_add(2))?;
        serde_json::to_writer(self, value).map_err(io::Error::other)
    }

    fn require_remaining(&self, bytes: u64) -> io::Result<()> {
        if bytes > self.max_bytes.saturating_sub(self.output.len() as u64) {
            return Err(io::Error::other(
                "canonical JSON exceeds configured byte limit",
            ));
        }
        Ok(())
    }
}

impl io::Write for CanonicalJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.require_remaining(bytes.len() as u64)?;
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
