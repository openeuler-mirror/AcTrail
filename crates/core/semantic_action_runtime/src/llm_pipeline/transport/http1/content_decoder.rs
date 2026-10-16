//! Incremental HTTP content decoding after transfer framing.

use std::io::{self, Write};
use std::sync::Arc;

use super::decoder::Http1DecodeFailure;

pub(super) enum Http1ContentDecoder {
    Identity(BoundedBody),
    Brotli(Box<brotli::DecompressorWriter<BoundedBody>>),
}

impl Http1ContentDecoder {
    pub(super) fn new(headers: &str, max_bytes: usize) -> Result<Self, Http1DecodeFailure> {
        let mut codings = headers.split("\r\n").filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-encoding")
                .then_some(value.trim())
        });
        let coding = codings.next().unwrap_or("identity");
        if codings.next().is_some() {
            return Err(Http1DecodeFailure::InvalidContentEncoding);
        }
        let body = BoundedBody {
            bytes: Arc::new(Vec::new()),
            max_bytes,
        };
        if coding.eq_ignore_ascii_case("identity") {
            Ok(Self::Identity(body))
        } else if coding.eq_ignore_ascii_case("br") {
            // Use Brotli's standard scratch buffer; retained output is bounded
            // independently by the operator's HTTP assembly byte limit.
            Ok(Self::Brotli(Box::new(brotli::DecompressorWriter::new(
                body, 0,
            ))))
        } else {
            Err(Http1DecodeFailure::InvalidContentEncoding)
        }
    }

    pub(super) fn is_identity(&self) -> bool {
        matches!(self, Self::Identity(_))
    }

    pub(super) fn append(&mut self, bytes: &[u8]) -> Result<(), Http1DecodeFailure> {
        let result = match self {
            Self::Identity(body) => body.write_all(bytes),
            Self::Brotli(decoder) => decoder.write_all(bytes),
        };
        result.map_err(Self::failure)
    }

    pub(super) fn finish(&mut self) -> Result<(), Http1DecodeFailure> {
        match self {
            Self::Identity(_) => Ok(()),
            Self::Brotli(decoder) => decoder.close().map_err(Self::failure),
        }
    }

    pub(super) fn snapshot(&self) -> Arc<Vec<u8>> {
        let body = match self {
            Self::Identity(body) => body,
            Self::Brotli(decoder) => decoder.get_ref(),
        };
        Arc::clone(&body.bytes)
    }

    pub(super) fn clear_identity(&mut self) {
        if let Self::Identity(body) = self {
            Arc::make_mut(&mut body.bytes).clear();
        }
    }

    fn failure(error: io::Error) -> Http1DecodeFailure {
        if error.kind() == io::ErrorKind::OutOfMemory {
            Http1DecodeFailure::BufferCapacity
        } else {
            Http1DecodeFailure::InvalidContentEncoding
        }
    }
}

pub(super) struct BoundedBody {
    bytes: Arc<Vec<u8>>,
    max_bytes: usize,
}

impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > self.max_bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "decoded HTTP body exceeds assembly limit",
            ));
        }
        Arc::make_mut(&mut self.bytes).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
