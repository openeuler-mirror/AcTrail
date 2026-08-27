//! Large-field split/join for event payloads.
//!
//! Only `ApplicationPayload.body` and `StdioPayload.data` are large; every
//! other payload family carries small, ABI-bounded fields and stays in the
//! serialized small-fields blob.

use model_core::event::{ApplicationBody, EventPayload};

use super::{BlockKind, PayloadBlock};

pub fn split_large_fields(payload: &mut EventPayload) -> Vec<PayloadBlock> {
    match payload {
        EventPayload::Stdio(payload) => {
            if payload.data.is_empty() {
                Vec::new()
            } else {
                vec![PayloadBlock {
                    kind: BlockKind::StdioData,
                    bytes: std::mem::take(&mut payload.data),
                }]
            }
        }
        EventPayload::Application(payload) => {
            let Some(body) = payload.body.take() else {
                return Vec::new();
            };
            let (kind, bytes) = match body {
                ApplicationBody::Text(text) => (BlockKind::HttpBodyText, text.into_bytes()),
                ApplicationBody::Json(text) => (BlockKind::HttpBodyJson, text.into_bytes()),
                ApplicationBody::Base64(text) => (BlockKind::HttpBodyBase64, text.into_bytes()),
            };
            vec![PayloadBlock { kind, bytes }]
        }
        _ => Vec::new(),
    }
}

pub fn join_large_fields(payload: &mut EventPayload, blocks: &[PayloadBlock]) {
    for block in blocks {
        match block.kind {
            BlockKind::StdioData => {
                if let EventPayload::Stdio(payload) = payload {
                    payload.data = block.bytes.clone();
                }
            }
            BlockKind::HttpBodyText => {
                set_application_body(payload, ApplicationBody::Text(text(&block.bytes)));
            }
            BlockKind::HttpBodyJson => {
                set_application_body(payload, ApplicationBody::Json(text(&block.bytes)));
            }
            BlockKind::HttpBodyBase64 => {
                set_application_body(payload, ApplicationBody::Base64(text(&block.bytes)));
            }
        }
    }
}

fn set_application_body(payload: &mut EventPayload, body: ApplicationBody) {
    if let EventPayload::Application(payload) = payload {
        payload.body = Some(body);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
