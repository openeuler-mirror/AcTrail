use std::io::{self, Write};
use std::sync::Arc;

use super::super::super::bounded_body::BoundedBody;
use super::super::super::decoder::Decoder;

pub(in crate::llm_pipeline::transport::http1::content_decoder) struct GzipDecoder {
    inner: flate2::write::MultiGzDecoder<BoundedBody>,
}

impl GzipDecoder {
    pub(in crate::llm_pipeline::transport::http1::content_decoder) fn new(
        body: BoundedBody,
    ) -> Self {
        Self {
            inner: flate2::write::MultiGzDecoder::new(body),
        }
    }
}

impl Decoder for GzipDecoder {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)?;
        self.inner.flush()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.inner.try_finish()
    }

    fn snapshot(&self) -> Arc<Vec<u8>> {
        self.inner.get_ref().snapshot()
    }
}
