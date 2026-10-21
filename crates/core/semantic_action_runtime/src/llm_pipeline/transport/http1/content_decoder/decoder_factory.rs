use super::super::decoder::Http1DecodeFailure;
use super::bounded_body::BoundedBody;
use super::constant::DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING;
use super::decoder_handle::DecoderHandle;
use super::implementations::{BrotliDecoder, GzipDecoder, IdentityDecoder, ZstdDecoder};

pub(in crate::llm_pipeline::transport::http1) struct DecoderFactory;

impl DecoderFactory {
    pub(in crate::llm_pipeline::transport::http1) fn create(
        headers: &str,
        max_bytes: usize,
    ) -> Result<DecoderHandle, Http1DecodeFailure> {
        let mut codings = headers.split("\r\n").filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-encoding")
                .then_some(value.trim())
        });
        let coding = codings
            .next()
            .unwrap_or(DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING);
        if codings.next().is_some() {
            return Err(Http1DecodeFailure::InvalidContentEncoding);
        }
        let body = BoundedBody::new(max_bytes);
        if coding.eq_ignore_ascii_case("identity") {
            Ok(DecoderHandle::identity(IdentityDecoder::shared(), body))
        } else if coding.eq_ignore_ascii_case("br") {
            Ok(DecoderHandle::stateful(Box::new(BrotliDecoder::new(body))))
        } else if coding.eq_ignore_ascii_case("gzip") {
            Ok(DecoderHandle::stateful(Box::new(GzipDecoder::new(body))))
        } else if coding.eq_ignore_ascii_case("zstd") {
            Ok(DecoderHandle::stateful(Box::new(
                ZstdDecoder::new(body).map_err(DecoderHandle::failure)?,
            )))
        } else {
            Err(Http1DecodeFailure::InvalidContentEncoding)
        }
    }
}
