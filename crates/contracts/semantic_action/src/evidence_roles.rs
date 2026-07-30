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

pub mod process {
    use super::SemanticActionKind;

    pub const EXEC_COMPLETED: &str = "process.exec.completed";
    pub const EXEC_INTENT: &str = "process.exec.intent";
    pub const EXIT: &str = SemanticActionKind::ProcessExit.as_str();
    pub const FORK: &str = "process.fork";
    pub const FORK_ATTEMPT: &str = SemanticActionKind::ProcessForkAttempt.as_str();
}

pub mod agent {
    use super::SemanticActionKind;

    pub const IDENTITY: &str = SemanticActionKind::AgentIdentity.as_str();
    pub const EXIT: &str = SemanticActionKind::AgentExit.as_str();
}
