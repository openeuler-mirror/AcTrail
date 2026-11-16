use super::ContentDecodeFailure;
use super::bounded_body::BoundedBody;
use super::constant::DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING;
use super::decoder_handle::DecoderHandle;
use super::implementations::{BrotliDecoder, GzipDecoder, IdentityDecoder, ZstdDecoder};

pub(in crate::llm_pipeline) struct DecoderFactory;

impl DecoderFactory {
    pub(in crate::llm_pipeline) fn create(
        headers: &str,
        max_bytes: usize,
    ) -> Result<DecoderHandle, ContentDecodeFailure> {
        let mut codings = headers.split("\r\n").filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-encoding")
                .then_some(value.trim())
        });
        let coding = codings
            .next()
            .unwrap_or(DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING);
        if codings.next().is_some() {
            return Err(ContentDecodeFailure::InvalidContentEncoding);
        }
        Self::for_encoding(Some(coding), max_bytes)
    }

    pub(in crate::llm_pipeline) fn for_encoding(
        coding: Option<&str>,
        max_bytes: usize,
    ) -> Result<DecoderHandle, ContentDecodeFailure> {
        let coding = coding.unwrap_or(DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING);
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
            Err(ContentDecodeFailure::InvalidContentEncoding)
        }
    }
}
