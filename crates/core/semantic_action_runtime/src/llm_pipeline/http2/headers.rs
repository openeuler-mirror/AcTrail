//! One HPACK context per connection direction; metadata is emitted at END_HEADERS.
use crate::{Http1MessageHead, Http2FrameObservation};
use config_core::daemon::Http2ProtocolConfig;

#[derive(Clone, Copy, Debug)]
pub(super) enum HeaderFailure {
    Compression,
    BlockCapacity,
    ListCapacity,
    InvalidFields,
}
impl HeaderFailure {
    pub(super) fn connection_failed(self) -> bool {
        matches!(self, Self::Compression | Self::BlockCapacity)
    }
}
pub(super) struct HeaderBlock {
    pub(super) stream_id: u32,
    pub(super) end_stream: bool,
    pub(super) head: Http1MessageHead,
    pub(super) push_promise: bool,
    bytes: Vec<u8>,
}
pub(super) struct HeaderDecoder {
    hpack: loona_hpack::Decoder<'static>,
    pending: Option<HeaderBlock>,
    max_block: usize,
    max_list: usize,
}
impl HeaderDecoder {
    pub(super) fn new(config: Http2ProtocolConfig) -> Self {
        let mut hpack = loona_hpack::Decoder::new();
        hpack.set_max_table_size(4096);
        hpack.set_max_allowed_table_size(config.max_hpack_table_bytes as usize);
        Self {
            hpack,
            pending: None,
            max_block: config.max_header_block_bytes as usize,
            max_list: config.max_header_list_bytes as usize,
        }
    }
    pub(super) fn advance(
        &mut self,
        frame: &Http2FrameObservation,
    ) -> Result<Option<HeaderBlock>, HeaderFailure> {
        if let Some(mut pending) = self.pending.take() {
            if frame.frame_type != 9 || frame.stream_id != pending.stream_id {
                return Err(HeaderFailure::Compression);
            }
            if pending
                .bytes
                .len()
                .checked_add(frame.payload.len())
                .is_none_or(|size| size > self.max_block)
            {
                return Err(HeaderFailure::BlockCapacity);
            }
            pending.bytes.extend_from_slice(&frame.payload);
            if frame.flags & 4 == 0 {
                self.pending = Some(pending);
                return Ok(None);
            }
            self.decode(&mut pending.head, &pending.bytes, pending.push_promise)?;
            pending.bytes.clear();
            return Ok(Some(pending));
        }
        if frame.frame_type == 9 {
            return Err(HeaderFailure::Compression);
        }
        if !matches!(frame.frame_type, 1 | 5) {
            return Ok(None);
        }
        let fragment = Self::fragment(frame).ok_or(HeaderFailure::Compression)?;
        if fragment.len() > self.max_block {
            return Err(HeaderFailure::BlockCapacity);
        }
        let mut block = HeaderBlock {
            stream_id: frame.stream_id,
            end_stream: frame.frame_type == 1 && frame.flags & 1 != 0,
            push_promise: frame.frame_type == 5,
            head: Http1MessageHead {
                message_id: String::new(),
                source: frame.source.clone(),
                protocol: "h2",
                method: None,
                authority: None,
                path: None,
                status_code: None,
                reason: None,
                headers_text: String::new(),
                declared_body_len: None,
            },
            bytes: Vec::new(),
        };
        if frame.flags & 4 == 0 {
            block.bytes.extend_from_slice(fragment);
            self.pending = Some(block);
            return Ok(None);
        }
        self.decode(&mut block.head, fragment, block.push_promise)?;
        Ok(Some(block))
    }
    fn fragment(frame: &Http2FrameObservation) -> Option<&[u8]> {
        let payload = frame.payload.as_slice();
        let (start, padding) = if frame.flags & 8 != 0 {
            (1usize, usize::from(*payload.first()?))
        } else {
            (0, 0)
        };
        let start = start
            + if frame.frame_type == 1 && frame.flags & 0x20 != 0 {
                5
            } else {
                0
            };
        let start = if frame.frame_type == 5 {
            let promised =
                u32::from_be_bytes(payload.get(start..start + 4)?.try_into().ok()?) & 0x7fff_ffff;
            if promised == 0 {
                return None;
            }
            start + 4
        } else {
            start
        };
        payload.get(start..payload.len().checked_sub(padding)?)
    }
    fn decode(
        &mut self,
        head: &mut Http1MessageHead,
        bytes: &[u8],
        discard_fields: bool,
    ) -> Result<(), HeaderFailure> {
        let mut size = 0usize;
        let mut failure = None;
        self.hpack
            .decode_with_cb(bytes, |name, value| {
                if discard_fields {
                    return;
                }
                size = size
                    .saturating_add(name.len())
                    .saturating_add(value.len())
                    .saturating_add(32);
                if size > self.max_list {
                    failure = Some(HeaderFailure::ListCapacity);
                }
                if failure.is_some() {
                    return;
                }
                let Some(name) = Self::field_name(&name) else {
                    failure = Some(HeaderFailure::InvalidFields);
                    return;
                };
                if !Self::valid_value(&value) {
                    failure = Some(HeaderFailure::InvalidFields);
                    return;
                }
                if name.starts_with(':') && std::str::from_utf8(&value).is_err() {
                    failure = Some(HeaderFailure::InvalidFields);
                    return;
                }
                // HTTP obs-text is opaque; only its display representation is UTF-8.
                // The raw HPACK list budget also bounds this conversion's growth.
                let value = String::from_utf8_lossy(&value);
                let field = match name {
                    ":method" => Some(&mut head.method),
                    ":authority" => Some(&mut head.authority),
                    ":path" => Some(&mut head.path),
                    ":status" => Some(&mut head.status_code),
                    _ => None,
                };
                if let Some(field) = field {
                    if field.replace(value.to_string()).is_some() {
                        failure = Some(HeaderFailure::InvalidFields);
                        return;
                    }
                }
                if name == "content-length" {
                    let Ok(length) = value.parse::<usize>() else {
                        failure = Some(HeaderFailure::InvalidFields);
                        return;
                    };
                    if head
                        .declared_body_len
                        .is_some_and(|previous| previous != length)
                    {
                        failure = Some(HeaderFailure::InvalidFields);
                        return;
                    }
                    head.declared_body_len = Some(length);
                }
                head.headers_text.push_str(name);
                head.headers_text.push_str(": ");
                head.headers_text.push_str(&value);
                head.headers_text.push_str("\r\n");
            })
            .map_err(|_| HeaderFailure::Compression)?;
        failure.map_or(Ok(()), Err)
    }

    fn field_name(name: &[u8]) -> Option<&str> {
        let name = std::str::from_utf8(name).ok()?;
        let token = name.strip_prefix(':').unwrap_or(name);
        (!token.is_empty()
            && token.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
            }))
        .then_some(name)
    }

    fn valid_value(value: &[u8]) -> bool {
        !matches!(value.first(), Some(b' ' | b'\t'))
            && !matches!(value.last(), Some(b' ' | b'\t'))
            && value
                .iter()
                .copied()
                .all(|byte| matches!(byte, b'\t' | b' '..=b'~' | 0x80..=0xff))
    }
}
