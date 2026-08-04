mod canonical;
mod read;
mod write;

pub(in crate::semantic_actions) use read::mcp_jsonrpc_content_page;
pub(in crate::semantic_actions) use write::upsert_mcp_jsonrpc_contents;
