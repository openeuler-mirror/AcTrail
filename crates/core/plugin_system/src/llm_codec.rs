#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmCodecRequest<'a> {
    pub method: Option<&'a str>,
    pub authority: Option<&'a str>,
    pub path: Option<&'a str>,
    pub body: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmCodecSseEvent<'a> {
    pub index: usize,
    pub event_type: Option<&'a str>,
    pub id: Option<&'a str>,
    pub data: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlmCodecOutcome {
    NoMatch,
    Decoded(LlmCodecDecoded),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmCodecDecoded {
    pub classifier_id: Option<String>,
    pub protocol_id: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub body: Vec<u8>,
}

pub trait LlmCodecPlugin: Send + Sync {
    fn instance_id(&self) -> &str;
    fn plugin_id(&self) -> &str;
    fn decode_request(&self, request: LlmCodecRequest<'_>) -> Result<LlmCodecOutcome, String>;
    fn decode_sse_event(&self, event: LlmCodecSseEvent<'_>) -> Result<LlmCodecOutcome, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmCodecPluginStatus {
    pub instance_id: String,
    pub plugin_id: String,
}
