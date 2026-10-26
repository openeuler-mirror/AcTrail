//! HTTP/1 wire framing budgets, independent of content decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Http1ProtocolConfig {
    pub max_buffer_bytes: u64,
    pub max_evidence_ranges: u32,
}
impl Default for Http1ProtocolConfig {
    fn default() -> Self {
        Self {
            max_buffer_bytes: 8 * 1024 * 1024,
            max_evidence_ranges: 8192,
        }
    }
}
