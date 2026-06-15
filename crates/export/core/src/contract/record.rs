use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink};

pub struct SemanticActionExportRecord<'a> {
    pub trace: &'a TraceRecord,
    pub action: &'a SemanticAction,
    pub links: &'a [SemanticActionLink],
}
