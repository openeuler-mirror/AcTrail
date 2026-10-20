use std::io::{self, Write};
use std::sync::Arc;

use super::super::super::bounded_body::BoundedBody;
use super::super::super::decoder::Decoder;
use super::constant::BUFFER_SIZE;

pub(in crate::llm_pipeline::transport::http1::content_decoder) struct BrotliDecoder {
    inner: brotli::DecompressorWriter<BoundedBody>,
}

impl BrotliDecoder {
    pub(in crate::llm_pipeline::transport::http1::content_decoder) fn new(
        body: BoundedBody,
    ) -> Self {
        Self {
            inner: brotli::DecompressorWriter::new(body, BUFFER_SIZE),
        }
    }
}

impl Decoder for BrotliDecoder {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)
    }

    fn finish(&mut self) -> io::Result<()> {
        self.inner.close()
    }

    fn snapshot(&self) -> Arc<Vec<u8>> {
        self.inner.get_ref().snapshot()
    }
}
