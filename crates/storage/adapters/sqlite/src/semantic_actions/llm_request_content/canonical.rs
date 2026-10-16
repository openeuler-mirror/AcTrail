use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, Write};

use super::read::{BlockMeta, ContentReader};

pub(super) trait ContentOutput: Write {
    fn block(&mut self, meta: &BlockMeta) -> io::Result<()>;
}

pub(super) struct CanonicalTraversal<'a> {
    blocks: &'a BTreeMap<u32, BlockMeta>,
}

impl<'a> CanonicalTraversal<'a> {
    pub(super) fn new(blocks: &'a BTreeMap<u32, BlockMeta>) -> Self {
        Self { blocks }
    }

    pub(super) fn write(&self, value: &Value, output: &mut impl ContentOutput) -> io::Result<()> {
        if let Some(ordinal) = ContentReader::placeholder(value) {
            let block = self.blocks.get(&ordinal).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "skeleton references missing block ordinal",
                )
            })?;
            return output.block(block);
        }
        match value {
            Value::Null => output.write_all(b"null"),
            Value::Bool(value) => output.write_all(if *value { b"true" } else { b"false" }),
            Value::Number(value) => output.write_all(value.to_string().as_bytes()),
            Value::String(value) => Self::string(value, output),
            Value::Array(values) => {
                output.write_all(b"[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.write_all(b",")?;
                    }
                    self.write(value, output)?;
                }
                output.write_all(b"]")
            }
            Value::Object(values) => {
                output.write_all(b"{")?;
                let mut keys: Vec<_> = values.keys().collect();
                keys.sort_unstable();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.write_all(b",")?;
                    }
                    Self::string(key, output)?;
                    output.write_all(b":")?;
                    self.write(&values[key], output)?;
                }
                output.write_all(b"}")
            }
        }
    }

    fn string(value: &str, output: &mut impl Write) -> io::Result<()> {
        serde_json::to_writer(output, value).map_err(|error| {
            io::Error::new(error.io_error_kind().unwrap_or(io::ErrorKind::Other), error)
        })
    }
}

#[derive(Default)]
pub(super) struct LengthWriter {
    bytes: u64,
}

impl LengthWriter {
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
    fn add(&mut self, bytes: u64) -> io::Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "canonical length overflow")
        })?;
        Ok(())
    }
}

impl Write for LengthWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.add(bytes.len() as u64)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ContentOutput for LengthWriter {
    fn block(&mut self, meta: &BlockMeta) -> io::Result<()> {
        self.add(meta.bytes)
    }
}
