use std::io::{self, Write};

use super::super::bounded_body::BoundedBody;

pub(in crate::llm_pipeline::transport::http1::content_decoder) struct IdentityDecoder;

static IDENTITY: IdentityDecoder = IdentityDecoder;

impl IdentityDecoder {
    pub(in crate::llm_pipeline::transport::http1::content_decoder) fn shared() -> &'static Self {
        &IDENTITY
    }

    pub(in crate::llm_pipeline::transport::http1::content_decoder) fn append(
        &self,
        bytes: &[u8],
        body: &mut BoundedBody,
    ) -> io::Result<()> {
        body.write_all(bytes)
    }
}
