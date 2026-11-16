use std::io;
use std::sync::Arc;

use super::ContentDecodeFailure;
use super::bounded_body::BoundedBody;
use super::decoder::Decoder;
use super::implementations::IdentityDecoder;

pub(in crate::llm_pipeline) struct DecoderHandle {
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

    pub(in crate::llm_pipeline) fn is_identity(&self) -> bool {
        matches!(self.instance, DecoderInstance::Identity { .. })
    }

    pub(in crate::llm_pipeline) fn append(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), ContentDecodeFailure> {
        match &mut self.instance {
            DecoderInstance::Identity { decoder, body } => decoder.append(bytes, body),
            DecoderInstance::Stateful(decoder) => decoder.append(bytes),
        }
        .map_err(Self::failure)
    }

    pub(in crate::llm_pipeline) fn finish(&mut self) -> Result<(), ContentDecodeFailure> {
        match &mut self.instance {
            DecoderInstance::Identity { .. } => Ok(()),
            DecoderInstance::Stateful(decoder) => decoder.finish(),
        }
        .map_err(Self::failure)
    }

    pub(in crate::llm_pipeline) fn snapshot(&self) -> Arc<Vec<u8>> {
        match &self.instance {
            DecoderInstance::Identity { body, .. } => body.snapshot(),
            DecoderInstance::Stateful(decoder) => decoder.snapshot(),
        }
    }

    pub(in crate::llm_pipeline) fn clear_identity(&mut self) {
        if let DecoderInstance::Identity { body, .. } = &mut self.instance {
            body.clear();
        }
    }

    pub(super) fn failure(error: io::Error) -> ContentDecodeFailure {
        if error.kind() == io::ErrorKind::OutOfMemory {
            ContentDecodeFailure::BufferCapacity
        } else {
            ContentDecodeFailure::InvalidContentEncoding
        }
    }
}
