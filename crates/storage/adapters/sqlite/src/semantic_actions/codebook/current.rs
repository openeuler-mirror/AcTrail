//! Current semantic action storage: compact integer codes in SQLite.

use super::{
    ActionCompletenessCodes, ActionKindCodes, ActionStatusCodes, EvidenceKindCodes,
    LinkConfidenceCodes, LinkRoleCodes, SemanticActionCodebook,
};

pub(in crate::semantic_actions::codebook) const SCHEMA_VERSION: i32 = 12;

pub(in crate::semantic_actions::codebook) const CODEBOOK: &SemanticActionCodebook =
    &SemanticActionCodebook {
        action_kind: ActionKindCodes {
            process_exec: 101,
            file_modify: 102,
            file_read: 103,
            file_write: 104,
            file_tty_io: 105,
            file_bulk_read: 106,
            fs_enumerate: 107,
            http_message: 108,
            llm_call: 109,
            llm_request: 110,
            llm_response: 111,
            sse_stream: 112,
            sse_event: 113,
            enforcement_decision: 114,
            process_fork_attempt: 115,
            agent_invocation: 116,
            command_invocation: 117,
        },
        action_status: ActionStatusCodes {
            in_progress: 201,
            success: 202,
            error: 203,
            unknown: 204,
        },
        action_completeness: ActionCompletenessCodes {
            complete: 301,
            partial: 302,
            inferred: 303,
        },
        evidence_kind: EvidenceKindCodes {
            event: 401,
            payload_aggregate: 402,
            payload_segment: 403,
        },
        link_role: LinkRoleCodes {
            agent_performed_action: 501,
            command_contains_file_access: 502,
            command_contains_process_fork_attempt: 503,
            command_contains_process_exec: 504,
            command_contains_command_invocation: 505,
            command_contains_llm_call: 506,
            file_write_contains_file_event: 507,
            agent_invocation_exec: 508,
            agent_invocation_child_llm_request: 509,
            llm_call_request: 510,
            llm_call_response: 511,
            llm_request_http_message: 512,
            llm_request_llm_response: 513,
            llm_response_http_message: 514,
            llm_response_sse_stream: 515,
            sse_stream_event: 516,
        },
        link_confidence: LinkConfidenceCodes {
            observed: 601,
            derived: 602,
        },
    };
