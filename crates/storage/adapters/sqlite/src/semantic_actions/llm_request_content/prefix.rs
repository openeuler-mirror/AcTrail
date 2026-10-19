use std::io::{self, Read, Write};

use semantic_action::{LlmRequestContentPage, SemanticActionStoreError};

use super::{
    canonical::{CanonicalTraversal, ContentOutput},
    read::{BlockMeta, ContentReader},
};

impl ContentReader<'_> {
    pub(in crate::semantic_actions) fn prefix(
        &self,
        max_bytes: usize,
    ) -> Result<LlmRequestContentPage, SemanticActionStoreError> {
        let mut output = PrefixWriter {
            reader: self,
            bytes: Vec::new(),
            limit: max_bytes,
        };
        if let Err(error) = CanonicalTraversal::new(&self.blocks).write(&self.skeleton, &mut output)
        {
            if error.kind() != io::ErrorKind::WriteZero {
                return Err(Self::error(error));
            }
        }
        // The byte budget may end inside a UTF-8 scalar. Remove only the incomplete
        // suffix; malformed bytes within the returned prefix remain a read error.
        let mut bytes = output.bytes;
        if let Err(error) = std::str::from_utf8(&bytes) {
            if error.error_len().is_some() {
                return Err(Self::error(error));
            }
            bytes.truncate(error.valid_up_to());
        }
        let body_json = String::from_utf8(bytes).map_err(Self::error)?;
        Ok(LlmRequestContentPage {
            trace_id: self.trace_id,
            action_id: self.action_id.clone(),
            format_version: self.format_version,
            canonical_body_bytes: self.canonical_body_bytes,
            returned_bytes: body_json.len() as u64,
            truncated: (body_json.len() as u64) < self.canonical_body_bytes,
            body_json,
        })
    }
}

struct PrefixWriter<'r, 'c> {
    reader: &'r ContentReader<'c>,
    bytes: Vec<u8>,
    limit: usize,
}

impl PrefixWriter<'_, '_> {
    fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.bytes.len())
    }
    fn complete() -> io::Error {
        io::Error::new(io::ErrorKind::WriteZero, "canonical prefix complete")
    }
}

impl Write for PrefixWriter<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = bytes.len().min(self.remaining());
        if count == 0 {
            return Err(Self::complete());
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ContentOutput for PrefixWriter<'_, '_> {
    fn block(&mut self, meta: &BlockMeta) -> io::Result<()> {
        if self.remaining() == 0 {
            return Err(Self::complete());
        }
        let count = self
            .remaining()
            .min(usize::try_from(meta.bytes).map_err(io::Error::other)?);
        let mut blob = self
            .reader
            .blob(meta)
            .map_err(|error| io::Error::other(error.message))?;
        let start = self.bytes.len();
        self.bytes.resize(start + count, 0);
        blob.read_exact(&mut self.bytes[start..])?;
        if (count as u64) < meta.bytes {
            return Err(Self::complete());
        }
        Ok(())
    }
}
