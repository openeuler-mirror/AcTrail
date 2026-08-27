//! Semantic action contracts kept separate from raw fact events.

pub mod attr_keys;
pub mod evidence_roles;
pub mod link_roles;
pub mod llm;
mod merge;
pub mod model;
pub mod model_identity;
pub mod store;

pub use llm::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmSseEvent, LlmSseResponseInput,
    LlmTokenUsage, LlmToolCall, LlmToolFunction,
};
pub use merge::SemanticActionMergeError;
pub use model::{
    FileChangeKind, FileObservationPath, FilePathSetIdentity, FilePathSetPath, FilePathSetPathPage,
    FilePathSetState, FilePathSetWrite, LlmRequestBlock, LlmRequestBlockRef, LlmRequestContentPage,
    LlmRequestContentWrite, LlmRequestLineage, LlmRequestLineageWrite, LlmRequestManifest,
    LlmTrajectoryStartReason, LlmTrajectoryTransition, McpJsonRpcContentPage,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionLink, SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionPage,
    SemanticActionStatus, SemanticEvidence, SemanticEvidenceKind,
    file_path_set_identity_for_overflow_scope, file_path_set_identity_for_paths,
};
pub use model_identity::validated_model_identifier;
pub use store::{SemanticActionReadStore, SemanticActionStoreError, SemanticActionWriteStore};
