use std::io;
use std::sync::Arc;

use super::super::decoder::Http1DecodeFailure;
use super::bounded_body::BoundedBody;
use super::decoder::Decoder;
use super::implementations::IdentityDecoder;

pub(in crate::llm_pipeline::transport::http1) struct DecoderHandle {
    instance: DecoderInstance,
}

enum DecoderInstance {
    Identity {
        decoder: &'static IdentityDecoder,
        body: BoundedBody,
    },
    Stateful(Box<dyn Decoder>),
}

impl DecoderHandle {
    pub(super) fn identity(decoder: &'static IdentityDecoder, body: BoundedBody) -> Self {
        Self {
            instance: DecoderInstance::Identity { decoder, body },
        }
    }

    pub(super) fn stateful(decoder: Box<dyn Decoder>) -> Self {
        Self {
            instance: DecoderInstance::Stateful(decoder),
        }
    }

    pub(in crate::llm_pipeline::transport::http1) fn is_identity(&self) -> bool {
        matches!(self.instance, DecoderInstance::Identity { .. })
    }

    pub(in crate::llm_pipeline::transport::http1) fn append(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), Http1DecodeFailure> {
        match &mut self.instance {
            DecoderInstance::Identity { decoder, body } => decoder.append(bytes, body),
            DecoderInstance::Stateful(decoder) => decoder.append(bytes),
        }
        .map_err(Self::failure)
    }

    pub(in crate::llm_pipeline::transport::http1) fn finish(
        &mut self,
    ) -> Result<(), Http1DecodeFailure> {
        match &mut self.instance {
            DecoderInstance::Identity { .. } => Ok(()),
            DecoderInstance::Stateful(decoder) => decoder.finish(),
        }
        .map_err(Self::failure)
    }

    pub(in crate::llm_pipeline::transport::http1) fn snapshot(&self) -> Arc<Vec<u8>> {
        match &self.instance {
            DecoderInstance::Identity { body, .. } => body.snapshot(),
            DecoderInstance::Stateful(decoder) => decoder.snapshot(),
        }
    }

    pub(in crate::llm_pipeline::transport::http1) fn clear_identity(&mut self) {
        if let DecoderInstance::Identity { body, .. } = &mut self.instance {
            body.clear();
        }
    }

    pub(super) fn failure(error: io::Error) -> Http1DecodeFailure {
        if error.kind() == io::ErrorKind::OutOfMemory {
            Http1DecodeFailure::BufferCapacity
        } else {
            Http1DecodeFailure::InvalidContentEncoding
        }
    }
}
