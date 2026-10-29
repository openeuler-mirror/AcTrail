//! Semantic action contracts kept separate from raw fact events.

pub mod attr_keys;
mod content_node;
pub mod evidence_roles;
pub mod link_roles;
pub mod llm;
pub mod model;
pub mod model_identity;
pub mod store;
mod update;
pub use content_node::{
    ContentNode, ContentNodeChild, ContentNodeKind, ContentNodePage, ContentNodeQuery,
};

pub use update::{
    SemanticActionChange, SemanticActionFinalizationReason, SemanticActionIdentity,
    SemanticActionUpdate, SemanticAttributeSupplement, SemanticCommandKind,
    SemanticToolResultBinding,
};

pub use llm::{
    LlmJsonResponseInput, LlmParsedResponse, LlmParsedSseEvent, LlmProviderMatch,
    LlmProviderResponseParser, LlmProviderResponseStreamParser, LlmResponseRetention,
    LlmResponseTermination, LlmSseEvent, LlmSseResponseInput, LlmTokenUsage, LlmToolCall,
    LlmToolFunction,
};
pub use model::{
    FileChangeKind, FileObservationPath, FilePathSetIdentity, FilePathSetPath, FilePathSetPathPage,
    FilePathSetState, FilePathSetWrite, LlmRequestBlock, LlmRequestBlockRef, LlmRequestContentPage,
    LlmRequestContentWrite, LlmRequestLineage, LlmRequestLineageWrite, LlmRequestManifest,
    LlmTrajectoryStartReason, LlmTrajectoryTransition, McpJsonRpcContentPage,
    McpJsonRpcContentWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionLink, SemanticActionLinkOrigin, SemanticActionLinkRole, SemanticActionPage,
    SemanticActionStatus, SemanticEvidence, SemanticEvidenceKind,
    file_path_set_identity_for_overflow_scope, file_path_set_identity_for_paths,
};
pub use model_identity::validated_model_identifier;
pub use store::{
    SemanticActionReadStore, SemanticActionStoreError, SemanticActionWriteStore,
    SemanticAttributeSupplementRejection,
};
