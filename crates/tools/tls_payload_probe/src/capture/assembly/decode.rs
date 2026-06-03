//! HTTP body content decoding.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use flate2::read::{GzDecoder, ZlibDecoder};

use crate::capture::CaptureConfig;
use crate::{ToolError, ToolResult};

use super::model::{HttpBody, HttpHeader};

const HEADER_CONTENT_ENCODING: &str = "content-encoding";
const ENCODING_IDENTITY: &str = "identity";
const ENCODING_GZIP: &str = "gzip";
const ENCODING_DEFLATE: &str = "deflate";
const ENCODING_ZSTD: &str = "zstd";
const ENCODING_BR: &str = "br";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HttpDecodeConfig {
    pub(crate) max_input_bytes: usize,
    pub(crate) max_output_bytes: usize,
    pub(crate) reader_buffer_bytes: usize,
}

impl From<&CaptureConfig> for HttpDecodeConfig {
    fn from(config: &CaptureConfig) -> Self {
        Self {
            max_input_bytes: config.decode_input_bytes,
            max_output_bytes: config.decode_output_bytes,
            reader_buffer_bytes: config.decode_reader_buffer_bytes,
        }
    }
}

pub(super) fn body_content(
    body: Vec<u8>,
    fields: &BTreeMap<String, String>,
    config: &HttpDecodeConfig,
) -> ToolResult<HttpBody> {
    if body.is_empty() {
        return Ok(HttpBody::Empty);
    }
    let Some(encoding) = content_encoding(fields) else {
        return Ok(text_or_binary(body));
    };
    if encoding == ENCODING_IDENTITY {
        return Ok(text_or_binary(body));
    }
    if body.len() > config.max_input_bytes {
        return Ok(HttpBody::DecodeSkipped {
            encoding,
            compressed_bytes: body.len(),
            limit_bytes: config.max_input_bytes,
        });
    }
    let compressed_bytes = body.len();
    match decode_body(&body, &encoding, config) {
        Ok(decoded) => match String::from_utf8(decoded) {
            Ok(text) => Ok(HttpBody::DecodedText {
                encoding,
                compressed_bytes,
                decoded_bytes: text.len(),
                text,
            }),
            Err(error) => Ok(HttpBody::DecodedBinary {
                encoding,
                compressed_bytes,
                decoded_bytes: error.as_bytes().len(),
            }),
        },
        Err(error) => Ok(HttpBody::DecodeFailed {
            encoding,
            compressed_bytes,
            error: error.to_string(),
        }),
    }
}

pub(crate) fn decoded_text_from_headers(
    body: &[u8],
    headers: &[HttpHeader],
    config: &HttpDecodeConfig,
) -> ToolResult<Option<String>> {
    let fields = header_fields(headers);
    match body_content(body.to_vec(), &fields, config)? {
        HttpBody::Text { text, .. } | HttpBody::DecodedText { text, .. } => Ok(Some(text)),
        HttpBody::Empty
        | HttpBody::Binary { .. }
        | HttpBody::DecodedBinary { .. }
        | HttpBody::DecodeSkipped { .. }
        | HttpBody::DecodeFailed { .. }
        | HttpBody::Partial { .. }
        | HttpBody::PartialText { .. }
        | HttpBody::PartialDecodedText { .. }
        | HttpBody::Streamed { .. } => Ok(None),
    }
}

fn text_or_binary(body: Vec<u8>) -> HttpBody {
    match String::from_utf8(body) {
        Ok(text) => HttpBody::Text {
            bytes: text.len(),
            text,
        },
        Err(error) => HttpBody::Binary {
            bytes: error.as_bytes().len(),
        },
    }
}

fn decode_body(bytes: &[u8], encoding: &str, config: &HttpDecodeConfig) -> ToolResult<Vec<u8>> {
    let mut decoded = bytes.to_vec();
    let encodings = encoding
        .split(',')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for current in encodings.iter().rev() {
        decoded = decode_single(&decoded, current, config)?;
    }
    Ok(decoded)
}

fn decode_single(bytes: &[u8], encoding: &str, config: &HttpDecodeConfig) -> ToolResult<Vec<u8>> {
    match encoding {
        ENCODING_IDENTITY => Ok(bytes.to_vec()),
        ENCODING_GZIP => read_limited(GzDecoder::new(Cursor::new(bytes)), config),
        ENCODING_DEFLATE => read_limited(ZlibDecoder::new(Cursor::new(bytes)), config),
        ENCODING_ZSTD => {
            let decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))
                .map_err(|error| ToolError::new(format!("zstd decoder init failed: {error}")))?;
            read_limited(decoder, config)
        }
        ENCODING_BR => read_limited(
            brotli::Decompressor::new(Cursor::new(bytes), config.reader_buffer_bytes),
            config,
        ),
        value => Err(ToolError::new(format!(
            "unsupported HTTP content-encoding: {value}"
        ))),
    }
}

fn read_limited(mut reader: impl Read, config: &HttpDecodeConfig) -> ToolResult<Vec<u8>> {
    let read_limit = u64::try_from(config.max_output_bytes)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| ToolError::new("decode output limit overflow"))?;
    let mut output = Vec::new();
    reader
        .by_ref()
        .take(read_limit)
        .read_to_end(&mut output)
        .map_err(|error| ToolError::new(format!("decode failed: {error}")))?;
    if output.len() > config.max_output_bytes {
        return Err(ToolError::new(format!(
            "decoded body exceeded {} bytes",
            config.max_output_bytes
        )));
    }
    Ok(output)
}

fn header_fields(headers: &[HttpHeader]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|header| (header.name.to_ascii_lowercase(), header.value.clone()))
        .collect()
}

fn content_encoding(fields: &BTreeMap<String, String>) -> Option<String> {
    fields
        .get(HEADER_CONTENT_ENCODING)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}
