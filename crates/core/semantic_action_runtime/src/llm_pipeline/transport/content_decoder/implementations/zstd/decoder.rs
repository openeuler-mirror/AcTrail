use std::io::{self, Write};
use std::sync::Arc;

use super::super::super::bounded_body::BoundedBody;
use super::super::super::decoder::Decoder;

pub(in crate::llm_pipeline::transport::content_decoder) struct ZstdDecoder {
    inner: zstd::stream::zio::Writer<BoundedBody, zstd::stream::raw::Decoder<'static>>,
}

impl ZstdDecoder {
    pub(in crate::llm_pipeline::transport::content_decoder) fn new(
        body: BoundedBody,
    ) -> io::Result<Self> {
        Ok(Self {
            inner: zstd::stream::zio::Writer::new(body, zstd::stream::raw::Decoder::new()?),
        })
    }
}

impl Decoder for ZstdDecoder {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)?;
        // The zstd writer buffers decompressed output internally; flush after
        // every append so snapshots and the byte bound stay current.
        self.inner.flush()
    }

    fn finish(&mut self) -> io::Result<()> {
        // Finalization also rejects frames still waiting for compressed input.
        self.inner.finish()
    }

    fn snapshot(&self) -> Arc<Vec<u8>> {
        self.inner.writer().snapshot()
    }
}
