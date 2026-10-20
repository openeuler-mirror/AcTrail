mod decoder;
mod framing;

pub(in crate::llm_pipeline) use decoder::{Http2Decoder, Http2Event};
pub(crate) use framing::{HTTP2_CONNECTION_PREFACE, data_payload};
pub(in crate::llm_pipeline) use framing::{Http2FrameDecode, decode_http2_frame_state};
