// Tests for the corresponding production module.
use super::*;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

#[test]
fn tool_interaction_codes_are_unique_and_round_trip() {
    let codebook = current();
    codebook.validate().expect("current codebook is unique");
    for kind in [
        SemanticActionKind::LlmToolCall,
        SemanticActionKind::LlmToolResult,
    ] {
        let code = codebook.action_kind.code(kind);
        assert_eq!(codebook.action_kind.decode(i64::from(code)), Ok(kind));
    }
    for role in [
        SemanticActionLinkRole::LlmRequestTrajectoryParent,
        SemanticActionLinkRole::LlmRequestTrajectoryFork,
        SemanticActionLinkRole::LlmResponseToolCall,
        SemanticActionLinkRole::LlmToolCallResult,
        SemanticActionLinkRole::LlmToolCallAgentInvocation,
    ] {
        let code = codebook.link_role.code(role);
        assert_eq!(codebook.link_role.decode(i64::from(code)), Ok(role));
    }
}
