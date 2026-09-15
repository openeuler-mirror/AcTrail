//! Compact codes for open-ended semantic evidence roles.

use rusqlite::Error as SqlError;

pub(in crate::semantic_actions) struct EncodedEvidenceRole<'a> {
    pub(in crate::semantic_actions) code: i64,
    pub(in crate::semantic_actions) inline: Option<&'a str>,
}

pub(in crate::semantic_actions) fn encode(role: &str) -> EncodedEvidenceRole<'_> {
    let code = match role {
        "file.modify" => 1,
        "file.write" => 2,
        "file.open" => 3,
        "file.close" => 4,
        "file.read" => 5,
        "process.fork_attempt" => 6,
        "command.exec" => 7,
        "process.fork" => 8,
        "http.message" => 9,
        "process.exec.intent" => 10,
        "process.exec.completed" => 11,
        "process.exit" => 12,
        "agent.identity" => 13,
        "agent.exit" => 14,
        "fs.enumerate" => 15,
        "llm.request.payload" => 16,
        "llm.response.payload" => 17,
        "mcp.request.payload" => 18,
        "mcp.response.payload" => 19,
        "mcp.stdin.payload" => 20,
        "mcp.stdout.payload" => 21,
        "mcp.tool_call.payload" => 22,
        "enforcement.decision" => 23,
        _ => 0,
    };
    EncodedEvidenceRole {
        code,
        inline: (code == 0).then_some(role),
    }
}

pub(in crate::semantic_actions) fn decode(
    code: i64,
    inline: Option<String>,
) -> Result<String, SqlError> {
    let role = match code {
        0 => return inline.ok_or(SqlError::InvalidQuery),
        1 => "file.modify",
        2 => "file.write",
        3 => "file.open",
        4 => "file.close",
        5 => "file.read",
        6 => "process.fork_attempt",
        7 => "command.exec",
        8 => "process.fork",
        9 => "http.message",
        10 => "process.exec.intent",
        11 => "process.exec.completed",
        12 => "process.exit",
        13 => "agent.identity",
        14 => "agent.exit",
        15 => "fs.enumerate",
        16 => "llm.request.payload",
        17 => "llm.response.payload",
        18 => "mcp.request.payload",
        19 => "mcp.response.payload",
        20 => "mcp.stdin.payload",
        21 => "mcp.stdout.payload",
        22 => "mcp.tool_call.payload",
        23 => "enforcement.decision",
        _ => return Err(SqlError::InvalidQuery),
    };
    if inline.is_some() {
        return Err(SqlError::InvalidQuery);
    }
    Ok(role.to_string())
}
