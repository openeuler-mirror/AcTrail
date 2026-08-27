//! Provider-neutral HTTP messages emitted by transport normalization.

use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpRequestParts {
    pub(crate) protocol: &'static str,
    pub(crate) scheme: &'static str,
    pub(crate) method: Option<String>,
    pub(crate) authority: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) stream_id: Option<u32>,
    pub(crate) headers_text: Option<String>,
    pub(crate) headers_hpack_base64: Option<String>,
    pub(crate) body: Arc<Vec<u8>>,
    pub(crate) encoded_len: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpResponseParts {
    pub(crate) protocol: &'static str,
    pub(crate) scheme: &'static str,
    pub(crate) status_code: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) stream_id: Option<u32>,
    pub(crate) headers_text: Option<String>,
    pub(crate) headers_hpack_base64: Option<String>,
    pub(crate) body: Arc<Vec<u8>>,
    pub(crate) encoded_len: usize,
    pub(crate) complete: bool,
    pub(crate) body_boundary_known: bool,
}
