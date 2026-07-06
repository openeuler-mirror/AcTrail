//! Semantic evidence role names grouped by source namespace.

use crate::model::SemanticActionKind;

pub mod command {
    pub const EXEC: &str = "command.exec";
}

pub mod file {
    use super::SemanticActionKind;

    pub const CLOSE: &str = "file.close";
    pub const OPEN: &str = "file.open";
    pub const READ: &str = SemanticActionKind::FileRead.as_str();
    pub const WRITE: &str = SemanticActionKind::FileWrite.as_str();
}

pub mod fs {
    use super::SemanticActionKind;

    pub const ENUMERATE: &str = SemanticActionKind::FsEnumerate.as_str();
}

pub mod llm_request {
    pub const PAYLOAD: &str = "llm.request.payload";
}

pub mod llm_response {
    pub const PAYLOAD: &str = "llm.response.payload";
}

pub mod mcp {
    pub const CLIENT_RECEIVE_PAYLOAD: &str = "mcp.client_receive.payload";
    pub const CLIENT_SEND_PAYLOAD: &str = "mcp.client_send.payload";
    pub const REQUEST_PAYLOAD: &str = "mcp.request.payload";
    pub const RESPONSE_PAYLOAD: &str = "mcp.response.payload";
    pub const STDIN_PAYLOAD: &str = "mcp.stdin.payload";
    pub const STDOUT_PAYLOAD: &str = "mcp.stdout.payload";
    pub const TOOL_CALL_PAYLOAD: &str = "mcp.tool_call.payload";
}

pub mod process {
    use super::SemanticActionKind;

    pub const EXEC: &str = SemanticActionKind::ProcessExec.as_str();
    pub const EXIT: &str = "process.exit";
    pub const FORK: &str = "process.fork";
    pub const FORK_ATTEMPT: &str = SemanticActionKind::ProcessForkAttempt.as_str();
}
