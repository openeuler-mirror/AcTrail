use config_core::daemon::{L0McpCallRetention, McpJsonRpcContentRetention};
use model_core::ids::TraceId;
use semantic_action::McpJsonRpcContentWrite;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MCP_JSONRPC_CANONICAL_FORMAT_VERSION: u32 = 1;

pub(super) struct McpJsonRpcContentProjector {
    retention: L0McpCallRetention,
}

impl McpJsonRpcContentProjector {
    pub(super) fn new(retention: L0McpCallRetention) -> Self {
        Self { retention }
    }

    pub(super) fn request(
        &self,
        trace_id: TraceId,
        action_id: &str,
        value: &Value,
    ) -> Option<McpJsonRpcContentWrite> {
        self.project(self.retention.request_content, trace_id, action_id, value)
    }

    pub(super) fn response(
        &self,
        trace_id: TraceId,
        action_id: &str,
        value: &Value,
    ) -> Option<McpJsonRpcContentWrite> {
        self.project(self.retention.response_content, trace_id, action_id, value)
    }

    fn project(
        &self,
        retention: McpJsonRpcContentRetention,
        trace_id: TraceId,
        action_id: &str,
        value: &Value,
    ) -> Option<McpJsonRpcContentWrite> {
        if matches!(retention, McpJsonRpcContentRetention::None) {
            return None;
        }
        Some(CanonicalMcpJsonRpc::from_value(value).into_write(trace_id, action_id))
    }
}

struct CanonicalMcpJsonRpc {
    bytes: Vec<u8>,
    hash: String,
}

impl CanonicalMcpJsonRpc {
    fn from_value(value: &Value) -> Self {
        assert!(
            value.get("jsonrpc").and_then(Value::as_str) == Some("2.0") && value.is_object(),
            "canonical MCP content requires a JSON-RPC 2.0 object"
        );
        let mut bytes = Vec::new();
        Self::encode_value(value, &mut bytes);
        let digest = Sha256::digest(&bytes);
        Self {
            bytes,
            hash: format!("sha256:{digest:x}"),
        }
    }

    fn encode_value(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null => output.extend_from_slice(b"null"),
            Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
            Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
            Value::String(value) => output.extend_from_slice(
                serde_json::to_string(value)
                    .expect("serde_json strings are always serializable")
                    .as_bytes(),
            ),
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    Self::encode_value(value, output);
                }
                output.push(b']');
            }
            Value::Object(values) => {
                output.push(b'{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend_from_slice(
                        serde_json::to_string(key)
                            .expect("serde_json object keys are always serializable")
                            .as_bytes(),
                    );
                    output.push(b':');
                    Self::encode_value(value, output);
                }
                output.push(b'}');
            }
        }
    }

    fn into_write(self, trace_id: TraceId, action_id: &str) -> McpJsonRpcContentWrite {
        McpJsonRpcContentWrite {
            trace_id,
            action_ids: vec![action_id.to_string()],
            format_version: MCP_JSONRPC_CANONICAL_FORMAT_VERSION,
            canonical_json_hash: self.hash,
            canonical_json: self.bytes,
        }
    }
}
